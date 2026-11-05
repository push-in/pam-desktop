//! `http`: the application's network requests.
//!
//! One-shot requests (`request`, `upload`, and the legacy `get`) run on a
//! small pool of threads and answer once, with the whole response as text.
//! Streamed transfers (`transferStart` / `transferNext` / `transferCancel`)
//! upload a multipart form or a raw file from the sandbox on their own
//! threads and report progress through a [`WatchChannel`].
//!
//! Requests go to `https://` only, unless the module was built for
//! development or the host is the loopback. Redirects are never followed,
//! request headers are validated against injection, and every size is checked
//! before the corresponding bytes are read into memory.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::channel::WatchChannel;
use super::fetch::{HttpBody, HttpClient, HttpRequest, HttpResponse};
use super::files::{mime_for_path, resolve};
use super::support::{CopyError, WorkerPool, copy_bounded, uuid};
use super::{Module, Reply, request_map};
use crate::wiremap::{MAX_WIRE_MAP_BYTES, WireMap};

const MAX_REQUEST_BODY_BYTES: usize = 1_048_576;
const MAX_RESPONSE_BODY_BYTES: u64 = 900 * 1024;
const MAX_HEADERS: usize = 32;
const MAX_HEADER_VALUE_BYTES: usize = 8192;
const MAX_RESPONSE_HEADERS: usize = 64;
const MAX_RESPONSE_HEADER_BYTES: usize = 32 * 1024;
const MAX_UPLOAD_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PARTS: usize = 64;
const MAX_FIELD_BYTES: usize = 1_048_576;
const MAX_MULTIPART_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// One-shot requests in flight at once; the rest wait for a free thread.
const REQUEST_THREADS: usize = 4;
/// Streamed transfers in flight at once; the rest wait for a free thread.
const TRANSFER_THREADS: usize = 8;

const TRANSFER_MULTIPART: i64 = 1;
const TRANSFER_PROGRESS: i64 = 1;
const TRANSFER_COMPLETE: i64 = 2;
const TRANSFER_FAILED: i64 = 3;

const DEFAULT_ACCEPT: &str = "application/json, text/plain, */*";
const RESPONSE_LIMIT: &str = "HTTP response exceeds one MiB";
const REQUEST_FAILED: &str = "HTTP request failed";
const TRANSFER_CANCELLED: &str = "HTTP transfer cancelled";
const OUTSIDE_SANDBOX: &str = "File is outside the private files directory or missing";
const FILE_CHANGED: &str = "Multipart file changed during upload";

// ---- URLs -----------------------------------------------------------------

/// The parts of an absolute URL that request validation looks at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    /// Lower-case.
    pub scheme: String,
    /// Lower-case; an IPv6 address has no brackets.
    pub host: String,
    pub port: Option<u16>,
    /// Whether the authority carries `user[:password]@`.
    pub credentials: bool,
    ipv6: bool,
}

/// Parses `scheme://[userinfo@]host[:port][/…]`.
///
/// The parser is deliberately strict so that it can never disagree with the
/// HTTP stack about where a request goes: whitespace, control characters,
/// non-ASCII characters and backslashes are refused anywhere in the URL, and
/// the host must be a plain name, an IPv4 address or a bracketed IPv6
/// address.
#[must_use]
pub fn parse_url(url: &str) -> Option<Url> {
    if url
        .bytes()
        .any(|byte| byte <= b' ' || byte >= 0x7f || byte == b'\\')
    {
        return None;
    }
    let (scheme, rest) = url.split_once("://")?;
    let valid_scheme = scheme
        .bytes()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && scheme
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'));
    if !valid_scheme {
        return None;
    }
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let (credentials, host_port) = match authority.rsplit_once('@') {
        Some((_, host_port)) => (true, host_port),
        None => (false, authority),
    };
    let (host, port, ipv6) = if let Some(bracketed) = host_port.strip_prefix('[') {
        let (address, after) = bracketed.split_once(']')?;
        address.parse::<Ipv6Addr>().ok()?;
        let port = if after.is_empty() {
            None
        } else {
            Some(after.strip_prefix(':')?)
        };
        (address, port, true)
    } else {
        match host_port.rsplit_once(':') {
            Some((host, port)) => (host, Some(port), false),
            None => (host_port, None, false),
        }
    };
    let valid_host = ipv6
        || (!host.is_empty()
            && host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_')));
    if !valid_host {
        return None;
    }
    let port = match port {
        Some(digits) => {
            if digits.is_empty()
                || digits.len() > 5
                || !digits.bytes().all(|byte| byte.is_ascii_digit())
            {
                return None;
            }
            Some(digits.parse::<u16>().ok()?)
        }
        None => None,
    };
    Some(Url {
        scheme: scheme.to_ascii_lowercase(),
        host: host.to_ascii_lowercase(),
        port,
        credentials,
        ipv6,
    })
}

impl Url {
    /// Whether the host is this machine: `localhost`, `127.0.0.0/8` or `::1`.
    #[must_use]
    pub fn is_loopback(&self) -> bool {
        if self.ipv6 {
            return self
                .host
                .parse::<Ipv6Addr>()
                .is_ok_and(|address| address.is_loopback());
        }
        self.host == "localhost"
            || self.host.ends_with(".localhost")
            || self
                .host
                .parse::<Ipv4Addr>()
                .is_ok_and(|address| address.is_loopback())
    }

    /// The origin as trace contexts spell it: `scheme://host[:port]`, the
    /// port omitted when absent or 443 and an IPv6 host in brackets.
    #[must_use]
    pub fn origin(&self) -> String {
        let port = match self.port {
            Some(port) if port != 443 => format!(":{port}"),
            _ => String::new(),
        };
        if self.ipv6 {
            format!("{}://[{}]{port}", self.scheme, self.host)
        } else {
            format!("{}://{}{port}", self.scheme, self.host)
        }
    }
}

/// Validates the target of a request: HTTPS, or plain HTTP in development
/// or to the loopback; never with credentials in the URL.
fn request_url(url: &str, allow_insecure: bool) -> Result<Url, String> {
    let parsed = parse_url(url).ok_or("Invalid HTTP URL")?;
    match parsed.scheme.as_str() {
        "https" => {}
        "http" if allow_insecure || parsed.is_loopback() => {}
        _ => return Err("HTTP requests require HTTPS".into()),
    }
    if parsed.credentials {
        return Err("Invalid HTTP URL".into());
    }
    Ok(parsed)
}

// ---- request headers ------------------------------------------------------

/// How the request body is produced, which decides the headers a caller may set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transport {
    /// No body, or a text body.
    Text,
    /// A file streamed with a fixed length.
    File,
    /// A multipart form, which owns `Content-Type`.
    Multipart,
}

/// Headers that frame the message on the connection; the HTTP stack writes them.
fn frames_message(name: &str) -> bool {
    matches!(
        name,
        "content-length" | "transfer-encoding" | "connection" | "trailer" | "upgrade"
    )
}

/// Decodes and validates the `headers` JSON object of a request.
///
/// Names are `[A-Za-z0-9-]{1,64}`, values single-line and at most 8192
/// bytes. Names that differ only by case collapse to the last one. For a
/// text request, headers that frame the message are dropped (the HTTP stack
/// computes them); for a file or multipart body they are refused.
fn request_headers(json: &str, transport: Transport) -> Result<Vec<(String, String)>, String> {
    const INVALID: &str = "Invalid HTTP header";
    let headers = match serde_json::from_str::<serde_json::Value>(json) {
        Ok(serde_json::Value::Object(headers)) => headers,
        // PHP encodes an empty list of headers as an empty array.
        Ok(serde_json::Value::Array(items)) if items.is_empty() => return Ok(Vec::new()),
        _ => return Err(INVALID.into()),
    };
    if headers.len() > MAX_HEADERS {
        return Err("HTTP requests support at most 32 headers".into());
    }
    let mut decoded: Vec<(String, String)> = Vec::with_capacity(headers.len());
    for (name, value) in headers {
        let Some(value) = value.as_str() else {
            return Err(INVALID.into());
        };
        let valid_name = (1..=64).contains(&name.len())
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
        if !valid_name
            || value
                .chars()
                .any(|character| character.is_control() && character != '\t')
        {
            return Err(INVALID.into());
        }
        if value.len() > MAX_HEADER_VALUE_BYTES {
            return Err("HTTP header value is too large".into());
        }
        let lower = name.to_ascii_lowercase();
        if matches!(lower.as_str(), "traceparent" | "tracestate") {
            return Err("Trace headers require an origin-scoped context".into());
        }
        if frames_message(&lower) || (transport != Transport::Text && lower == "host") {
            if transport == Transport::Text {
                continue;
            }
            return Err("File upload headers cannot override HTTP transport fields".into());
        }
        if transport == Transport::Multipart && lower == "content-type" {
            return Err("Multipart requests own the Content-Type boundary".into());
        }
        decoded.retain(|(existing, _)| !existing.eq_ignore_ascii_case(&name));
        decoded.push((name, value.to_owned()));
    }
    Ok(decoded)
}

fn has_header(headers: &[(String, String)], name: &str) -> bool {
    headers
        .iter()
        .any(|(header, _)| header.eq_ignore_ascii_case(name))
}

fn valid_traceparent(traceparent: &str) -> bool {
    let hex = |text: &str, length: usize| {
        text.len() == length
            && text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    let nonzero = |text: &str| text.bytes().any(|byte| byte != b'0');
    let mut parts = traceparent.split('-');
    let (Some(version), Some(trace), Some(span), Some(flags), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return false;
    };
    version == "00"
        && hex(trace, 32)
        && nonzero(trace)
        && hex(span, 16)
        && nonzero(span)
        && hex(flags, 2)
}

/// The `traceparent` header of a request, when the application attached a
/// trace context whose origin is the request's.
fn trace_header(url: &Url, request: &WireMap) -> Result<Option<(String, String)>, String> {
    let (traceparent, origin) = match (request.text("traceparent"), request.text("traceOrigin")) {
        (None, None) => return Ok(None),
        (Some(traceparent), Some(origin)) => (traceparent, origin),
        _ => return Err("Incomplete HTTP trace context".into()),
    };
    if !valid_traceparent(traceparent) {
        return Err("Invalid W3C version 00 traceparent".into());
    }
    // The SDK wraps an already bracketed IPv6 host in a second pair.
    let sdk_ipv6 = url.ipv6.then(|| {
        url.origin()
            .replacen("://[", "://[[", 1)
            .replacen(']', "]]", 1)
    });
    let same_origin = origin == url.origin() || sdk_ipv6.as_deref() == Some(origin);
    if !(same_origin && origin.starts_with("https://")) {
        return Err("Trace context origin does not match the HTTP request origin".into());
    }
    Ok(Some((String::from("traceparent"), traceparent.to_owned())))
}

fn timeout(request: &WireMap, default: i64, maximum: i64) -> Duration {
    let milliseconds = request
        .integer("timeoutMs")
        .unwrap_or(default)
        .clamp(1_000, maximum);
    Duration::from_millis(u64::try_from(milliseconds).unwrap_or(1_000))
}

// ---- responses ------------------------------------------------------------

/// Response headers as a JSON object: names lower-cased, repeated headers
/// joined with `", "`, at most 64 entries and 32 KiB (the rest is left out).
fn response_headers(headers: &[(String, String)]) -> String {
    let mut joined: Vec<(String, String)> = Vec::new();
    for (name, value) in headers {
        let name = name.to_ascii_lowercase();
        match joined.iter_mut().find(|(existing, _)| *existing == name) {
            Some((_, values)) => {
                values.push_str(", ");
                values.push_str(value);
            }
            None => joined.push((name, value.clone())),
        }
    }
    let mut object = serde_json::Map::new();
    let mut bytes = 0_usize;
    for (name, value) in joined {
        bytes += name.len() + value.len();
        if bytes <= MAX_RESPONSE_HEADER_BYTES && object.len() < MAX_RESPONSE_HEADERS {
            object.insert(name, serde_json::Value::String(value));
        }
    }
    serde_json::Value::Object(object).to_string()
}

/// Reads a response into the `{statusCode, body, headers}` result.
///
/// The body is text: at most 900 KiB, decoded as UTF-8 with invalid bytes
/// replaced. A longer body fails without being read to the end.
fn response_map(response: HttpResponse, cancel: &AtomicBool) -> Result<WireMap, String> {
    let HttpResponse {
        status,
        headers,
        content_length,
        mut body,
    } = response;
    if content_length.is_some_and(|length| length > MAX_RESPONSE_BODY_BYTES) {
        return Err(RESPONSE_LIMIT.into());
    }
    let mut bytes = Vec::new();
    copy_bounded(
        &mut body,
        &mut bytes,
        MAX_RESPONSE_BODY_BYTES,
        cancel,
        |_| {},
    )
    .map_err(|error| match error {
        CopyError::TooLarge => String::from(RESPONSE_LIMIT),
        CopyError::Cancelled => String::from("HTTP request cancelled"),
        CopyError::Io(message) if message.is_empty() => String::from(REQUEST_FAILED),
        CopyError::Io(message) => message,
    })?;
    let result = WireMap::new()
        .with("statusCode", i64::from(status))
        .with("body", String::from_utf8_lossy(&bytes).into_owned())
        .with("headers", response_headers(&headers));
    // Replacing invalid bytes can grow the text past what the bridge carries.
    if result.encode().len() > MAX_WIRE_MAP_BYTES - 64 {
        return Err(RESPONSE_LIMIT.into());
    }
    Ok(result)
}

// ---- upload sources -------------------------------------------------------

/// A private copy of a sandbox file, taken so that the bytes sent match the
/// declared length whatever happens to the original meanwhile. The copy is
/// deleted when the snapshot is dropped.
struct Snapshot {
    path: PathBuf,
    length: u64,
}

impl Snapshot {
    fn create(
        files_root: &Path,
        cache: &Path,
        path: &str,
        cancel: &AtomicBool,
    ) -> Result<Self, String> {
        let source = resolve(files_root, path)
            .ok()
            .filter(|source| source.is_file())
            .ok_or(OUTSIDE_SANDBOX)?;
        let mut input = File::open(&source).map_err(|_| String::from(OUTSIDE_SANDBOX))?;
        let expected = input
            .metadata()
            .map_err(|_| String::from(OUTSIDE_SANDBOX))?
            .len();
        if expected > MAX_UPLOAD_BYTES {
            return Err("Upload file exceeds 64 MiB".into());
        }
        let unavailable = |_| String::from("Private upload cache is unavailable");
        fs::create_dir_all(cache).map_err(unavailable)?;
        let target = cache.join(format!("pam-upload-{}.tmp", uuid()));
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(unavailable)?;
        let snapshot = Self {
            path: target,
            length: expected,
        };
        let copied =
            copy_bounded(&mut input, &mut output, expected, cancel, |_| {}).map_err(|error| {
                match error {
                    CopyError::Cancelled => String::from("HTTP upload cancelled"),
                    CopyError::TooLarge => String::from("Upload source changed during copy"),
                    CopyError::Io(message) => message,
                }
            })?;
        if copied != expected {
            return Err("Upload source changed during copy".into());
        }
        Ok(snapshot)
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

// ---- multipart bodies -----------------------------------------------------

/// One part of a multipart form, as the request describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Part {
    Field {
        name: String,
        value: String,
    },
    File {
        name: String,
        /// Sandbox-relative.
        path: String,
        /// Blank: inferred from the file name.
        mime_type: String,
        /// Blank: the file's own name.
        filename: String,
    },
}

fn single_line(text: &str) -> bool {
    !text.contains(['\r', '\n', '\0'])
}

fn valid_part_mime(mime: &str) -> bool {
    let token = |text: &str| {
        (1..=127).contains(&text.len())
            && text.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(
                        byte,
                        b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-'
                    )
            })
    };
    mime.split_once('/')
        .is_some_and(|(kind, subtype)| token(kind) && token(subtype))
}

/// Decodes the `parts` JSON array of a multipart transfer.
fn parse_parts(json: &str) -> Result<Vec<Part>, String> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(json)
    else {
        return Err("Multipart parts are required".into());
    };
    if items.is_empty() {
        return Err("Multipart requests require at least one part".into());
    }
    if items.len() > MAX_PARTS {
        return Err("Multipart requests support at most 64 parts".into());
    }
    let text = |item: &serde_json::Value, key: &str| {
        item.get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let mut parts = Vec::with_capacity(items.len());
    for item in &items {
        let name = text(item, "name")
            .filter(|name| (1..=256).contains(&name.len()) && single_line(name))
            .ok_or("Invalid multipart field name")?;
        match item.get("type").and_then(serde_json::Value::as_i64) {
            Some(1) => {
                let value = text(item, "value").ok_or("Invalid multipart field value")?;
                if value.len() > MAX_FIELD_BYTES {
                    return Err("Multipart field exceeds 1 MiB".into());
                }
                parts.push(Part::Field { name, value });
            }
            Some(2) => {
                let path = text(item, "path").ok_or(OUTSIDE_SANDBOX)?;
                let filename = text(item, "filename").unwrap_or_default();
                let mime_type = text(item, "mimeType").unwrap_or_default();
                if !filename.trim().is_empty()
                    && (filename.len() > 255
                        || !single_line(&filename)
                        || filename.contains(['/', '\\']))
                {
                    return Err("Invalid multipart filename".into());
                }
                if !mime_type.trim().is_empty() && !valid_part_mime(&mime_type) {
                    return Err("Invalid multipart MIME type".into());
                }
                parts.push(Part::File {
                    name,
                    path,
                    mime_type,
                    filename,
                });
            }
            _ => return Err("Unknown multipart part type".into()),
        }
    }
    Ok(parts)
}

enum Segment {
    Bytes(Vec<u8>),
    File { path: PathBuf, length: u64 },
}

/// A `multipart/form-data` body of a known length whose files are streamed
/// from the sandbox, never held in memory.
struct MultipartBody {
    boundary: String,
    segments: Vec<Segment>,
    length: u64,
}

impl MultipartBody {
    /// Resolves the file parts inside `files_root` and lays the body out.
    fn build(parts: Vec<Part>, files_root: &Path, boundary: String) -> Result<Self, String> {
        // Quotes cannot appear inside the quoted parameters of a part header.
        let quoted = |value: &str| value.replace('"', "%22");
        let mut segments = Vec::with_capacity(parts.len() * 3 + 1);
        let mut length = 0_u64;
        let mut push = |segment: Segment| {
            length += match &segment {
                Segment::Bytes(bytes) => bytes.len() as u64,
                Segment::File { length, .. } => *length,
            };
            segments.push(segment);
        };
        for part in parts {
            match part {
                Part::Field { name, value } => {
                    let head = format!(
                        "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n",
                        quoted(&name)
                    );
                    push(Segment::Bytes(head.into_bytes()));
                    push(Segment::Bytes(value.into_bytes()));
                }
                Part::File {
                    name,
                    path,
                    mime_type,
                    filename,
                } => {
                    let file = resolve(files_root, &path)
                        .ok()
                        .filter(|file| file.is_file())
                        .ok_or(OUTSIDE_SANDBOX)?;
                    let size = fs::metadata(&file)
                        .map_err(|_| String::from("Multipart file does not exist"))?
                        .len();
                    let filename = if filename.trim().is_empty() {
                        file.file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    } else {
                        filename
                    };
                    if filename.is_empty() || filename.len() > 255 || !single_line(&filename) {
                        return Err("Invalid multipart filename".into());
                    }
                    let mime_type = if mime_type.trim().is_empty() {
                        mime_for_path(&file).to_owned()
                    } else {
                        mime_type
                    };
                    let head = format!(
                        "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {mime_type}\r\n\r\n",
                        quoted(&name),
                        quoted(&filename)
                    );
                    push(Segment::Bytes(head.into_bytes()));
                    push(Segment::File {
                        path: file,
                        length: size,
                    });
                }
            }
            push(Segment::Bytes(b"\r\n".to_vec()));
        }
        push(Segment::Bytes(format!("--{boundary}--\r\n").into_bytes()));
        if length > MAX_MULTIPART_BYTES {
            return Err("Multipart body exceeds 2 GiB".into());
        }
        Ok(Self {
            boundary,
            segments,
            length,
        })
    }

    fn content_type(&self) -> String {
        format!("multipart/form-data; boundary={}", self.boundary)
    }

    /// The body as a reader. A file that is missing or whose size changed
    /// since [`build`](Self::build) fails the read and leaves the reason in
    /// `failure`.
    fn into_reader(self, failure: Arc<Mutex<Option<String>>>) -> MultipartReader {
        MultipartReader {
            segments: self.segments.into_iter(),
            current: Current::None,
            failure,
        }
    }
}

enum Current {
    None,
    Bytes(std::io::Cursor<Vec<u8>>),
    File { file: File, remaining: u64 },
}

struct MultipartReader {
    segments: std::vec::IntoIter<Segment>,
    current: Current,
    failure: Arc<Mutex<Option<String>>>,
}

impl MultipartReader {
    fn fail(&self, message: &str) -> std::io::Error {
        if let Ok(mut failure) = self.failure.lock() {
            failure.get_or_insert_with(|| message.to_owned());
        }
        std::io::Error::other(message.to_owned())
    }
}

impl Read for MultipartReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        loop {
            match &mut self.current {
                Current::None => match self.segments.next() {
                    Some(Segment::Bytes(bytes)) => {
                        self.current = Current::Bytes(std::io::Cursor::new(bytes))
                    }
                    Some(Segment::File { path, length }) => {
                        let file = File::open(path)
                            .map_err(|_| self.fail("Multipart file does not exist"))?;
                        self.current = Current::File {
                            file,
                            remaining: length,
                        };
                    }
                    None => return Ok(0),
                },
                Current::Bytes(cursor) => match cursor.read(buffer)? {
                    0 => self.current = Current::None,
                    count => return Ok(count),
                },
                Current::File { file, remaining } => {
                    if *remaining == 0 {
                        // The declared length was sent: the file must end here.
                        let grown = file.read(&mut [0_u8; 1])? > 0;
                        self.current = Current::None;
                        if grown {
                            return Err(self.fail(FILE_CHANGED));
                        }
                        continue;
                    }
                    let wanted = usize::try_from(*remaining)
                        .unwrap_or(usize::MAX)
                        .min(buffer.len());
                    match file.read(&mut buffer[..wanted])? {
                        0 => return Err(self.fail(FILE_CHANGED)),
                        count => {
                            *remaining -= count as u64;
                            return Ok(count);
                        }
                    }
                }
            }
        }
    }
}

// ---- transfers ------------------------------------------------------------

/// Limits progress events to steps of one percent or 256 KiB, whichever is
/// larger, plus the first chunk and the completed body.
struct ProgressThrottle {
    total: u64,
    step: u64,
    reported: Option<u64>,
}

impl ProgressThrottle {
    fn new(total: u64) -> Self {
        Self {
            total,
            step: (total / 100).max(256 * 1024),
            reported: None,
        }
    }

    fn should_report(&mut self, sent: u64) -> bool {
        let due = match self.reported {
            None => true,
            Some(reported) => {
                sent != reported
                    && (sent >= self.total || sent.saturating_sub(reported) >= self.step)
            }
        };
        if due {
            self.reported = Some(sent);
        }
        due
    }
}

fn progress_event(sent: u64, total: u64) -> WireMap {
    WireMap::new()
        .with("state", TRANSFER_PROGRESS)
        .with("bytesSent", i64::try_from(sent).unwrap_or(i64::MAX))
        .with("totalBytes", i64::try_from(total).unwrap_or(i64::MAX))
}

fn failed_event(message: &str) -> WireMap {
    WireMap::new()
        .with("state", TRANSFER_FAILED)
        .with("message", message)
}

/// What a transfer uploads.
enum Source {
    Multipart(Vec<Part>),
    /// A sandbox-relative file sent as the raw body.
    File(String),
}

/// A validated `transferStart` request.
struct TransferRequest {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    timeout: Duration,
    source: Source,
}

/// A running transfer: its event channel and its cancellation flag.
struct Transfer {
    channel: Arc<WatchChannel<WireMap>>,
    cancel: Arc<AtomicBool>,
}

impl Transfer {
    fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.channel.close();
    }
}

/// What the worker threads need from the module.
struct Shared {
    files_root: PathBuf,
    /// Where upload snapshots live: private to the application.
    upload_cache: PathBuf,
    client: Arc<dyn HttpClient>,
}

impl Shared {
    /// Uploads `request`, offering progress to `channel`, and returns the
    /// terminal `complete` event.
    fn transfer(
        &self,
        request: TransferRequest,
        channel: &WatchChannel<WireMap>,
        cancel: &AtomicBool,
    ) -> Result<WireMap, String> {
        // Cancelled while it waited for a thread: nothing to prepare.
        if cancel.load(Ordering::Relaxed) {
            return Err(TRANSFER_CANCELLED.into());
        }
        let failure = Arc::new(Mutex::new(None));
        let mut headers = Vec::with_capacity(request.headers.len() + 2);
        if !has_header(&request.headers, "accept") {
            headers.push((String::from("Accept"), String::from(DEFAULT_ACCEPT)));
        }
        headers.extend(request.headers);
        let mut snapshot = None;
        let (body, total) = match request.source {
            Source::Multipart(parts) => {
                let multipart =
                    MultipartBody::build(parts, &self.files_root, format!("pam-{}", uuid()))?;
                headers.push((String::from("Content-Type"), multipart.content_type()));
                let length = multipart.length;
                let reader = Box::new(multipart.into_reader(Arc::clone(&failure)));
                (HttpBody::Stream { reader, length }, length)
            }
            Source::File(path) => {
                let created =
                    Snapshot::create(&self.files_root, &self.upload_cache, &path, cancel)?;
                if !has_header(&headers, "content-type") {
                    headers.push((
                        String::from("Content-Type"),
                        String::from("application/octet-stream"),
                    ));
                }
                let body = HttpBody::File {
                    path: created.path.clone(),
                    length: created.length,
                };
                let length = created.length;
                snapshot = Some(created);
                (body, length)
            }
        };
        let outgoing = HttpRequest {
            method: request.method,
            url: request.url,
            headers,
            body,
            connect_timeout: request.timeout.min(Duration::from_secs(15)),
            read_timeout: request.timeout,
            total_timeout: None,
            follow_redirects: false,
        };
        channel.offer(progress_event(0, total));
        let throttle = Mutex::new(ProgressThrottle::new(total));
        let report = |sent: u64, _total: u64| {
            let due = throttle
                .lock()
                .is_ok_and(|mut throttle| throttle.should_report(sent));
            if due {
                channel.offer(progress_event(sent, total));
            }
        };
        let sent = self.client.send(outgoing, Some(&report), cancel);
        drop(snapshot);
        let response = sent.map_err(|error| {
            let reason = failure.lock().ok().and_then(|mut failure| failure.take());
            match reason {
                Some(reason) => reason,
                None if error.is_empty() => String::from("HTTP transfer failed"),
                None => error,
            }
        })?;
        if cancel.load(Ordering::Relaxed) {
            return Err(TRANSFER_CANCELLED.into());
        }
        // The whole body went out, whatever the client reported on the way.
        let complete = throttle
            .lock()
            .is_ok_and(|throttle| throttle.reported == Some(total));
        if !complete {
            channel.offer(progress_event(total, total));
        }
        Ok(response_map(response, cancel)?.with("state", TRANSFER_COMPLETE))
    }
}

// ---- module ---------------------------------------------------------------

/// A validated one-shot request.
struct OneShot {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
    timeout: Duration,
    /// `upload`: the sandbox-relative file sent as the body.
    upload: Option<String>,
}

pub struct Http {
    shared: Arc<Shared>,
    allow_insecure: bool,
    requests: WorkerPool,
    uploads: WorkerPool,
    transfers: HashMap<i64, Transfer>,
    next_transfer: i64,
    /// Set when the worker that asked is gone: one-shot requests stop.
    stale: Arc<AtomicBool>,
}

impl Http {
    /// `files_root` is the file sandbox (`files::sandbox_root`) uploads read
    /// from, `client` performs the requests, and `allow_insecure` lets the
    /// application reach `http://` hosts other than the loopback
    /// (development only).
    #[must_use]
    pub fn new(files_root: &Path, client: Arc<dyn HttpClient>, allow_insecure: bool) -> Self {
        // Snapshots stay next to the sandbox, inside the application's own
        // data directory, never in a temporary directory other users can read.
        let upload_cache = files_root
            .parent()
            .unwrap_or(files_root)
            .join("pam-upload-cache");
        // Snapshots of a previous run that ended abruptly are of no use.
        let _ = fs::remove_dir_all(&upload_cache);
        Self {
            shared: Arc::new(Shared {
                files_root: files_root.to_path_buf(),
                upload_cache,
                client,
            }),
            allow_insecure,
            requests: WorkerPool::new("pam-http", REQUEST_THREADS),
            uploads: WorkerPool::new("pam-http-transfer", TRANSFER_THREADS),
            transfers: HashMap::new(),
            next_transfer: 0,
            stale: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Validates a `request`, `upload` or `get` call.
    fn one_shot(&self, method: &str, request: &WireMap) -> Result<OneShot, String> {
        let url = request.text("url").ok_or("HTTP URL is required")?;
        let upload = method == "upload";
        let http_method = match method {
            "get" => "GET",
            "upload" => "PUT",
            _ => request.text("method").ok_or("HTTP method is required")?,
        };
        if !matches!(http_method, "GET" | "POST" | "PUT" | "PATCH" | "DELETE") {
            return Err(format!("Unsupported HTTP method {http_method}"));
        }
        let parsed = request_url(url, self.allow_insecure)?;
        let body = request.text("body");
        if upload && body.is_some() {
            return Err("File upload cannot include a text body".into());
        }
        if body.is_some_and(|body| body.len() > MAX_REQUEST_BODY_BYTES) {
            return Err("HTTP request body exceeds one MiB".into());
        }
        let transport = if upload {
            Transport::File
        } else {
            Transport::Text
        };
        let mut headers = request_headers(request.text("headers").unwrap_or("{}"), transport)?;
        if !has_header(&headers, "accept") {
            headers.insert(0, (String::from("Accept"), String::from(DEFAULT_ACCEPT)));
        }
        headers.extend(trace_header(&parsed, request)?);
        let path = if upload {
            Some(
                request
                    .text("path")
                    .ok_or("Upload source path is required")?
                    .to_owned(),
            )
        } else {
            None
        };
        Ok(OneShot {
            method: http_method.to_owned(),
            url: url.to_owned(),
            headers,
            body: body.map(str::to_owned),
            timeout: timeout(request, 30_000, 120_000),
            upload: path,
        })
    }

    /// Validates a `transferStart` call.
    fn transfer_request(&self, request: &WireMap) -> Result<TransferRequest, String> {
        let url = request.text("url").ok_or("HTTP URL is required")?;
        let method = request.text("method").unwrap_or("POST");
        if !matches!(method, "POST" | "PUT" | "PATCH") {
            return Err(format!("Unsupported HTTP transfer method {method}"));
        }
        let parsed = request_url(url, self.allow_insecure)?;
        let multipart = request.integer("kind").unwrap_or(TRANSFER_MULTIPART) == TRANSFER_MULTIPART;
        let transport = if multipart {
            Transport::Multipart
        } else {
            Transport::File
        };
        let mut headers = request_headers(request.text("headers").unwrap_or("{}"), transport)?;
        headers.extend(trace_header(&parsed, request)?);
        let source = if multipart {
            Source::Multipart(parse_parts(
                request
                    .text("parts")
                    .ok_or("Multipart parts are required")?,
            )?)
        } else {
            Source::File(
                request
                    .text("path")
                    .ok_or("Upload source path is required")?
                    .to_owned(),
            )
        };
        Ok(TransferRequest {
            method: method.to_owned(),
            url: url.to_owned(),
            headers,
            timeout: timeout(request, 60_000, 600_000),
            source,
        })
    }

    fn start_transfer(&mut self, request: &WireMap, reply: Reply) {
        let request = match self.transfer_request(request) {
            Ok(request) => request,
            Err(message) => return reply.failure(message),
        };
        self.next_transfer += 1;
        let id = self.next_transfer;
        let transfer = Transfer {
            channel: Arc::new(WatchChannel::new()),
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let channel = Arc::clone(&transfer.channel);
        let cancel = Arc::clone(&transfer.cancel);
        self.transfers.insert(id, transfer);
        let shared = Arc::clone(&self.shared);
        self.uploads.run(move || {
            let event = match shared.transfer(request, &channel, &cancel) {
                Ok(event) => event,
                Err(_) if cancel.load(Ordering::Relaxed) => failed_event(TRANSFER_CANCELLED),
                Err(message) => failed_event(&message),
            };
            channel.finish(event);
        });
        reply.map(&WireMap::new().with("transfer", id));
    }
}

/// Performs a one-shot request and shapes its response.
fn perform(shared: &Shared, request: OneShot, cancel: &AtomicBool) -> Result<Vec<u8>, String> {
    let started = Instant::now();
    let uploading = request.upload.is_some();
    let snapshot = match &request.upload {
        Some(path) => Some(Snapshot::create(
            &shared.files_root,
            &shared.upload_cache,
            path,
            cancel,
        )?),
        None => None,
    };
    let body = match (&snapshot, request.body) {
        (Some(snapshot), _) => HttpBody::File {
            path: snapshot.path.clone(),
            length: snapshot.length,
        },
        (None, Some(text)) => HttpBody::Bytes(text.into_bytes()),
        (None, None) => HttpBody::None,
    };
    let outgoing = HttpRequest {
        method: request.method,
        url: request.url,
        headers: request.headers,
        body,
        connect_timeout: request.timeout.min(Duration::from_secs(10)),
        read_timeout: request.timeout,
        // An upload has that long as a whole, not per read.
        total_timeout: uploading.then_some(request.timeout),
        follow_redirects: false,
    };
    let timed_out = || uploading && started.elapsed() >= request.timeout;
    let failed = |error: String| {
        if timed_out() {
            String::from("HTTP upload timed out")
        } else if error.is_empty() {
            String::from(REQUEST_FAILED)
        } else {
            error
        }
    };
    let response = shared.client.send(outgoing, None, cancel).map_err(failed)?;
    drop(snapshot);
    Ok(response_map(response, cancel).map_err(failed)?.encode())
}

impl Module for Http {
    fn call(&mut self, method: &str, payload: &[u8], reply: Reply) {
        let request = match request_map(payload) {
            Ok(request) => request,
            Err(error) => return reply.failure(error),
        };
        match method {
            "request" | "upload" | "get" => {
                let request = match self.one_shot(method, &request) {
                    Ok(request) => request,
                    Err(message) => return reply.failure(message),
                };
                let shared = Arc::clone(&self.shared);
                let stale = Arc::clone(&self.stale);
                self.requests
                    .run(move || reply.result(perform(&shared, request, &stale)));
            }
            "transferStart" => self.start_transfer(&request, reply),
            "transferNext" => {
                let Some(id) = request.integer("transfer") else {
                    return reply.failure("HTTP transfer id is required");
                };
                match self.transfers.get(&id) {
                    Some(transfer) => transfer.channel.next_reply(reply),
                    None => reply.failure("HTTP transfer not found"),
                }
            }
            "transferCancel" => {
                if let Some(transfer) = request
                    .integer("transfer")
                    .and_then(|id| self.transfers.remove(&id))
                {
                    transfer.cancel();
                }
                reply.done();
            }
            _ => reply.failure("Unknown HTTP method"),
        }
    }

    fn reset(&mut self) {
        for (_, transfer) in self.transfers.drain() {
            transfer.cancel();
        }
        self.stale.store(true, Ordering::Relaxed);
        self.stale = Arc::new(AtomicBool::new(false));
    }
}

impl Drop for Http {
    fn drop(&mut self) {
        self.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::ModuleResult;
    use crate::modules::files::sandbox_root;
    use crate::modules::test_support::{PlainClient, Served, TestServer, temporary_directory};
    use crate::runtime::{MODULE_FAILURE, MODULE_SUCCESS};
    use std::sync::mpsc::Receiver;

    const WAIT: Duration = Duration::from_secs(10);

    struct Fixture {
        directory: PathBuf,
        http: Http,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            Self::with(name, false)
        }

        fn with(name: &str, allow_insecure: bool) -> Self {
            let directory = temporary_directory(name);
            let root = sandbox_root(&directory.join("data"));
            fs::create_dir_all(&root).unwrap();
            let http = Http::new(&root, Arc::new(PlainClient), allow_insecure);
            Self { directory, http }
        }

        fn root(&self) -> PathBuf {
            sandbox_root(&self.directory.join("data"))
        }

        fn start(&mut self, method: &str, request: &WireMap) -> Receiver<ModuleResult> {
            let (reply, answer) = Reply::channel();
            self.http.call(method, &request.encode(), reply);
            answer
        }

        fn call(&mut self, method: &str, request: &WireMap) -> Result<WireMap, String> {
            let (status, payload) = self.start(method, request).recv_timeout(WAIT).unwrap();
            if status == MODULE_SUCCESS {
                Ok(request_map(&payload).unwrap())
            } else {
                assert_eq!(status, MODULE_FAILURE);
                Err(String::from_utf8(payload).unwrap())
            }
        }

        fn fails(&mut self, method: &str, request: &WireMap) -> String {
            self.call(method, request).unwrap_err()
        }

        /// Reads transfer events until the terminal one; returns the progress
        /// events and the terminal event.
        fn drain(&mut self, transfer: i64) -> (Vec<(i64, i64)>, WireMap) {
            let mut progress = Vec::new();
            loop {
                let event = self
                    .call("transferNext", &WireMap::new().with("transfer", transfer))
                    .unwrap();
                if event.integer("state") == Some(TRANSFER_PROGRESS) {
                    progress.push((
                        event.integer("bytesSent").unwrap(),
                        event.integer("totalBytes").unwrap(),
                    ));
                } else {
                    return (progress, event);
                }
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    fn get(url: &str) -> WireMap {
        WireMap::new()
            .with("url", url)
            .with("method", "GET")
            .with("headers", "{}")
    }

    fn pairs(headers: &[(&str, &str)]) -> Vec<(String, String)> {
        headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn urls_are_parsed_strictly() {
        let url = parse_url("HTTPS://API.Example.com:8443/v1/users?page=2#top").unwrap();
        assert_eq!(
            (
                url.scheme.as_str(),
                url.host.as_str(),
                url.port,
                url.credentials
            ),
            ("https", "api.example.com", Some(8443), false)
        );
        assert_eq!(parse_url("https://example.com").unwrap().port, None);
        assert!(
            parse_url("https://user:secret@example.com/")
                .unwrap()
                .credentials
        );
        assert!(parse_url("https://user@example.com/").unwrap().credentials);
        let ipv6 = parse_url("http://[::1]:8080/x").unwrap();
        assert_eq!((ipv6.host.as_str(), ipv6.port), ("::1", Some(8080)));
        for invalid in [
            "",
            "example.com/path",
            "https:/example.com",
            "https://",
            "https:///path",
            "https://exa mple.com/",
            "https://example.com/a b",
            "https://example.com/\r\nHost: evil",
            "https://example.com\\@evil.com/",
            "https://example.com:port/",
            "https://example.com:/",
            "https://example.com:99999/",
            "https://[::1/",
            "https://[not-an-address]/",
            "https://[::1]8080/",
            "https://exämple.com/",
            "1https://example.com/",
            "https://exa%6dple.com/",
        ] {
            assert_eq!(parse_url(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn loopback_hosts_and_origins_are_recognised() {
        for loopback in [
            "http://localhost:3000/",
            "http://127.0.0.1/",
            "http://127.8.9.10:81/",
            "http://[::1]/",
            "http://app.localhost/",
        ] {
            assert!(parse_url(loopback).unwrap().is_loopback(), "{loopback}");
        }
        for remote in [
            "http://example.com/",
            "http://localhost.example.com/",
            "http://128.0.0.1/",
            "http://[::2]/",
            "http://notlocalhost/",
        ] {
            assert!(!parse_url(remote).unwrap().is_loopback(), "{remote}");
        }
        let origin = |url: &str| parse_url(url).unwrap().origin();
        assert_eq!(
            origin("https://API.example.com/v1"),
            "https://api.example.com"
        );
        assert_eq!(
            origin("https://api.example.com:443/v1"),
            "https://api.example.com"
        );
        assert_eq!(
            origin("https://api.example.com:8443/"),
            "https://api.example.com:8443"
        );
        assert_eq!(
            origin("https://[2001:db8::1]:8443/"),
            "https://[2001:db8::1]:8443"
        );
    }

    #[test]
    fn plain_http_needs_development_or_the_loopback() {
        assert!(request_url("https://example.com/", false).is_ok());
        assert!(request_url("http://127.0.0.1:8000/", false).is_ok());
        assert!(request_url("http://example.com/", true).is_ok());
        assert_eq!(
            request_url("http://example.com/", false).unwrap_err(),
            "HTTP requests require HTTPS"
        );
        assert_eq!(
            request_url("ftp://example.com/", true).unwrap_err(),
            "HTTP requests require HTTPS"
        );
        assert_eq!(
            request_url("https://user:pass@example.com/", false).unwrap_err(),
            "Invalid HTTP URL"
        );
        assert_eq!(
            request_url("not a url", true).unwrap_err(),
            "Invalid HTTP URL"
        );
    }

    #[test]
    fn request_headers_are_validated() {
        assert_eq!(request_headers("{}", Transport::Text).unwrap(), []);
        assert_eq!(request_headers("[]", Transport::Text).unwrap(), []);
        assert_eq!(
            request_headers(
                r#"{"Authorization":"Bearer t","X-Id":"7"}"#,
                Transport::Text
            )
            .unwrap(),
            pairs(&[("Authorization", "Bearer t"), ("X-Id", "7")])
        );
        for invalid in [
            r#"{"X-A":"a\r\nInjected: 1"}"#,
            r#"{"X-A":"a\nb"}"#,
            r#"{"X-A":"a\u0000b"}"#,
            r#"{"X A":"v"}"#,
            r#"{"X_A":"v"}"#,
            r#"{"X:A":"v"}"#,
            r#"{"":"v"}"#,
            r#"{"X-A":1}"#,
            r#"{"X-A":null}"#,
            r#"["X-A"]"#,
            "nonsense",
        ] {
            assert_eq!(
                request_headers(invalid, Transport::Text).unwrap_err(),
                "Invalid HTTP header",
                "{invalid}"
            );
        }
        let long_name = format!(r#"{{"{}":"v"}}"#, "n".repeat(65));
        assert_eq!(
            request_headers(&long_name, Transport::Text).unwrap_err(),
            "Invalid HTTP header"
        );
        let large = format!(r#"{{"X-A":"{}"}}"#, "v".repeat(8193));
        assert_eq!(
            request_headers(&large, Transport::Text).unwrap_err(),
            "HTTP header value is too large"
        );
        assert!(
            request_headers(
                &format!(r#"{{"X-A":"{}"}}"#, "v".repeat(8192)),
                Transport::Text
            )
            .is_ok()
        );
        for trace in ["traceparent", "TraceState"] {
            assert_eq!(
                request_headers(&format!(r#"{{"{trace}":"x"}}"#), Transport::Text).unwrap_err(),
                "Trace headers require an origin-scoped context"
            );
        }
        let many: serde_json::Map<String, serde_json::Value> = (0..33)
            .map(|index| (format!("X-H{index}"), serde_json::Value::from("v")))
            .collect();
        assert_eq!(
            request_headers(
                &serde_json::Value::Object(many).to_string(),
                Transport::Text
            )
            .unwrap_err(),
            "HTTP requests support at most 32 headers"
        );
    }

    #[test]
    fn transport_headers_belong_to_the_http_stack() {
        // A text request loses the framing headers silently.
        assert_eq!(
            request_headers(
                r#"{"Content-Length":"5","Transfer-Encoding":"chunked","Connection":"close","X-Id":"1"}"#,
                Transport::Text
            )
            .unwrap(),
            pairs(&[("X-Id", "1")])
        );
        for name in [
            "Host",
            "content-length",
            "Transfer-Encoding",
            "CONNECTION",
            "Trailer",
            "upgrade",
        ] {
            for transport in [Transport::File, Transport::Multipart] {
                assert_eq!(
                    request_headers(&format!(r#"{{"{name}":"x"}}"#), transport).unwrap_err(),
                    "File upload headers cannot override HTTP transport fields"
                );
            }
        }
        assert_eq!(
            request_headers(r#"{"content-type":"text/plain"}"#, Transport::Multipart).unwrap_err(),
            "Multipart requests own the Content-Type boundary"
        );
        assert!(request_headers(r#"{"Content-Type":"video/mp4"}"#, Transport::File).is_ok());
        // Names that differ only by case collapse to the last one.
        assert_eq!(
            request_headers(r#"{"Accept":"a/a","accept":"b/b"}"#, Transport::Text).unwrap(),
            pairs(&[("accept", "b/b")])
        );
    }

    #[test]
    fn trace_contexts_are_bound_to_their_origin() {
        const TRACEPARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        let url = parse_url("https://api.example.com/v1").unwrap();
        let context = |traceparent: &str, origin: &str| {
            WireMap::new()
                .with("traceparent", traceparent)
                .with("traceOrigin", origin)
        };
        assert_eq!(trace_header(&url, &WireMap::new()), Ok(None));
        assert_eq!(
            trace_header(&url, &context(TRACEPARENT, "https://api.example.com")),
            Ok(Some((
                String::from("traceparent"),
                String::from(TRACEPARENT)
            )))
        );
        assert_eq!(
            trace_header(&url, &WireMap::new().with("traceparent", TRACEPARENT)).unwrap_err(),
            "Incomplete HTTP trace context"
        );
        assert_eq!(
            trace_header(
                &url,
                &WireMap::new().with("traceOrigin", "https://api.example.com")
            )
            .unwrap_err(),
            "Incomplete HTTP trace context"
        );
        for invalid in [
            "01-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
            "00-4BF92F3577B34DA6A3CE929D0E0E4736-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-1",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-extra",
            "",
        ] {
            assert_eq!(
                trace_header(&url, &context(invalid, "https://api.example.com")).unwrap_err(),
                "Invalid W3C version 00 traceparent",
                "{invalid}"
            );
        }
        for origin in [
            "https://other.example.com",
            "https://api.example.com:8443",
            "http://api.example.com",
            "https://api.example.com/",
        ] {
            assert_eq!(
                trace_header(&url, &context(TRACEPARENT, origin)).unwrap_err(),
                "Trace context origin does not match the HTTP request origin"
            );
        }
        // A trace never goes over plain HTTP, even to the same host.
        let plain = parse_url("http://127.0.0.1:8000/").unwrap();
        assert_eq!(
            trace_header(&plain, &context(TRACEPARENT, "http://127.0.0.1:8000")).unwrap_err(),
            "Trace context origin does not match the HTTP request origin"
        );
        let ipv6 = parse_url("https://[2001:db8::1]:8443/").unwrap();
        assert!(trace_header(&ipv6, &context(TRACEPARENT, "https://[2001:db8::1]:8443")).is_ok());
        assert!(trace_header(&ipv6, &context(TRACEPARENT, "https://[[2001:db8::1]]:8443")).is_ok());
    }

    #[test]
    fn timeouts_are_clamped() {
        let of = |milliseconds: i64| {
            timeout(
                &WireMap::new().with("timeoutMs", milliseconds),
                30_000,
                120_000,
            )
        };
        assert_eq!(
            timeout(&WireMap::new(), 30_000, 120_000),
            Duration::from_secs(30)
        );
        assert_eq!(of(5), Duration::from_secs(1));
        assert_eq!(of(-1), Duration::from_secs(1));
        assert_eq!(of(45_000), Duration::from_secs(45));
        assert_eq!(of(i64::MAX), Duration::from_secs(120));
    }

    fn response(status: u16, headers: &[(&str, &str)], body: &[u8]) -> HttpResponse {
        HttpResponse {
            status,
            headers: pairs(headers),
            content_length: None,
            body: Box::new(std::io::Cursor::new(body.to_vec())),
        }
    }

    #[test]
    fn responses_become_text_with_joined_headers() {
        let cancel = AtomicBool::new(false);
        let map = response_map(
            response(
                404,
                &[
                    ("Content-Type", "text/plain"),
                    ("set-cookie", "a=1"),
                    ("Set-Cookie", "b=2"),
                ],
                b"ol\xc3\xa1 \xff!",
            ),
            &cancel,
        )
        .unwrap();
        assert_eq!(map.integer("statusCode"), Some(404));
        assert_eq!(map.text("body"), Some("olá \u{fffd}!"));
        let headers: serde_json::Value =
            serde_json::from_str(map.text("headers").unwrap()).unwrap();
        assert_eq!(
            headers,
            serde_json::json!({"content-type": "text/plain", "set-cookie": "a=1, b=2"})
        );
        assert_eq!(response_headers(&[]), "{}");
    }

    #[test]
    fn response_headers_are_bounded() {
        let many: Vec<(String, String)> = (0..100)
            .map(|index| (format!("x-h{index}"), String::from("v")))
            .collect();
        let object: serde_json::Value = serde_json::from_str(&response_headers(&many)).unwrap();
        assert_eq!(object.as_object().unwrap().len(), 64);
        let large = pairs(&[("a", "1"), ("big", &"v".repeat(40_000)), ("z", "2")]);
        let object: serde_json::Value = serde_json::from_str(&response_headers(&large)).unwrap();
        assert_eq!(object, serde_json::json!({"a": "1"}));
    }

    #[test]
    fn large_responses_fail_before_they_are_read() {
        let cancel = AtomicBool::new(false);
        let limit = usize::try_from(MAX_RESPONSE_BODY_BYTES).unwrap();
        assert!(response_map(response(200, &[], &vec![b'a'; limit]), &cancel).is_ok());
        assert_eq!(
            response_map(response(200, &[], &vec![b'a'; limit + 1]), &cancel).unwrap_err(),
            RESPONSE_LIMIT
        );
        // A declared length over the limit is refused without reading at all.
        struct Unreadable;
        impl Read for Unreadable {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                panic!("the body must not be read");
            }
        }
        let declared = HttpResponse {
            status: 200,
            headers: Vec::new(),
            content_length: Some(MAX_RESPONSE_BODY_BYTES + 1),
            body: Box::new(Unreadable),
        };
        assert_eq!(response_map(declared, &cancel).unwrap_err(), RESPONSE_LIMIT);
        // Invalid bytes triple in size once replaced: still bounded for the bridge.
        assert_eq!(
            response_map(response(200, &[], &vec![0xff; limit]), &cancel).unwrap_err(),
            RESPONSE_LIMIT
        );
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(
            response_map(response(200, &[], b"x"), &cancel).unwrap_err(),
            "HTTP request cancelled"
        );
    }

    #[test]
    fn multipart_parts_are_validated() {
        let parts = parse_parts(
            r#"[{"type":1,"name":"title","value":"Olá"},
                {"type":2,"name":"photo","path":"imports/a.png","mimeType":"","filename":""}]"#,
        )
        .unwrap();
        assert_eq!(
            parts,
            [
                Part::Field {
                    name: "title".into(),
                    value: "Olá".into()
                },
                Part::File {
                    name: "photo".into(),
                    path: "imports/a.png".into(),
                    mime_type: String::new(),
                    filename: String::new()
                },
            ]
        );
        let error = |json: &str| parse_parts(json).unwrap_err();
        assert_eq!(error("{}"), "Multipart parts are required");
        assert_eq!(error("nope"), "Multipart parts are required");
        assert_eq!(error("[]"), "Multipart requests require at least one part");
        let many = vec![serde_json::json!({"type": 1, "name": "f", "value": ""}); 65];
        assert_eq!(
            error(&serde_json::Value::Array(many).to_string()),
            "Multipart requests support at most 64 parts"
        );
        assert_eq!(
            error(r#"[{"type":1,"name":"","value":"v"}]"#),
            "Invalid multipart field name"
        );
        assert_eq!(
            error(r#"[{"type":1,"name":"a\r\nb","value":"v"}]"#),
            "Invalid multipart field name"
        );
        assert_eq!(
            error(r#"[{"type":1,"value":"v"}]"#),
            "Invalid multipart field name"
        );
        assert_eq!(
            error(r#"[{"type":3,"name":"a"}]"#),
            "Unknown multipart part type"
        );
        assert_eq!(
            error(r#"[{"name":"a","value":"v"}]"#),
            "Unknown multipart part type"
        );
        let large =
            serde_json::json!([{"type": 1, "name": "a", "value": "v".repeat(MAX_FIELD_BYTES + 1)}]);
        assert_eq!(error(&large.to_string()), "Multipart field exceeds 1 MiB");
        assert_eq!(
            error(r#"[{"type":2,"name":"a","path":"a.png","filename":"../a.png"}]"#),
            "Invalid multipart filename"
        );
        assert_eq!(
            error(r#"[{"type":2,"name":"a","path":"a.png","filename":"a\r\n.png"}]"#),
            "Invalid multipart filename"
        );
        assert_eq!(
            error(r#"[{"type":2,"name":"a","path":"a.png","mimeType":"image/png\r\nX: 1"}]"#),
            "Invalid multipart MIME type"
        );
        assert_eq!(
            error(r#"[{"type":2,"name":"a","path":"a.png","mimeType":"image"}]"#),
            "Invalid multipart MIME type"
        );
    }

    fn read_all(body: MultipartBody) -> (Result<Vec<u8>, String>, Option<String>) {
        let failure = Arc::new(Mutex::new(None));
        let mut bytes = Vec::new();
        let outcome = body
            .into_reader(Arc::clone(&failure))
            .read_to_end(&mut bytes)
            .map(|_| bytes)
            .map_err(|error| error.to_string());
        let reason = failure.lock().unwrap().clone();
        (outcome, reason)
    }

    #[test]
    fn multipart_bodies_have_the_exact_wire_form() {
        let fixture = Fixture::new("multipart-bytes");
        let root = fixture.root();
        fs::create_dir_all(root.join("imports")).unwrap();
        fs::write(root.join("imports/photo.png"), b"\x89PNG\r\n\x00\xff").unwrap();
        fs::write(root.join("notes"), b"plain").unwrap();
        let parts = vec![
            Part::Field {
                name: "title".into(),
                value: "Olá \"mundo\"".into(),
            },
            Part::File {
                name: "photo \"1\"".into(),
                path: "imports/photo.png".into(),
                mime_type: String::new(),
                filename: String::new(),
            },
            Part::File {
                name: "attachment".into(),
                path: "notes".into(),
                mime_type: "text/x-custom".into(),
                filename: "my \"notes\".txt".into(),
            },
            Part::Field {
                name: "empty".into(),
                value: String::new(),
            },
        ];
        let body = MultipartBody::build(parts, &root, String::from("pam-test-boundary")).unwrap();
        assert_eq!(
            body.content_type(),
            "multipart/form-data; boundary=pam-test-boundary"
        );
        let mut expected = Vec::new();
        expected.extend_from_slice(
            "--pam-test-boundary\r\n\
             Content-Disposition: form-data; name=\"title\"\r\n\
             \r\n\
             Olá \"mundo\"\r\n\
             --pam-test-boundary\r\n\
             Content-Disposition: form-data; name=\"photo %221%22\"; filename=\"photo.png\"\r\n\
             Content-Type: image/png\r\n\
             \r\n"
                .as_bytes(),
        );
        expected.extend_from_slice(b"\x89PNG\r\n\x00\xff");
        expected.extend_from_slice(
            b"\r\n\
              --pam-test-boundary\r\n\
              Content-Disposition: form-data; name=\"attachment\"; filename=\"my %22notes%22.txt\"\r\n\
              Content-Type: text/x-custom\r\n\
              \r\n\
              plain\r\n\
              --pam-test-boundary\r\n\
              Content-Disposition: form-data; name=\"empty\"\r\n\
              \r\n\
              \r\n\
              --pam-test-boundary--\r\n",
        );
        assert_eq!(body.length, expected.len() as u64);
        let (bytes, reason) = read_all(body);
        assert_eq!(bytes.unwrap(), expected);
        assert_eq!(reason, None);
    }

    #[test]
    fn multipart_files_must_stay_inside_the_sandbox_and_unchanged() {
        let fixture = Fixture::new("multipart-files");
        let root = fixture.root();
        fs::write(fixture.directory.join("outside.txt"), b"secret").unwrap();
        let file = |path: &str| {
            vec![Part::File {
                name: "f".into(),
                path: path.into(),
                mime_type: String::new(),
                filename: String::new(),
            }]
        };
        let build =
            |path: &str| MultipartBody::build(file(path), &root, String::from("pam-b")).map(|_| ());
        assert_eq!(build("missing.bin").unwrap_err(), OUTSIDE_SANDBOX);
        assert_eq!(build("../outside.txt").unwrap_err(), OUTSIDE_SANDBOX);
        assert_eq!(build("/etc/passwd").unwrap_err(), OUTSIDE_SANDBOX);
        fs::create_dir_all(root.join("folder")).unwrap();
        assert_eq!(build("folder").unwrap_err(), OUTSIDE_SANDBOX);

        // The file shrinks, grows or disappears after the body was laid out.
        for (after, message) in [
            (Some(&b"shor"[..]), FILE_CHANGED),
            (Some(&b"longer than before"[..]), FILE_CHANGED),
            (None, "Multipart file does not exist"),
        ] {
            fs::write(root.join("data.bin"), b"original").unwrap();
            let body =
                MultipartBody::build(file("data.bin"), &root, String::from("pam-b")).unwrap();
            match after {
                Some(contents) => fs::write(root.join("data.bin"), contents).unwrap(),
                None => fs::remove_file(root.join("data.bin")).unwrap(),
            }
            let (bytes, reason) = read_all(body);
            assert!(bytes.is_err());
            assert_eq!(reason.as_deref(), Some(message));
        }
    }

    #[test]
    fn progress_is_throttled_to_meaningful_steps() {
        let mut small = ProgressThrottle::new(1000);
        assert!(small.should_report(100));
        assert!(!small.should_report(500));
        assert!(small.should_report(1000));
        assert!(!small.should_report(1000));

        let total = 100 * 1024 * 1024;
        let mut large = ProgressThrottle::new(total);
        let mut reported = Vec::new();
        let mut sent = 0;
        while sent < total {
            sent += 64 * 1024;
            if large.should_report(sent) {
                reported.push(sent);
            }
        }
        // The first chunk, then one step per percent, ending on the total.
        assert_eq!(reported.first(), Some(&(64 * 1024)));
        assert_eq!(reported.last(), Some(&total));
        assert!(reported.len() <= 102);
        assert!(
            reported
                .windows(2)
                .skip(1)
                .all(|pair| pair[1] - pair[0] >= total / 100 || pair[1] == total)
        );
    }

    #[test]
    fn requests_reach_the_server_and_any_status_is_a_success() {
        let server = TestServer::start(|request| match request.path.as_str() {
            "/users?page=2" => Served::ok(r#"{"ok":true}"#)
                .header("Content-Type", "application/json")
                .header("X-Request-Id", "abc"),
            "/missing" => Served::status(404, "not here"),
            "/moved" => Served::status(302, "").header("Location", "/users?page=2"),
            _ => Served::status(500, "boom"),
        });
        let mut fixture = Fixture::new("request");
        let request = WireMap::new()
            .with("url", server.url("/users?page=2"))
            .with("method", "POST")
            .with(
                "headers",
                r#"{"Authorization":"Bearer t","Content-Type":"application/json"}"#,
            )
            .with("body", r#"{"name":"Ana"}"#);
        let response = fixture.call("request", &request).unwrap();
        assert_eq!(response.integer("statusCode"), Some(200));
        assert_eq!(response.text("body"), Some(r#"{"ok":true}"#));
        let headers: serde_json::Value =
            serde_json::from_str(response.text("headers").unwrap()).unwrap();
        assert_eq!(headers["content-type"], "application/json");
        assert_eq!(headers["x-request-id"], "abc");

        let received = &server.requests()[0];
        assert_eq!(received.method, "POST");
        assert_eq!(received.path, "/users?page=2");
        assert_eq!(received.body, br#"{"name":"Ana"}"#);
        assert_eq!(received.header("authorization"), Some("Bearer t"));
        assert_eq!(received.header("content-type"), Some("application/json"));
        assert_eq!(received.header("accept"), Some(DEFAULT_ACCEPT));

        let missing = fixture
            .call("request", &get(&server.url("/missing")))
            .unwrap();
        assert_eq!(missing.integer("statusCode"), Some(404));
        assert_eq!(missing.text("body"), Some("not here"));
        // Redirects come back as they are.
        let moved = fixture
            .call("request", &get(&server.url("/moved")))
            .unwrap();
        assert_eq!(moved.integer("statusCode"), Some(302));
        let headers: serde_json::Value =
            serde_json::from_str(moved.text("headers").unwrap()).unwrap();
        assert_eq!(headers["location"], "/users?page=2");
        assert_eq!(server.requests().len(), 3);

        // The legacy `get` needs no method, and a caller's Accept wins.
        let legacy = WireMap::new()
            .with("url", server.url("/missing"))
            .with("headers", r#"{"accept":"text/html"}"#);
        assert_eq!(
            fixture.call("get", &legacy).unwrap().integer("statusCode"),
            Some(404)
        );
        let received = &server.requests()[3];
        assert_eq!(received.method, "GET");
        assert_eq!(received.header("accept"), Some("text/html"));
    }

    #[test]
    fn invalid_requests_fail_before_any_connection() {
        let server = TestServer::start(|_| Served::ok("never"));
        let mut fixture = Fixture::new("request-validation");
        let url = server.url("/");
        assert_eq!(
            fixture.fails("request", &WireMap::new()),
            "HTTP URL is required"
        );
        assert_eq!(
            fixture.fails("request", &WireMap::new().with("url", url.as_str())),
            "HTTP method is required"
        );
        assert_eq!(
            fixture.fails("request", &get(&url).with("method", "TRACE")),
            "Unsupported HTTP method TRACE"
        );
        assert_eq!(
            fixture.fails("request", &get(&url).with("method", "get")),
            "Unsupported HTTP method get"
        );
        assert_eq!(
            fixture.fails("request", &get("http://example.com/")),
            "HTTP requests require HTTPS"
        );
        assert_eq!(
            fixture.fails("request", &get("https://user:pw@example.com/")),
            "Invalid HTTP URL"
        );
        assert_eq!(
            fixture.fails("request", &get(&url).with("headers", r#"{"X":"a\nb"}"#)),
            "Invalid HTTP header"
        );
        let large = get(&url)
            .with("method", "POST")
            .with("body", "b".repeat(MAX_REQUEST_BODY_BYTES + 1));
        // A payload this large never crosses the bridge; the limit stands on its own.
        assert_eq!(
            fixture
                .http
                .one_shot("request", &large)
                .map(|_| ())
                .unwrap_err(),
            "HTTP request body exceeds one MiB"
        );
        let largest = get(&url)
            .with("method", "POST")
            .with("body", "b".repeat(MAX_REQUEST_BODY_BYTES));
        assert!(fixture.http.one_shot("request", &largest).is_ok());
        assert_eq!(
            fixture.fails("request", &get(&url).with("traceparent", "00-x")),
            "Incomplete HTTP trace context"
        );
        assert_eq!(
            fixture.fails("frobnicate", &WireMap::new()),
            "Unknown HTTP method"
        );
        assert_eq!(server.requests().len(), 0);
    }

    #[test]
    fn development_allows_plain_http_to_other_hosts() {
        let fixture = Fixture::with("insecure", true);
        assert!(
            fixture
                .http
                .one_shot("request", &get("http://example.com/"))
                .is_ok()
        );
        let strict = Fixture::new("secure");
        assert!(
            strict
                .http
                .one_shot("request", &get("http://localhost:8080/"))
                .is_ok()
        );
        assert!(
            strict
                .http
                .one_shot("request", &get("https://example.com/"))
                .is_ok()
        );
    }

    #[test]
    fn transport_errors_and_oversized_responses_are_failures() {
        let limit = usize::try_from(MAX_RESPONSE_BODY_BYTES).unwrap();
        let server = TestServer::start(move |request| match request.path.as_str() {
            "/declared" => Served::ok(vec![b'a'; limit + 1]),
            "/streamed" => Served::ok(vec![b'a'; limit + 1]).without_length(),
            _ => Served::ok(vec![b'a'; limit]),
        });
        let mut fixture = Fixture::new("request-limits");
        assert_eq!(
            fixture.fails("request", &get(&server.url("/declared"))),
            RESPONSE_LIMIT
        );
        assert_eq!(
            fixture.fails("request", &get(&server.url("/streamed"))),
            RESPONSE_LIMIT
        );
        let fits = fixture.call("request", &get(&server.url("/fits"))).unwrap();
        assert_eq!(fits.text("body").map(str::len), Some(limit));
        // Nothing listens on this port.
        let closed = {
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            listener.local_addr().unwrap().port()
        };
        let error = fixture.fails("request", &get(&format!("http://127.0.0.1:{closed}/")));
        assert!(!error.is_empty());
    }

    #[test]
    fn uploads_send_a_sandbox_file_as_the_body() {
        let server = TestServer::start(|_| Served::status(201, "stored"));
        let mut fixture = Fixture::new("upload");
        let contents: Vec<u8> = (0..150_000_u32).map(|index| (index % 253) as u8).collect();
        fs::create_dir_all(fixture.root().join("media")).unwrap();
        fs::write(fixture.root().join("media/clip.bin"), &contents).unwrap();
        let request = WireMap::new()
            .with("url", server.url("/files/clip"))
            .with("method", "POST")
            .with("headers", r#"{"Content-Type":"video/mp4"}"#)
            .with("path", "media/clip.bin");
        let response = fixture.call("upload", &request).unwrap();
        assert_eq!(response.integer("statusCode"), Some(201));
        assert_eq!(response.text("body"), Some("stored"));
        let received = &server.requests()[0];
        // `upload` is always a PUT.
        assert_eq!(received.method, "PUT");
        assert_eq!(received.header("content-length"), Some("150000"));
        assert_eq!(received.header("content-type"), Some("video/mp4"));
        assert_eq!(received.body, contents);
        // The snapshot is gone once the request finished.
        let cache = fixture.directory.join("data/pam-upload-cache");
        assert_eq!(fs::read_dir(&cache).map(Iterator::count).unwrap_or(0), 0);

        let upload = |path: &str| {
            WireMap::new()
                .with("url", server.url("/x"))
                .with("headers", "{}")
                .with("path", path)
        };
        assert_eq!(
            fixture.fails("upload", &upload("media/missing.bin")),
            OUTSIDE_SANDBOX
        );
        assert_eq!(
            fixture.fails("upload", &upload("../data.bin")),
            OUTSIDE_SANDBOX
        );
        assert_eq!(fixture.fails("upload", &upload("media")), OUTSIDE_SANDBOX);
        assert_eq!(
            fixture.fails("upload", &upload("media/clip.bin").with("body", "text")),
            "File upload cannot include a text body"
        );
        assert_eq!(
            fixture.fails(
                "upload",
                &upload("media/clip.bin").with("headers", r#"{"Content-Length":"3"}"#)
            ),
            "File upload headers cannot override HTTP transport fields"
        );
        let no_path = WireMap::new()
            .with("url", server.url("/x"))
            .with("headers", "{}");
        assert_eq!(
            fixture.fails("upload", &no_path),
            "Upload source path is required"
        );
        assert_eq!(server.requests().len(), 1);
    }

    #[test]
    fn upload_snapshots_enforce_the_size_limit_without_copying() {
        let fixture = Fixture::new("snapshot");
        let root = fixture.root();
        let cache = fixture.directory.join("cache");
        let file = File::create(root.join("huge.bin")).unwrap();
        // A sparse file: large on paper, no blocks on disk.
        file.set_len(MAX_UPLOAD_BYTES + 1).unwrap();
        drop(file);
        let cancel = AtomicBool::new(false);
        assert_eq!(
            Snapshot::create(&root, &cache, "huge.bin", &cancel)
                .map(|_| ())
                .unwrap_err(),
            "Upload file exceeds 64 MiB"
        );
        assert!(!cache.exists());
        fs::write(root.join("small.bin"), b"12345").unwrap();
        let snapshot = Snapshot::create(&root, &cache, "small.bin", &cancel).unwrap();
        assert_eq!(snapshot.length, 5);
        assert_eq!(fs::read(&snapshot.path).unwrap(), b"12345");
        let path = snapshot.path.clone();
        drop(snapshot);
        assert!(!path.exists());
    }

    fn multipart_request(server: &TestServer, parts: &serde_json::Value) -> WireMap {
        WireMap::new()
            .with("kind", 1)
            .with("url", server.url("/upload"))
            .with("method", "POST")
            .with("headers", r#"{"Authorization":"Bearer t"}"#)
            .with("parts", parts.to_string())
            .with("timeoutMs", 20_000)
    }

    #[test]
    fn multipart_transfers_stream_progress_and_complete() {
        let server = TestServer::start(|_| {
            Served::status(201, r#"{"id":7}"#).header("Content-Type", "application/json")
        });
        let mut fixture = Fixture::new("transfer-multipart");
        let contents: Vec<u8> = (0..700_000_u32).map(|index| (index % 249) as u8).collect();
        fs::write(fixture.root().join("video.mp4"), &contents).unwrap();
        let parts = serde_json::json!([
            {"type": 1, "name": "title", "value": "Férias"},
            {"type": 2, "name": "file", "path": "video.mp4", "mimeType": "", "filename": ""},
        ]);
        let started = fixture
            .call("transferStart", &multipart_request(&server, &parts))
            .unwrap();
        let transfer = started.integer("transfer").unwrap();
        assert!(transfer >= 1);
        let (progress, complete) = fixture.drain(transfer);
        assert_eq!(complete.integer("state"), Some(TRANSFER_COMPLETE));
        assert_eq!(complete.integer("statusCode"), Some(201));
        assert_eq!(complete.text("body"), Some(r#"{"id":7}"#));
        let headers: serde_json::Value =
            serde_json::from_str(complete.text("headers").unwrap()).unwrap();
        assert_eq!(headers["content-type"], "application/json");

        let received = &server.requests()[0];
        assert_eq!(received.method, "POST");
        assert_eq!(received.header("authorization"), Some("Bearer t"));
        let content_type = received.header("content-type").unwrap();
        let boundary = content_type
            .strip_prefix("multipart/form-data; boundary=")
            .unwrap();
        assert!(boundary.starts_with("pam-") && boundary.len() == 40);
        let mut expected = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nFérias\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"video.mp4\"\r\n\
             Content-Type: video/mp4\r\n\r\n"
        )
        .into_bytes();
        expected.extend_from_slice(&contents);
        expected.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        assert_eq!(received.body.len(), expected.len());
        assert!(received.body == expected);
        assert_eq!(
            received.header("content-length"),
            Some(expected.len().to_string().as_str())
        );

        // The queue keeps the latest events: progress never goes backwards
        // and ends on the whole body.
        let total = i64::try_from(expected.len()).unwrap();
        assert!(progress.iter().all(|(_, of)| *of == total));
        assert!(progress.windows(2).all(|pair| pair[0].0 <= pair[1].0));
        assert_eq!(progress.last(), Some(&(total, total)));

        // After the terminal event the channel is closed; cancelling cleans up.
        let id = WireMap::new().with("transfer", transfer);
        assert_eq!(fixture.fails("transferNext", &id), "Observation is closed");
        assert!(fixture.call("transferCancel", &id).unwrap().is_empty());
        assert_eq!(
            fixture.fails("transferNext", &id),
            "HTTP transfer not found"
        );
        assert!(fixture.call("transferCancel", &id).unwrap().is_empty());
        assert_eq!(
            fixture.fails("transferNext", &WireMap::new()),
            "HTTP transfer id is required"
        );
    }

    #[test]
    fn raw_file_transfers_default_to_octet_stream() {
        let server = TestServer::start(|_| Served::ok("ok"));
        let mut fixture = Fixture::new("transfer-raw");
        fs::write(fixture.root().join("backup.db"), b"sqlite-bytes").unwrap();
        let request = WireMap::new()
            .with("kind", 2)
            .with("url", server.url("/backup"))
            .with("method", "PUT")
            .with("headers", "{}")
            .with("path", "backup.db");
        let transfer = fixture
            .call("transferStart", &request)
            .unwrap()
            .integer("transfer")
            .unwrap();
        let (progress, complete) = fixture.drain(transfer);
        assert_eq!(complete.integer("statusCode"), Some(200));
        assert_eq!(progress.first(), Some(&(0, 12)));
        assert_eq!(progress.last(), Some(&(12, 12)));
        let received = &server.requests()[0];
        assert_eq!(received.method, "PUT");
        assert_eq!(
            received.header("content-type"),
            Some("application/octet-stream")
        );
        assert_eq!(received.body, b"sqlite-bytes");
    }

    #[test]
    fn transfer_requests_are_validated() {
        let server = TestServer::start(|_| Served::ok("never"));
        let mut fixture = Fixture::new("transfer-validation");
        let field = serde_json::json!([{"type": 1, "name": "a", "value": "b"}]);
        let request = multipart_request(&server, &field);
        assert_eq!(
            fixture.fails("transferStart", &request.clone().with("method", "GET")),
            "Unsupported HTTP transfer method GET"
        );
        assert_eq!(
            fixture.fails(
                "transferStart",
                &request.clone().with("url", "http://example.com/")
            ),
            "HTTP requests require HTTPS"
        );
        assert_eq!(
            fixture.fails(
                "transferStart",
                &request.clone().with("headers", r#"{"Content-Type":"x/y"}"#)
            ),
            "Multipart requests own the Content-Type boundary"
        );
        assert_eq!(
            fixture.fails(
                "transferStart",
                &request.clone().with("headers", r#"{"Host":"evil"}"#)
            ),
            "File upload headers cannot override HTTP transport fields"
        );
        let no_parts = WireMap::new().with("url", server.url("/"));
        assert_eq!(
            fixture.fails("transferStart", &no_parts),
            "Multipart parts are required"
        );
        assert_eq!(
            fixture.fails("transferStart", &no_parts.clone().with("kind", 2)),
            "Upload source path is required"
        );
        assert_eq!(
            fixture.fails("transferStart", &WireMap::new()),
            "HTTP URL is required"
        );
        assert_eq!(server.requests().len(), 0);
    }

    #[test]
    fn transfer_failures_arrive_as_events() {
        let server = TestServer::start(|_| Served::ok("never"));
        let mut fixture = Fixture::new("transfer-failure");
        let parts = serde_json::json!([{"type": 2, "name": "f", "path": "missing.bin"}]);
        let transfer = fixture
            .call("transferStart", &multipart_request(&server, &parts))
            .unwrap()
            .integer("transfer")
            .unwrap();
        let (progress, failed) = fixture.drain(transfer);
        assert!(progress.is_empty());
        assert_eq!(failed.integer("state"), Some(TRANSFER_FAILED));
        assert_eq!(failed.text("message"), Some(OUTSIDE_SANDBOX));
        assert_eq!(server.requests().len(), 0);
    }

    #[test]
    fn cancelling_a_transfer_stops_the_upload_and_its_reader() {
        // The server never answers in time: the upload is cancelled meanwhile.
        let server = TestServer::start(|_| {
            std::thread::sleep(Duration::from_secs(3));
            Served::ok("late")
        });
        let mut fixture = Fixture::new("transfer-cancel");
        let field = serde_json::json!([{"type": 1, "name": "a", "value": "b"}]);
        let transfer = fixture
            .call("transferStart", &multipart_request(&server, &field))
            .unwrap()
            .integer("transfer")
            .unwrap();
        let id = WireMap::new().with("transfer", transfer);
        // Read what is queued, then park a reader.
        let parked = loop {
            let answer = fixture.start("transferNext", &id);
            match answer.recv_timeout(Duration::from_millis(50)) {
                Ok(_) => {}
                Err(_) => break answer,
            }
        };
        assert!(fixture.call("transferCancel", &id).unwrap().is_empty());
        assert_eq!(
            parked.recv_timeout(WAIT).unwrap(),
            (MODULE_FAILURE, b"Observation stopped".to_vec())
        );
        assert_eq!(
            fixture.fails("transferNext", &id),
            "HTTP transfer not found"
        );
    }

    #[test]
    fn a_restarted_worker_cancels_transfers_and_requests() {
        let server = TestServer::start(|_| {
            std::thread::sleep(Duration::from_millis(300));
            Served::ok(vec![b'x'; 300_000]).trickle(8192, Duration::from_millis(10))
        });
        let mut fixture = Fixture::new("reset");
        let field = serde_json::json!([{"type": 1, "name": "a", "value": "b"}]);
        let transfer = fixture
            .call("transferStart", &multipart_request(&server, &field))
            .unwrap()
            .integer("transfer")
            .unwrap();
        let pending = fixture.start("request", &get(&server.url("/slow")));
        std::thread::sleep(Duration::from_millis(50));
        fixture.http.reset();
        assert_eq!(
            fixture.fails("transferNext", &WireMap::new().with("transfer", transfer)),
            "HTTP transfer not found"
        );
        let (status, payload) = pending.recv_timeout(WAIT).unwrap();
        assert_eq!(status, MODULE_FAILURE);
        assert_eq!(
            String::from_utf8(payload).unwrap(),
            "HTTP request cancelled"
        );
    }
}
