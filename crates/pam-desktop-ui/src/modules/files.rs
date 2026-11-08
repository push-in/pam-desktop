//! `files`: the application's private file sandbox.
//!
//! Every path on the wire is relative to `<data dir>/pam-files/` and is
//! resolved through [`resolve`], which refuses anything that would leave that
//! directory, including through symbolic links. File contents cross the
//! bridge only for `read` and `write` (base64, one mebibyte at most);
//! everything else (copies, downloads, uploads, imports) moves bytes on this
//! side and hands PHP a [`FileReference`].
//!
//! File operations run in call order on one worker thread. Downloads run
//! concurrently on their own threads and report progress through a
//! [`WatchChannel`].
//!
//! The native file dialog belongs to the runtime (it needs the UI thread), so
//! `pick` and `pickMany` are not methods here: the runtime validates the
//! request with [`pick_request`], shows the dialog, and imports the selection
//! with [`import_picked`] or [`import_selection`].

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use sha2::{Digest, Sha256};

use super::channel::WatchChannel;
use super::fetch::{HttpClient, HttpRequest};
use super::http::parse_url;
use super::support::{CopyError, WorkerPool, copy_bounded, hex, required_text, uuid};
use super::{Module, Reply, SerialWorker, request_map};
use crate::wiremap::WireMap;

/// The sandbox directory inside the application's data directory.
pub const SANDBOX_DIRECTORY: &str = "pam-files";
/// Where picked and imported files are copied, inside the sandbox.
pub const IMPORTS_DIRECTORY: &str = "imports";

const MEBIBYTE: u64 = 1024 * 1024;
/// Largest file that `read` and `write` move through the bridge.
const MAX_BRIDGE_BYTES: u64 = MEBIBYTE;
/// Largest file whose base64 form still fits a one-mebibyte result: the
/// envelope of `{data}` takes 13 bytes and base64 turns 3 bytes into 4.
const MAX_READ_BYTES: u64 = ((crate::wiremap::MAX_WIRE_MAP_BYTES - 13) / 4 * 3) as u64;
/// Longest base64 text that can decode to [`MAX_BRIDGE_BYTES`].
const MAX_BRIDGE_BASE64: usize = 1_398_104;
const DEFAULT_TRANSFER_BYTES: u64 = 64 * MEBIBYTE;
const MAX_DOWNLOAD_BYTES: u64 = 256 * MEBIBYTE;
const MAX_PICK_BYTES: u64 = 8 * 1024 * MEBIBYTE;
const MIN_SELECTION_BYTES: u64 = 256 * MEBIBYTE;
const MAX_PATH_BYTES: usize = 4096;
const MAX_ASSET_PATH_BYTES: usize = 1024;
const MAX_URL_CHARACTERS: usize = 2048;
const MAX_DOWNLOAD_HEADERS: usize = 32;
const MAX_HEADER_VALUE_CHARACTERS: usize = 8192;
const DEFAULT_PICK_LIMIT: i64 = 10;
const MAX_PICK_LIMIT: i64 = 50;
/// Listings larger than this cannot cross the bridge in one result.
const MAX_LISTING_BYTES: usize = 1_048_576 - 64;
/// Downloads running at once; the rest wait for a free thread.
const DOWNLOAD_THREADS: usize = 8;

const DOWNLOAD_PROGRESS: i64 = 1;
const DOWNLOAD_COMPLETE: i64 = 2;
const DOWNLOAD_FAILED: i64 = 3;

/// The message PHP recognises as "this file is not there".
pub const MISSING_FILE: &str = "File does not exist";
const ESCAPES_SANDBOX: &str = "File path escapes sandbox";
const INVALID_PATH: &str = "Invalid file path";
const EMPTY_PATH: &str = "File path cannot be empty";
const BRIDGE_LIMIT: &str = "File exceeds the one MiB bridge limit";
const FALLBACK: &str = "File operation failed";
const DOWNLOAD_LIMIT: &str = "Download exceeds configured size limit";
const DOWNLOAD_CANCELLED: &str = "Download cancelled";
const NO_APPLICATION: &str = "No application can open this file type";
const UNREADABLE_SELECTION: &str = "Unable to read selected file";
const DEFAULT_MIME: &str = "application/octet-stream";

// ---- MIME types -----------------------------------------------------------

/// Extension (lower-case, no dot) to MIME type, for the common formats.
const MIME_TYPES: &[(&str, &str)] = &[
    ("3gp", "video/3gpp"),
    ("7z", "application/x-7z-compressed"),
    ("aac", "audio/aac"),
    ("apk", "application/vnd.android.package-archive"),
    ("avi", "video/x-msvideo"),
    ("avif", "image/avif"),
    ("bin", "application/octet-stream"),
    ("bmp", "image/bmp"),
    ("bz2", "application/x-bzip2"),
    ("css", "text/css"),
    ("csv", "text/csv"),
    ("doc", "application/msword"),
    (
        "docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
    ("eot", "application/vnd.ms-fontobject"),
    ("epub", "application/epub+zip"),
    ("flac", "audio/flac"),
    ("gif", "image/gif"),
    ("gz", "application/gzip"),
    ("heic", "image/heic"),
    ("heif", "image/heif"),
    ("htm", "text/html"),
    ("html", "text/html"),
    ("ico", "image/vnd.microsoft.icon"),
    ("ics", "text/calendar"),
    ("jpeg", "image/jpeg"),
    ("jpg", "image/jpeg"),
    ("js", "text/javascript"),
    ("json", "application/json"),
    ("m4a", "audio/mp4"),
    ("m4v", "video/x-m4v"),
    ("md", "text/markdown"),
    ("mid", "audio/midi"),
    ("midi", "audio/midi"),
    ("mjs", "text/javascript"),
    ("mkv", "video/x-matroska"),
    ("mov", "video/quicktime"),
    ("mp3", "audio/mpeg"),
    ("mp4", "video/mp4"),
    ("mpeg", "video/mpeg"),
    ("mpg", "video/mpeg"),
    ("odp", "application/vnd.oasis.opendocument.presentation"),
    ("ods", "application/vnd.oasis.opendocument.spreadsheet"),
    ("odt", "application/vnd.oasis.opendocument.text"),
    ("oga", "audio/ogg"),
    ("ogg", "audio/ogg"),
    ("ogv", "video/ogg"),
    ("opus", "audio/opus"),
    ("otf", "font/otf"),
    ("pdf", "application/pdf"),
    ("png", "image/png"),
    ("ppt", "application/vnd.ms-powerpoint"),
    (
        "pptx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ),
    ("rar", "application/vnd.rar"),
    ("rtf", "application/rtf"),
    ("sqlite", "application/vnd.sqlite3"),
    ("svg", "image/svg+xml"),
    ("tar", "application/x-tar"),
    ("tif", "image/tiff"),
    ("tiff", "image/tiff"),
    ("ttf", "font/ttf"),
    ("txt", "text/plain"),
    ("vcf", "text/vcard"),
    ("wav", "audio/wav"),
    ("weba", "audio/webm"),
    ("webm", "video/webm"),
    ("webp", "image/webp"),
    ("woff", "font/woff"),
    ("woff2", "font/woff2"),
    ("xhtml", "application/xhtml+xml"),
    ("xls", "application/vnd.ms-excel"),
    (
        "xlsx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ),
    ("xml", "application/xml"),
    ("yaml", "application/yaml"),
    ("yml", "application/yaml"),
    ("zip", "application/zip"),
];

/// The MIME type of a file name by its extension, case-insensitively;
/// `application/octet-stream` when the extension is unknown or absent.
#[must_use]
pub fn mime_for_name(name: &str) -> &'static str {
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return DEFAULT_MIME;
    };
    if stem.is_empty() || extension.len() > 8 {
        return DEFAULT_MIME;
    }
    let extension = extension.to_ascii_lowercase();
    MIME_TYPES
        .binary_search_by(|(known, _)| (*known).cmp(extension.as_str()))
        .map_or(DEFAULT_MIME, |index| MIME_TYPES[index].1)
}

/// The MIME type of the file at `path`, by its name.
#[must_use]
pub fn mime_for_path(path: &Path) -> &'static str {
    path.file_name()
        .and_then(|name| name.to_str())
        .map_or(DEFAULT_MIME, mime_for_name)
}

/// Known extensions of a MIME type or of a `type/*` pattern. `*/*` and
/// unknown types give no extension, meaning "any file".
#[must_use]
pub fn extensions_for_mime(pattern: &str) -> Vec<&'static str> {
    let pattern = pattern.to_ascii_lowercase();
    let family = pattern.strip_suffix("/*").filter(|family| *family != "*");
    MIME_TYPES
        .iter()
        .filter(|(_, mime)| match family {
            Some(family) => mime
                .strip_prefix(family)
                .is_some_and(|rest| rest.starts_with('/')),
            None => *mime == pattern,
        })
        .map(|(extension, _)| *extension)
        .collect()
}

// ---- paths ----------------------------------------------------------------

/// The sandbox root of an application whose data directory is `data_dir`.
#[must_use]
pub fn sandbox_root(data_dir: &Path) -> PathBuf {
    data_dir.join(SANDBOX_DIRECTORY)
}

/// Splits a wire path into its segments, refusing anything that is not a
/// plain relative path: empty, absolute, with `\`, control characters, or
/// `.`/`..`/empty segments.
fn segments(path: &str) -> Result<Vec<&str>, String> {
    if path.trim().is_empty() {
        return Err(EMPTY_PATH.into());
    }
    if path.len() > MAX_PATH_BYTES || path.chars().any(char::is_control) {
        return Err(INVALID_PATH.into());
    }
    if path.starts_with('/') || path.contains('\\') {
        return Err(ESCAPES_SANDBOX.into());
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments
        .iter()
        .any(|segment| matches!(*segment, "" | "." | ".."))
    {
        return Err(ESCAPES_SANDBOX.into());
    }
    // Windows gives drive letters, streams and device names a meaning inside
    // a single path segment, and ignores trailing dots and spaces.
    if cfg!(windows)
        && segments.iter().any(|segment| {
            segment.contains(['<', '>', ':', '"', '|', '?', '*']) || segment.ends_with(['.', ' '])
        })
    {
        return Err(INVALID_PATH.into());
    }
    Ok(segments)
}

/// The canonical sandbox root, created when missing.
fn canonical_root(root: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(root)
        .and_then(|()| fs::canonicalize(root))
        .map_err(|_| String::from("Private files directory is unavailable"))
}

/// Resolves a sandbox-relative wire path to an absolute path inside `root`.
///
/// The path does not have to exist. Existing ancestors are followed through
/// symbolic links, and the result is refused (`File path escapes sandbox`)
/// when any of them leads outside `root`. `root` is created when missing.
pub fn resolve(root: &Path, path: &str) -> Result<PathBuf, String> {
    let segments = segments(path)?;
    let root = canonical_root(root)?;
    let mut resolved = root.clone();
    let mut remaining = segments.iter();
    for segment in remaining.by_ref() {
        let candidate = resolved.join(segment);
        let Ok(metadata) = fs::symlink_metadata(&candidate) else {
            // Nothing exists from here on, so no link can redirect the rest.
            resolved = candidate;
            break;
        };
        resolved = if metadata.file_type().is_symlink() {
            fs::canonicalize(&candidate).map_err(|_| String::from(ESCAPES_SANDBOX))?
        } else {
            candidate
        };
        if !resolved.starts_with(&root) {
            return Err(ESCAPES_SANDBOX.into());
        }
    }
    resolved.extend(remaining);
    if resolved == root {
        return Err(ESCAPES_SANDBOX.into());
    }
    Ok(resolved)
}

/// The wire form (`/`-separated, relative) of a path inside `root`; both must
/// be canonical, as [`resolve`] returns them.
fn relative(root: &Path, path: &Path) -> Result<String, String> {
    let rest = path
        .strip_prefix(root)
        .map_err(|_| String::from(ESCAPES_SANDBOX))?;
    let parts: Vec<&str> = rest
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect();
    if parts.is_empty() {
        return Err(ESCAPES_SANDBOX.into());
    }
    Ok(parts.join("/"))
}

fn percent_decode(segment: &str) -> Option<String> {
    let bytes = segment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let digits = segment.get(index + 1..index + 3)?;
            decoded.push(u8::from_str_radix(digits, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

/// Maps a renderer URI (`pam-file:///<percent-encoded segments>`, as
/// `FileReference::uri()` builds it) to a path inside the sandbox `root`.
///
/// `None` for any other scheme, a URI with an authority
/// (`pam-file://host/…`), a query or fragment, a malformed escape, or a path
/// that [`resolve`] refuses. The file itself does not have to exist.
#[must_use]
pub fn resolve_uri(root: &Path, uri: &str) -> Option<PathBuf> {
    let scheme = uri.get(..11)?;
    if !scheme.eq_ignore_ascii_case("pam-file://") {
        return None;
    }
    let path = uri[11..].strip_prefix('/')?;
    if path.contains(['?', '#']) {
        return None;
    }
    let mut decoded = Vec::new();
    for segment in path.split('/') {
        let segment = percent_decode(segment)?;
        // An escaped separator must not smuggle extra segments in.
        if segment.contains(['/', '\\']) {
            return None;
        }
        decoded.push(segment);
    }
    resolve(root, &decoded.join("/")).ok()
}

// ---- file references ------------------------------------------------------

/// What PHP receives for a file in the sandbox.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileReference {
    /// Sandbox-relative, `/`-separated.
    pub path: String,
    /// The display name: the file name, or the original name of an import.
    pub name: String,
    pub mime_type: String,
    /// Bytes; zero for a directory.
    pub size: u64,
}

impl FileReference {
    /// The reference of an existing entry inside the canonical `root`.
    fn of(root: &Path, file: &Path, mime_type: Option<String>) -> Result<Self, String> {
        let metadata = fs::metadata(file).map_err(|_| String::from(MISSING_FILE))?;
        Ok(Self {
            path: relative(root, file)?,
            name: file
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            mime_type: mime_type.unwrap_or_else(|| mime_for_path(file).to_owned()),
            size: if metadata.is_file() {
                metadata.len()
            } else {
                0
            },
        })
    }

    /// The module result: `path`, `name`, `mimeType`, `size`.
    #[must_use]
    pub fn to_map(&self) -> WireMap {
        WireMap::new()
            .with("path", self.path.as_str())
            .with("name", self.name.as_str())
            .with("mimeType", self.mime_type.as_str())
            .with("size", i64::try_from(self.size).unwrap_or(i64::MAX))
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "path": self.path,
            "name": self.name,
            "mimeType": self.mime_type,
            "size": self.size,
        })
    }
}

/// The `{items}` result of `list` and `pickMany`: a JSON array of file
/// references, in the given order. An empty slice is what a cancelled
/// `pickMany` answers.
#[must_use]
pub fn items_map(items: &[FileReference]) -> WireMap {
    let items: Vec<serde_json::Value> = items.iter().map(FileReference::to_json).collect();
    WireMap::new().with("items", serde_json::Value::Array(items).to_string())
}

// ---- pickers --------------------------------------------------------------

/// What the user may choose in a file dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickKind {
    Image,
    Video,
    Audio,
    Any,
    /// Images and videos.
    Media,
}

/// A validated `pick` or `pickMany` request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickRequest {
    pub kind: PickKind,
    /// A `type/subtype` filter that narrows `kind` (`pick` only).
    pub mime_type: Option<String>,
    /// Largest file accepted, in bytes.
    pub maximum_bytes: u64,
    /// Most files accepted by `pickMany`.
    pub limit: usize,
}

fn valid_picker_mime(mime: &str) -> bool {
    let token = |text: &str, wildcard: bool| {
        !text.is_empty()
            && text.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(
                        byte,
                        b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-'
                    )
                    || (wildcard && byte == b'*')
            })
    };
    mime.split_once('/')
        .is_some_and(|(kind, subtype)| token(kind, false) && token(subtype, true))
}

/// Validates a `pick` or `pickMany` request: `type` (1 image, 2 video,
/// 3 audio, 4 any, 5 media; default any), `mimeType`, `maximumBytes`
/// (1 byte to 8 GiB, default 64 MiB) and `limit` (1 to 50, default 10).
pub fn pick_request(request: &WireMap) -> Result<PickRequest, String> {
    let maximum_bytes = match request.integer("maximumBytes") {
        None => DEFAULT_TRANSFER_BYTES,
        Some(bytes) => u64::try_from(bytes)
            .ok()
            .filter(|bytes| (1..=MAX_PICK_BYTES).contains(bytes))
            .ok_or("Picker import limit must be between 1 byte and 8 GiB")?,
    };
    let mime_type = request.text("mimeType").unwrap_or_default();
    if !mime_type.is_empty() && !valid_picker_mime(mime_type) {
        return Err("Picker MIME type is invalid".into());
    }
    let kind = match request.integer("type") {
        Some(1) => PickKind::Image,
        Some(2) => PickKind::Video,
        Some(3) => PickKind::Audio,
        Some(5) => PickKind::Media,
        _ => PickKind::Any,
    };
    let limit = request
        .integer("limit")
        .unwrap_or(DEFAULT_PICK_LIMIT)
        .clamp(1, MAX_PICK_LIMIT);
    Ok(PickRequest {
        kind,
        mime_type: (!mime_type.is_empty()).then(|| mime_type.to_owned()),
        maximum_bytes,
        limit: usize::try_from(limit).unwrap_or(1),
    })
}

impl PickRequest {
    /// File extensions the dialog should offer; empty means any file.
    #[must_use]
    pub fn extensions(&self) -> Vec<&'static str> {
        if let Some(mime) = &self.mime_type {
            return extensions_for_mime(mime);
        }
        let families: &[&str] = match self.kind {
            PickKind::Image => &["image/*"],
            PickKind::Video => &["video/*"],
            PickKind::Audio => &["audio/*"],
            PickKind::Media => &["image/*", "video/*"],
            PickKind::Any => &[],
        };
        families
            .iter()
            .flat_map(|family| extensions_for_mime(family))
            .collect()
    }

    /// Bytes a whole `pickMany` selection may import: at least 256 MiB, the
    /// per-file limit when that is larger, never more than 8 GiB.
    #[must_use]
    pub fn selection_bytes(&self) -> u64 {
        self.maximum_bytes
            .clamp(MIN_SELECTION_BYTES, MAX_PICK_BYTES)
    }
}

/// Removes a file when dropped, unless it was kept.
struct Staged {
    path: PathBuf,
    keep: bool,
}

impl Staged {
    /// Creates `path` exclusively: an existing file is never reused.
    fn create(path: PathBuf) -> std::io::Result<(Self, File)> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok((Self { path, keep: false }, file))
    }

    /// Publishes the staged file as `target`, replacing it atomically.
    fn publish(mut self, target: &Path) -> std::io::Result<()> {
        fs::rename(&self.path, target)?;
        self.keep = true;
        Ok(())
    }

    /// Leaves the file where it was created.
    fn keep(mut self) {
        self.keep = true;
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn exceeds_message(maximum_bytes: u64) -> String {
    format!("Selected file exceeds {} MiB", maximum_bytes / MEBIBYTE)
}

/// Copies a file the user picked into `imports/<uuid>-<sanitised name>`
/// inside the sandbox `root` and returns its reference, whose `name` is the
/// original file name.
///
/// Fails with `Selected file exceeds <n> MiB` when the file is larger than
/// `maximum_bytes` (nothing is left behind) and with
/// `Unable to read selected file` when it cannot be read.
pub fn import_picked(
    root: &Path,
    source: &Path,
    maximum_bytes: u64,
) -> Result<FileReference, String> {
    let unreadable = |_| String::from(UNREADABLE_SELECTION);
    let name = source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| String::from("document"));
    let mut input = File::open(source).map_err(unreadable)?;
    let metadata = input.metadata().map_err(unreadable)?;
    if !metadata.is_file() {
        return Err(UNREADABLE_SELECTION.into());
    }
    if metadata.len() > maximum_bytes {
        return Err(exceeds_message(maximum_bytes));
    }
    let safe: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-') {
                character
            } else {
                '_'
            }
        })
        .take(128)
        .collect();
    let root = canonical_root(root)?;
    let directory = resolve(&root, IMPORTS_DIRECTORY)?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let target = directory.join(format!("{}-{safe}", uuid()));
    let (staged, mut output) = Staged::create(target.clone()).map_err(|error| error.to_string())?;
    let never = AtomicBool::new(false);
    let size =
        copy_bounded(&mut input, &mut output, maximum_bytes, &never, |_| {}).map_err(|error| {
            match error {
                CopyError::TooLarge => exceeds_message(maximum_bytes),
                CopyError::Cancelled | CopyError::Io(_) => String::from(UNREADABLE_SELECTION),
            }
        })?;
    drop(output);
    staged.keep();
    Ok(FileReference {
        path: relative(&root, &target)?,
        mime_type: mime_for_name(&name).to_owned(),
        name,
        size,
    })
}

/// Imports a `pickMany` selection in order, at most `request.limit` files.
///
/// Each file may use `request.maximum_bytes` and the whole selection
/// [`PickRequest::selection_bytes`]. On any failure the files already
/// imported are deleted. An empty selection is `No file was selected`; a
/// cancelled dialog is not a selection (answer [`items_map`] of nothing).
pub fn import_selection(
    root: &Path,
    sources: &[PathBuf],
    request: &PickRequest,
) -> Result<Vec<FileReference>, String> {
    if sources.is_empty() {
        return Err("No file was selected".into());
    }
    let selection = request.selection_bytes();
    let mut imported: Vec<FileReference> = Vec::new();
    let mut total = 0_u64;
    let mut import_all = || -> Result<(), String> {
        for source in sources.iter().take(request.limit) {
            let remaining = selection.saturating_sub(total);
            if remaining == 0 {
                return Err(format!(
                    "Selected files exceed {} MiB",
                    selection / MEBIBYTE
                ));
            }
            let reference = import_picked(root, source, request.maximum_bytes.min(remaining))?;
            total += reference.size;
            imported.push(reference);
        }
        Ok(())
    };
    match import_all() {
        Ok(()) => Ok(imported),
        Err(message) => {
            for reference in &imported {
                if let Ok(path) = resolve(root, &reference.path) {
                    let _ = fs::remove_file(path);
                }
            }
            Err(message)
        }
    }
}

// ---- file operations ------------------------------------------------------

fn io_message(error: &std::io::Error) -> String {
    let message = error.to_string();
    if message.is_empty() {
        FALLBACK.into()
    } else {
        message
    }
}

fn normalized_asset_path(path: &str) -> Result<String, String> {
    let normalized = path.trim().replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.len() > MAX_ASSET_PATH_BYTES
        || normalized.chars().any(char::is_control)
    {
        return Err("Invalid bundled asset path".into());
    }
    if normalized
        .split('/')
        .any(|segment| matches!(segment, "" | "." | ".."))
    {
        return Err("Bundled asset path escapes application bundle".into());
    }
    Ok(normalized)
}

/// The sandbox and the bundled assets, shared with the worker threads.
struct Sandbox {
    root: PathBuf,
    assets: PathBuf,
}

impl Sandbox {
    fn resolve(&self, path: &str) -> Result<PathBuf, String> {
        resolve(&self.root, path)
    }

    fn reference(&self, file: &Path, mime_type: Option<String>) -> Result<FileReference, String> {
        FileReference::of(&canonical_root(&self.root)?, file, mime_type)
    }

    /// Runs one serial file operation and encodes its result.
    fn run(&self, method: &str, request: &WireMap) -> Result<Vec<u8>, String> {
        match method {
            "read" => self.read(required_text(request, "path")?),
            "write" => self.write(
                required_text(request, "path")?,
                required_text(request, "data")?,
            ),
            "sha256" => self.sha256(required_text(request, "path")?),
            "copyAsset" => self.copy_asset(
                required_text(request, "assetPath")?,
                required_text(request, "path")?,
            ),
            "stat" => self.stat(required_text(request, "path")?),
            "exists" => self.exists(required_text(request, "path")?),
            "list" => self.list(required_text(request, "path")?),
            "delete" => self.delete(required_text(request, "path")?),
            "move" | "copy" => self.transfer(
                required_text(request, "from")?,
                required_text(request, "to")?,
                method == "move",
                request.boolean("overwrite").unwrap_or(false),
            ),
            "makeDirectory" => self.make_directory(required_text(request, "path")?),
            other => Err(format!("Unknown files method {other}")),
        }
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        let file = self.resolve(path)?;
        let metadata = fs::metadata(&file).map_err(|_| String::from(MISSING_FILE))?;
        if !metadata.is_file() {
            return Err(MISSING_FILE.into());
        }
        if metadata.len() > MAX_READ_BYTES {
            return Err(BRIDGE_LIMIT.into());
        }
        // The file may grow between the check and the read: never read past the limit.
        let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
        File::open(&file)
            .and_then(|input| input.take(MAX_READ_BYTES + 1).read_to_end(&mut bytes))
            .map_err(|error| io_message(&error))?;
        if bytes.len() as u64 > MAX_READ_BYTES {
            return Err(BRIDGE_LIMIT.into());
        }
        Ok(WireMap::new().with("data", BASE64.encode(bytes)).encode())
    }

    fn write(&self, path: &str, data: &str) -> Result<Vec<u8>, String> {
        let file = self.resolve(path)?;
        if data.len() > MAX_BRIDGE_BASE64 {
            return Err(BRIDGE_LIMIT.into());
        }
        let bytes = BASE64
            .decode(data)
            .map_err(|_| String::from("Invalid file payload"))?;
        if bytes.len() as u64 > MAX_BRIDGE_BYTES {
            return Err(BRIDGE_LIMIT.into());
        }
        let parent = self.parent(&file)?;
        let (staged, mut output) =
            Staged::create(staging_path(&parent, &file)).map_err(|error| io_message(&error))?;
        output
            .write_all(&bytes)
            .and_then(|()| output.sync_all())
            .map_err(|error| io_message(&error))?;
        drop(output);
        staged.publish(&file).map_err(|error| io_message(&error))?;
        Ok(Vec::new())
    }

    /// The parent directory of a resolved path, created when missing.
    fn parent(&self, file: &Path) -> Result<PathBuf, String> {
        let parent = file.parent().ok_or(ESCAPES_SANDBOX)?.to_path_buf();
        fs::create_dir_all(&parent)
            .map_err(|_| String::from("Unable to create destination directory"))?;
        Ok(parent)
    }

    fn sha256(&self, path: &str) -> Result<Vec<u8>, String> {
        let file = self.resolve(path)?;
        if !file.is_file() {
            return Err(MISSING_FILE.into());
        }
        let mut input = File::open(&file).map_err(|error| io_message(&error))?;
        let mut digest = Sha256::new();
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            let count = input
                .read(&mut buffer)
                .map_err(|error| io_message(&error))?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        Ok(WireMap::new()
            .with("sha256", hex(&digest.finalize()))
            .encode())
    }

    fn copy_asset(&self, asset_path: &str, path: &str) -> Result<Vec<u8>, String> {
        let asset_path = normalized_asset_path(asset_path)?;
        let file = self.resolve(path)?;
        let missing = |_| String::from("Bundled asset does not exist");
        let assets = fs::canonicalize(&self.assets).map_err(missing)?;
        let source = fs::canonicalize(assets.join(&asset_path)).map_err(missing)?;
        if !source.starts_with(&assets) {
            return Err("Bundled asset path escapes application bundle".into());
        }
        if !source.is_file() {
            return Err("Bundled asset does not exist".into());
        }
        let parent = self.parent(&file)?;
        let (staged, mut output) =
            Staged::create(staging_path(&parent, &file)).map_err(|error| io_message(&error))?;
        let mut input = File::open(&source).map_err(missing)?;
        std::io::copy(&mut input, &mut output)
            .and_then(|_| output.sync_all())
            .map_err(|error| io_message(&error))?;
        drop(output);
        staged.publish(&file).map_err(|error| io_message(&error))?;
        Ok(self.reference(&file, None)?.to_map().encode())
    }

    fn stat(&self, path: &str) -> Result<Vec<u8>, String> {
        let file = self.resolve(path)?;
        if !file.is_file() {
            return Err(MISSING_FILE.into());
        }
        Ok(self.reference(&file, None)?.to_map().encode())
    }

    fn exists(&self, path: &str) -> Result<Vec<u8>, String> {
        let file = self.resolve(path)?;
        Ok(WireMap::new()
            .with("exists", file.is_file())
            .with("directory", file.is_dir())
            .encode())
    }

    fn list(&self, path: &str) -> Result<Vec<u8>, String> {
        let root = canonical_root(&self.root)?;
        let directory = if path.trim().is_empty() {
            root.clone()
        } else {
            self.resolve(path)?
        };
        let entries =
            fs::read_dir(&directory).map_err(|_| String::from("Directory does not exist"))?;
        let mut files: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .collect();
        files.sort_by_key(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
        });
        let mut items = Vec::with_capacity(files.len());
        let mut bytes = 2_usize;
        for file in &files {
            let reference = FileReference::of(&root, file, None)?;
            // A rough size of the entry's JSON, checked before building it all.
            bytes += reference.path.len() + reference.name.len() + reference.mime_type.len() + 64;
            if bytes > MAX_LISTING_BYTES {
                return Err("Directory listing exceeds the one MiB bridge limit".into());
            }
            items.push(reference);
        }
        let result = items_map(&items).encode();
        if result.len() > crate::wiremap::MAX_WIRE_MAP_BYTES {
            return Err("Directory listing exceeds the one MiB bridge limit".into());
        }
        Ok(result)
    }

    fn delete(&self, path: &str) -> Result<Vec<u8>, String> {
        let file = self.resolve(path)?;
        if !file.is_file() {
            return Err(MISSING_FILE.into());
        }
        fs::remove_file(&file).map_err(|_| String::from("Unable to delete file"))?;
        Ok(Vec::new())
    }

    fn transfer(
        &self,
        from: &str,
        to: &str,
        moving: bool,
        overwrite: bool,
    ) -> Result<Vec<u8>, String> {
        let source = self.resolve(from)?;
        let target = self.resolve(to)?;
        if !source.exists() {
            return Err("Source does not exist".into());
        }
        if source == target {
            return Err("Source and destination are the same".into());
        }
        if !moving && !source.is_file() {
            return Err("Only files can be copied".into());
        }
        if target.starts_with(&source) {
            return Err("Cannot move a directory inside itself".into());
        }
        if target.exists() {
            if !overwrite {
                return Err("Destination already exists".into());
            }
            if !(target.is_file() && source.is_file()) {
                return Err("Only files can replace an existing destination".into());
            }
        }
        let parent = self.parent(&target)?;
        if moving {
            fs::rename(&source, &target).map_err(|error| io_message(&error))?;
        } else {
            let name = target
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let staging = parent.join(format!(".{name}.tmp-{}", uuid()));
            let (staged, mut output) =
                Staged::create(staging).map_err(|error| io_message(&error))?;
            let expected = fs::metadata(&source)
                .map_err(|error| io_message(&error))?
                .len();
            let mut input = File::open(&source).map_err(|error| io_message(&error))?;
            let copied = std::io::copy(&mut input, &mut output)
                .and_then(|copied| output.sync_all().map(|()| copied))
                .map_err(|error| io_message(&error))?;
            drop(output);
            if copied != expected {
                return Err("Source changed during copy".into());
            }
            staged
                .publish(&target)
                .map_err(|error| io_message(&error))?;
        }
        Ok(self.reference(&target, None)?.to_map().encode())
    }

    fn make_directory(&self, path: &str) -> Result<Vec<u8>, String> {
        let directory = self.resolve(path)?;
        if directory.is_file() {
            return Err("A file already exists at this path".into());
        }
        fs::create_dir_all(&directory).map_err(|_| String::from("Unable to create directory"))?;
        let root = canonical_root(&self.root)?;
        Ok(WireMap::new()
            .with("path", relative(&root, &directory)?)
            .encode())
    }
}

/// Where the new contents of `file` are staged: next to it, so publishing is
/// a rename inside one directory.
fn staging_path(parent: &Path, file: &Path) -> PathBuf {
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    parent.join(format!("{name}.tmp-{}", uuid()))
}

// ---- opening files --------------------------------------------------------

/// The program that opens `path` with the default application of the
/// system. The path is one argument of a direct process launch, never a
/// shell command line.
fn opener_command(path: &Path) -> std::process::Command {
    if cfg!(target_os = "macos") {
        let mut command = std::process::Command::new("open");
        command.arg(path);
        command
    } else if cfg!(windows) {
        // Canonical Windows paths carry a `\\?\` prefix the shell does not accept.
        let text = path.to_string_lossy();
        let plain = text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned();
        let mut command = std::process::Command::new("rundll32");
        command.arg("url.dll,FileProtocolHandler").arg(plain);
        command
    } else {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(path);
        command
    }
}

/// Opens `path` in its default application. The launcher reports a missing
/// handler through its exit status shortly after it starts; a launcher that
/// is still running after that is taken as a viewer that opened.
fn launch(path: &Path) -> Result<(), String> {
    let mut child = opener_command(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| String::from(NO_APPLICATION))?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => return Err(NO_APPLICATION.into()),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                // Still running: collect its exit status later, off this thread.
                let _ = std::thread::Builder::new()
                    .name("pam-files-open".into())
                    .spawn(move || {
                        let _ = child.wait();
                    });
                return Ok(());
            }
            Err(_) => return Err(FALLBACK.into()),
        }
    }
}

// ---- downloads ------------------------------------------------------------

/// A validated `download` / `downloadStart` request.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DownloadRequest {
    url: String,
    path: String,
    maximum_bytes: u64,
    headers: Vec<(String, String)>,
}

fn valid_header_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                        | b'-'
                )
        })
}

/// Decodes the `headers` JSON object of a download.
fn download_headers(json: &str) -> Result<Vec<(String, String)>, String> {
    const UNSAFE: &str = "Download request headers are invalid or unsafe";
    if json.trim().is_empty() {
        return Ok(Vec::new());
    }
    let headers = match serde_json::from_str::<serde_json::Value>(json) {
        Ok(serde_json::Value::Object(headers)) => headers,
        // PHP encodes an empty list of headers as an empty array.
        Ok(serde_json::Value::Array(items)) if items.is_empty() => return Ok(Vec::new()),
        _ => return Err(UNSAFE.into()),
    };
    if headers.len() > MAX_DOWNLOAD_HEADERS {
        return Err("Downloads accept at most 32 request headers".into());
    }
    let mut decoded = Vec::with_capacity(headers.len());
    for (name, value) in headers {
        let Some(value) = value.as_str() else {
            return Err(UNSAFE.into());
        };
        if !valid_header_name(&name)
            || value.chars().count() > MAX_HEADER_VALUE_CHARACTERS
            || value
                .chars()
                .any(|character| character.is_control() && character != '\t')
        {
            return Err(UNSAFE.into());
        }
        if matches!(
            name.to_ascii_lowercase().as_str(),
            "connection" | "content-length" | "host" | "transfer-encoding"
        ) {
            return Err("Download request header is not allowed".into());
        }
        decoded.push((name, value.to_owned()));
    }
    Ok(decoded)
}

fn download_request(request: &WireMap) -> Result<DownloadRequest, String> {
    let url = required_text(request, "url")?;
    let parsed = parse_url(url)
        .filter(|parsed| parsed.scheme == "https" && url.chars().count() <= MAX_URL_CHARACTERS)
        .ok_or("Download URL must be an absolute HTTPS URL")?;
    if parsed.credentials {
        return Err("Download URL cannot contain credentials".into());
    }
    let maximum_bytes = match request.integer("maximumBytes") {
        None => DEFAULT_TRANSFER_BYTES,
        Some(bytes) => u64::try_from(bytes)
            .ok()
            .filter(|bytes| (1..=MAX_DOWNLOAD_BYTES).contains(bytes))
            .ok_or("Invalid download size limit")?,
    };
    let path = required_text(request, "path")?;
    segments(path)?;
    Ok(DownloadRequest {
        url: url.to_owned(),
        path: path.to_owned(),
        maximum_bytes,
        headers: download_headers(request.text("headers").unwrap_or_default())?,
    })
}

/// The media type of a `Content-Type` header, without its parameters, when
/// it looks like one.
fn response_mime(content_type: Option<&str>) -> Option<String> {
    let mime = content_type?.split(';').next()?.trim();
    let plausible = mime.len() <= 255
        && mime.split_once('/').is_some_and(|(kind, subtype)| {
            !kind.is_empty()
                && !subtype.is_empty()
                && mime
                    .bytes()
                    .all(|byte| byte.is_ascii_graphic() && byte != b'"')
        });
    plausible.then(|| mime.to_ascii_lowercase())
}

/// Downloads into the sandbox: a temporary file next to the destination,
/// renamed over it once complete. `progress` receives the bytes written and
/// the declared length (zero when the server did not declare one).
fn download(
    client: &dyn HttpClient,
    sandbox: &Sandbox,
    request: &DownloadRequest,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> Result<FileReference, String> {
    let cancelled = || cancel.load(Ordering::Relaxed);
    let target = sandbox.resolve(&request.path)?;
    if target.is_dir() {
        return Err("Only files can replace an existing destination".into());
    }
    let parent = sandbox.parent(&target)?;
    let mut headers = vec![(String::from("Accept-Encoding"), String::from("identity"))];
    headers.extend(request.headers.iter().cloned());
    let outgoing = HttpRequest {
        headers,
        connect_timeout: Duration::from_secs(15),
        read_timeout: Duration::from_secs(30),
        ..HttpRequest::get(request.url.as_str())
    };
    if cancelled() {
        return Err(DOWNLOAD_CANCELLED.into());
    }
    let mut response = client.send(outgoing, None, cancel).map_err(|error| {
        if cancelled() {
            String::from(DOWNLOAD_CANCELLED)
        } else if error.is_empty() {
            String::from("Download failed")
        } else {
            error
        }
    })?;
    if !(200..300).contains(&response.status) {
        return Err(format!("Download failed with HTTP {}", response.status));
    }
    let declared = response.content_length;
    if declared.is_some_and(|length| length > request.maximum_bytes) {
        return Err(DOWNLOAD_LIMIT.into());
    }
    let (staged, mut output) =
        Staged::create(staging_path(&parent, &target)).map_err(|error| io_message(&error))?;
    let written = copy_bounded(
        &mut response.body,
        &mut output,
        request.maximum_bytes,
        cancel,
        |written| progress(written, declared.unwrap_or(0)),
    )
    .map_err(|error| match error {
        CopyError::Cancelled => String::from(DOWNLOAD_CANCELLED),
        CopyError::TooLarge => String::from(DOWNLOAD_LIMIT),
        CopyError::Io(_) if cancelled() => String::from(DOWNLOAD_CANCELLED),
        CopyError::Io(message) => message,
    })?;
    if declared.is_some_and(|length| length != written) {
        return Err("Download was interrupted".into());
    }
    output.sync_all().map_err(|error| io_message(&error))?;
    drop(output);
    if cancelled() {
        return Err(DOWNLOAD_CANCELLED.into());
    }
    staged
        .publish(&target)
        .map_err(|error| io_message(&error))?;
    sandbox.reference(&target, response_mime(response.header("content-type")))
}

fn progress_event(written: u64, total: u64) -> WireMap {
    WireMap::new()
        .with("state", DOWNLOAD_PROGRESS)
        .with("bytesWritten", i64::try_from(written).unwrap_or(i64::MAX))
        .with("totalBytes", i64::try_from(total).unwrap_or(i64::MAX))
}

fn complete_event(reference: &FileReference) -> WireMap {
    let size = i64::try_from(reference.size).unwrap_or(i64::MAX);
    reference
        .to_map()
        .with("state", DOWNLOAD_COMPLETE)
        .with("bytesWritten", size)
        .with("totalBytes", size)
}

fn failed_event(message: &str) -> WireMap {
    WireMap::new()
        .with("state", DOWNLOAD_FAILED)
        .with("bytesWritten", 0)
        .with("totalBytes", 0)
        .with("message", message)
}

/// A streamed download: its event channel and its cancellation flag.
struct Download {
    channel: Arc<WatchChannel<WireMap>>,
    cancel: Arc<AtomicBool>,
}

impl Download {
    fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.channel.close();
    }
}

// ---- module ---------------------------------------------------------------

pub struct Files {
    sandbox: Arc<Sandbox>,
    client: Arc<dyn HttpClient>,
    worker: SerialWorker,
    pool: WorkerPool,
    downloads: HashMap<i64, Download>,
    next_download: i64,
    /// Set when the worker that asked is gone: one-shot downloads stop.
    stale: Arc<AtomicBool>,
    opener: fn(&Path) -> Result<(), String>,
}

impl Files {
    /// `data_dir` is the application's data directory (the sandbox is its
    /// `pam-files` subdirectory), `assets_dir` the directory `copyAsset`
    /// reads bundled assets from, and `client` performs the downloads.
    #[must_use]
    pub fn new(data_dir: &Path, assets_dir: &Path, client: Arc<dyn HttpClient>) -> Self {
        Self {
            sandbox: Arc::new(Sandbox {
                root: sandbox_root(data_dir),
                assets: assets_dir.to_path_buf(),
            }),
            client,
            worker: SerialWorker::new("pam-files"),
            pool: WorkerPool::new("pam-files-download", DOWNLOAD_THREADS),
            downloads: HashMap::new(),
            next_download: 0,
            stale: Arc::new(AtomicBool::new(false)),
            opener: launch,
        }
    }

    /// The sandbox root this module serves.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.sandbox.root
    }

    fn open(&self, request: &WireMap, reply: Reply) {
        let path = match required_text(request, "path") {
            Ok(path) => path.to_owned(),
            Err(message) => return reply.failure(message),
        };
        let sandbox = Arc::clone(&self.sandbox);
        let opener = self.opener;
        // Launching waits briefly for the launcher's verdict: keep that off
        // the UI thread and off the file worker.
        let spawned = std::thread::Builder::new()
            .name("pam-files-open".into())
            .spawn(move || {
                let outcome = sandbox.resolve(&path).and_then(|file| {
                    if file.is_file() {
                        opener(&file)
                    } else {
                        Err(MISSING_FILE.into())
                    }
                });
                match outcome {
                    Ok(()) => reply.done(),
                    Err(message) => reply.failure(message),
                }
            });
        // The reply moved into the thread; a failed spawn drops it unanswered.
        let _ = spawned;
    }

    fn download(&self, request: &WireMap, reply: Reply) {
        let request = match download_request(request) {
            Ok(request) => request,
            Err(message) => return reply.failure(message),
        };
        let sandbox = Arc::clone(&self.sandbox);
        let client = Arc::clone(&self.client);
        let stale = Arc::clone(&self.stale);
        self.pool.run(move || {
            match download(client.as_ref(), &sandbox, &request, &stale, |_, _| {}) {
                Ok(reference) => reply.map(&reference.to_map()),
                Err(message) => reply.failure(message),
            }
        });
    }

    fn download_start(&mut self, request: &WireMap, reply: Reply) {
        let request = match download_request(request) {
            Ok(request) => request,
            Err(message) => return reply.failure(message),
        };
        self.next_download += 1;
        let id = self.next_download;
        let task = Download {
            channel: Arc::new(WatchChannel::new()),
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let channel = Arc::clone(&task.channel);
        let cancel = Arc::clone(&task.cancel);
        self.downloads.insert(id, task);
        let sandbox = Arc::clone(&self.sandbox);
        let client = Arc::clone(&self.client);
        self.pool.run(move || {
            // One event per whole percent; a single one when the length is unknown.
            let mut reported = None;
            let outcome = download(
                client.as_ref(),
                &sandbox,
                &request,
                &cancel,
                |written, total| {
                    let percent = written.saturating_mul(100).checked_div(total).unwrap_or(0);
                    if reported != Some(percent) {
                        reported = Some(percent);
                        channel.offer(progress_event(written, total));
                    }
                },
            );
            channel.finish(match outcome {
                Ok(reference) => complete_event(&reference),
                Err(message) => failed_event(&message),
            });
        });
        reply.map(&WireMap::new().with("subscription", id));
    }
}

impl Module for Files {
    fn call(&mut self, method: &str, payload: &[u8], reply: Reply) {
        let request = match request_map(payload) {
            Ok(request) => request,
            Err(error) => return reply.failure(error),
        };
        match method {
            "read" | "write" | "sha256" | "copyAsset" | "stat" | "exists" | "list" | "delete"
            | "move" | "copy" | "makeDirectory" => {
                let sandbox = Arc::clone(&self.sandbox);
                let method = method.to_owned();
                self.worker
                    .run(move || reply.result(sandbox.run(&method, &request)));
            }
            "open" => self.open(&request, reply),
            "download" => self.download(&request, reply),
            "downloadStart" => self.download_start(&request, reply),
            "downloadNext" => {
                let Some(id) = request.integer("subscription") else {
                    return reply.failure("File download subscription is required");
                };
                match self.downloads.get(&id) {
                    Some(task) => task.channel.next_reply(reply),
                    None => reply.failure("File download not found"),
                }
            }
            "downloadCancel" => {
                if let Some(task) = request
                    .integer("subscription")
                    .and_then(|id| self.downloads.remove(&id))
                {
                    task.cancel();
                }
                reply.done();
            }
            "share" => reply.failure("Sharing is not available on this platform"),
            "isPackageInstalled" => reply.map(
                &WireMap::new()
                    .with("supported", false)
                    .with("installed", false),
            ),
            "importUri" => reply.failure("Only content URIs can be imported"),
            "capture" => reply.failure("Camera is unavailable"),
            other => reply.failure(format!("Unknown files method {other}")),
        }
    }

    fn reset(&mut self) {
        for (_, task) in self.downloads.drain() {
            task.cancel();
        }
        self.stale.store(true, Ordering::Relaxed);
        self.stale = Arc::new(AtomicBool::new(false));
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        self.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::ModuleResult;
    use crate::modules::test_support::{PlainClient, Served, TestServer, temporary_directory};
    use crate::runtime::{MODULE_FAILURE, MODULE_SUCCESS};
    use std::sync::mpsc::Receiver;

    const WAIT: Duration = Duration::from_secs(10);

    struct Fixture {
        directory: PathBuf,
        files: Files,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let directory = temporary_directory(name);
            fs::create_dir_all(directory.join("assets/img")).unwrap();
            fs::write(directory.join("assets/img/logo.png"), b"PNG!").unwrap();
            let files = Files::new(
                &directory.join("data"),
                &directory.join("assets"),
                Arc::new(PlainClient),
            );
            Self { directory, files }
        }

        fn root(&self) -> PathBuf {
            sandbox_root(&self.directory.join("data"))
        }

        fn start(&mut self, method: &str, request: &WireMap) -> Receiver<ModuleResult> {
            let (reply, answer) = Reply::channel();
            self.files.call(method, &request.encode(), reply);
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

        fn write(&mut self, path: &str, contents: &[u8]) {
            let request = WireMap::new()
                .with("path", path)
                .with("data", BASE64.encode(contents));
            assert!(self.call("write", &request).unwrap().is_empty());
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    fn path(path: &str) -> WireMap {
        WireMap::new().with("path", path)
    }

    #[test]
    fn mime_types_follow_the_extension() {
        assert_eq!(mime_for_name("photo.JPG"), "image/jpeg");
        assert_eq!(mime_for_name("archive.tar.gz"), "application/gzip");
        assert_eq!(mime_for_name("notes.txt"), "text/plain");
        assert_eq!(mime_for_name("clip.mp4"), "video/mp4");
        assert_eq!(mime_for_name("unknown.xyz"), DEFAULT_MIME);
        assert_eq!(mime_for_name("no-extension"), DEFAULT_MIME);
        assert_eq!(mime_for_name(".hidden"), DEFAULT_MIME);
        assert_eq!(mime_for_path(Path::new("/tmp/a/song.mp3")), "audio/mpeg");
    }

    #[test]
    fn the_mime_table_is_sorted_for_binary_search() {
        assert!(MIME_TYPES.windows(2).all(|pair| pair[0].0 < pair[1].0));
        for (extension, mime) in MIME_TYPES {
            assert_eq!(mime_for_name(&format!("file.{extension}")), *mime);
        }
    }

    #[test]
    fn extensions_come_from_mime_patterns() {
        assert_eq!(extensions_for_mime("image/png"), ["png"]);
        assert!(extensions_for_mime("image/*").contains(&"webp"));
        assert!(!extensions_for_mime("image/*").contains(&"mp4"));
        assert!(extensions_for_mime("*/*").is_empty());
        assert!(extensions_for_mime("application/x-unknown").is_empty());
    }

    #[test]
    fn wire_paths_must_be_plain_relative_paths() {
        assert_eq!(segments("a/b.txt").unwrap(), ["a", "b.txt"]);
        assert_eq!(segments("").unwrap_err(), EMPTY_PATH);
        assert_eq!(segments("   ").unwrap_err(), EMPTY_PATH);
        for escaping in [
            "/etc/passwd",
            "../secret",
            "a/../../b",
            "a/./b",
            "a//b",
            "a/",
            "a\\b",
            "..",
        ] {
            assert_eq!(
                segments(escaping).unwrap_err(),
                ESCAPES_SANDBOX,
                "{escaping}"
            );
        }
        assert_eq!(segments("a\nb").unwrap_err(), INVALID_PATH);
        assert_eq!(segments("a\0b").unwrap_err(), INVALID_PATH);
        assert_eq!(segments(&"a".repeat(4097)).unwrap_err(), INVALID_PATH);
        assert!(segments(&"a".repeat(4096)).is_ok());
    }

    #[test]
    fn resolution_stays_inside_the_sandbox() {
        let fixture = Fixture::new("resolve");
        let root = fixture.root();
        let resolved = resolve(&root, "docs/new/file.txt").unwrap();
        let canonical = fs::canonicalize(&root).unwrap();
        assert_eq!(resolved, canonical.join("docs/new/file.txt"));
        assert_eq!(
            relative(&canonical, &resolved).unwrap(),
            "docs/new/file.txt"
        );
        assert_eq!(resolve(&root, "../outside").unwrap_err(), ESCAPES_SANDBOX);
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_links_cannot_leave_the_sandbox() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new("symlinks");
        let root = fixture.root();
        fs::create_dir_all(root.join("inside")).unwrap();
        let outside = fixture.directory.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), b"secret").unwrap();
        symlink(&outside, root.join("escape")).unwrap();
        symlink(outside.join("secret.txt"), root.join("leak.txt")).unwrap();
        symlink(outside.join("missing"), root.join("dangling")).unwrap();
        symlink(root.join("inside"), root.join("alias")).unwrap();
        symlink(&root, root.join("self")).unwrap();

        assert_eq!(
            resolve(&root, "escape/secret.txt").unwrap_err(),
            ESCAPES_SANDBOX
        );
        assert_eq!(
            resolve(&root, "escape/new.txt").unwrap_err(),
            ESCAPES_SANDBOX
        );
        assert_eq!(resolve(&root, "leak.txt").unwrap_err(), ESCAPES_SANDBOX);
        // A dangling link would let a write create its target outside.
        assert_eq!(resolve(&root, "dangling").unwrap_err(), ESCAPES_SANDBOX);
        assert_eq!(resolve(&root, "self").unwrap_err(), ESCAPES_SANDBOX);
        // Links that stay inside resolve to their target.
        let canonical = fs::canonicalize(&root).unwrap();
        assert_eq!(
            resolve(&root, "alias/file.txt").unwrap(),
            canonical.join("inside/file.txt")
        );
        assert_eq!(
            resolve(&root, "self/inside/file.txt").unwrap(),
            canonical.join("inside/file.txt")
        );
    }

    #[cfg(unix)]
    #[test]
    fn operations_refuse_paths_through_escaping_links() {
        use std::os::unix::fs::symlink;
        let mut fixture = Fixture::new("symlink-operations");
        let root = fixture.root();
        fs::create_dir_all(&root).unwrap();
        let outside = fixture.directory.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), b"secret").unwrap();
        symlink(&outside, root.join("escape")).unwrap();

        assert_eq!(
            fixture.fails("read", &path("escape/secret.txt")),
            ESCAPES_SANDBOX
        );
        let write = WireMap::new()
            .with("path", "escape/planted.txt")
            .with("data", BASE64.encode(b"x"));
        assert_eq!(fixture.fails("write", &write), ESCAPES_SANDBOX);
        assert!(!outside.join("planted.txt").exists());
        assert_eq!(
            fixture.fails("delete", &path("escape/secret.txt")),
            ESCAPES_SANDBOX
        );
        assert!(outside.join("secret.txt").exists());
    }

    #[test]
    fn renderer_uris_resolve_inside_the_sandbox() {
        let fixture = Fixture::new("uri");
        let root = fixture.root();
        let canonical = canonical_root(&root).unwrap();
        assert_eq!(
            resolve_uri(&root, "pam-file:///imports/My%20photo%20%231.png"),
            Some(canonical.join("imports/My photo #1.png"))
        );
        assert_eq!(
            resolve_uri(&root, "PAM-FILE:///a.txt"),
            Some(canonical.join("a.txt"))
        );
        for refused in [
            "pam-file://host/a.txt",
            "pam-file://a.txt",
            "pam-file:///../a.txt",
            "pam-file:///a/%2e%2e/b.txt",
            "pam-file:///a%2Fb.txt",
            "pam-file:///a%5Cb.txt",
            "pam-file:///a.txt?x=1",
            "pam-file:///a.txt#top",
            "pam-file:///bad%zz",
            "pam-file:///trunc%2",
            "pam-file:///",
            "pam-file:////etc/passwd",
            "file:///etc/passwd",
            "https://example.com/a.txt",
            "",
        ] {
            assert_eq!(resolve_uri(&root, refused), None, "{refused}");
        }
    }

    #[test]
    fn written_files_read_back_with_their_metadata() {
        let mut fixture = Fixture::new("read-write");
        fixture.write("notes/today.txt", b"hello \xff world");
        let read = fixture.call("read", &path("notes/today.txt")).unwrap();
        assert_eq!(
            BASE64.decode(read.text("data").unwrap()).unwrap(),
            b"hello \xff world"
        );
        // Overwriting replaces the contents and leaves no staging file.
        fixture.write("notes/today.txt", b"second");
        let stat = fixture.call("stat", &path("notes/today.txt")).unwrap();
        assert_eq!(stat.text("path"), Some("notes/today.txt"));
        assert_eq!(stat.text("name"), Some("today.txt"));
        assert_eq!(stat.text("mimeType"), Some("text/plain"));
        assert_eq!(stat.integer("size"), Some(6));
        assert_eq!(
            fs::read_dir(fixture.root().join("notes")).unwrap().count(),
            1
        );

        let digest = fixture.call("sha256", &path("notes/today.txt")).unwrap();
        assert_eq!(
            digest.text("sha256"),
            Some(super::super::support::sha256_hex(b"second").as_str())
        );
    }

    #[test]
    fn missing_files_and_invalid_requests_fail_with_the_sdk_messages() {
        let mut fixture = Fixture::new("failures");
        assert_eq!(fixture.fails("stat", &path("nope.txt")), MISSING_FILE);
        assert_eq!(fixture.fails("read", &path("nope.txt")), MISSING_FILE);
        assert_eq!(fixture.fails("sha256", &path("nope.txt")), MISSING_FILE);
        assert_eq!(fixture.fails("delete", &path("nope.txt")), MISSING_FILE);
        assert_eq!(fixture.fails("stat", &path("")), EMPTY_PATH);
        assert_eq!(fixture.fails("stat", &path("../x")), ESCAPES_SANDBOX);
        assert_eq!(
            fixture.fails("stat", &WireMap::new()),
            "Missing text field path"
        );
        assert_eq!(
            fixture.fails("write", &path("a.txt")),
            "Missing text field data"
        );
        assert_eq!(
            fixture.fails("list", &path("missing-directory")),
            "Directory does not exist"
        );
        assert_eq!(
            fixture.fails("frobnicate", &WireMap::new()),
            "Unknown files method frobnicate"
        );
        // A directory is not a file.
        fixture.call("makeDirectory", &path("folder")).unwrap();
        assert_eq!(fixture.fails("stat", &path("folder")), MISSING_FILE);
        assert_eq!(fixture.fails("delete", &path("folder")), MISSING_FILE);
    }

    #[test]
    fn the_bridge_limit_applies_to_reads_and_writes() {
        let mut fixture = Fixture::new("limits");
        let sandbox = Arc::clone(&fixture.files.sandbox);
        let limit = usize::try_from(MAX_BRIDGE_BYTES).unwrap();
        // Exactly one mebibyte is accepted, one byte more is not decoded at all.
        assert_eq!(BASE64.encode(vec![0_u8; limit]).len(), MAX_BRIDGE_BASE64);
        sandbox
            .write("exact.bin", &BASE64.encode(vec![7_u8; limit]))
            .unwrap();
        assert_eq!(
            fs::metadata(fixture.root().join("exact.bin"))
                .unwrap()
                .len(),
            MAX_BRIDGE_BYTES
        );
        assert_eq!(
            sandbox
                .write("large.bin", &BASE64.encode(vec![7_u8; limit + 1]))
                .unwrap_err(),
            BRIDGE_LIMIT
        );
        assert!(!fixture.root().join("large.bin").exists());
        let invalid = WireMap::new()
            .with("path", "x.bin")
            .with("data", "not base64!");
        assert_eq!(fixture.fails("write", &invalid), "Invalid file payload");

        // A read answers at most what one result can carry.
        let readable = usize::try_from(MAX_READ_BYTES).unwrap();
        fs::write(fixture.root().join("fits.bin"), vec![1_u8; readable]).unwrap();
        let (status, payload) = fixture
            .start("read", &path("fits.bin"))
            .recv_timeout(WAIT)
            .unwrap();
        assert_eq!(status, MODULE_SUCCESS);
        assert!(payload.len() <= crate::wiremap::MAX_WIRE_MAP_BYTES);
        assert!(payload.len() > crate::wiremap::MAX_WIRE_MAP_BYTES - 4);
        fs::write(fixture.root().join("grown.bin"), vec![1_u8; readable + 1]).unwrap();
        assert_eq!(fixture.fails("read", &path("grown.bin")), BRIDGE_LIMIT);
        assert_eq!(fixture.fails("read", &path("exact.bin")), BRIDGE_LIMIT);
    }

    #[test]
    fn existence_and_listing_report_files_only() {
        let mut fixture = Fixture::new("listing");
        fixture.write("docs/b.txt", b"bb");
        fixture.write("docs/A.png", b"a");
        fixture.write("docs/nested/c.txt", b"c");
        fixture.write("top.json", b"{}");

        let file = fixture.call("exists", &path("docs/b.txt")).unwrap();
        assert_eq!(
            (file.boolean("exists"), file.boolean("directory")),
            (Some(true), Some(false))
        );
        let directory = fixture.call("exists", &path("docs")).unwrap();
        assert_eq!(
            (directory.boolean("exists"), directory.boolean("directory")),
            (Some(false), Some(true))
        );
        let missing = fixture.call("exists", &path("nope")).unwrap();
        assert_eq!(
            (missing.boolean("exists"), missing.boolean("directory")),
            (Some(false), Some(false))
        );

        let listing = fixture.call("list", &path("docs")).unwrap();
        let items: serde_json::Value =
            serde_json::from_str(listing.text("items").unwrap()).unwrap();
        assert_eq!(
            items,
            serde_json::json!([
                {"path": "docs/A.png", "name": "A.png", "mimeType": "image/png", "size": 1},
                {"path": "docs/b.txt", "name": "b.txt", "mimeType": "text/plain", "size": 2},
            ])
        );
        let root = fixture.call("list", &path("")).unwrap();
        let items: serde_json::Value = serde_json::from_str(root.text("items").unwrap()).unwrap();
        assert_eq!(items.as_array().unwrap().len(), 1);
        assert_eq!(items[0]["path"], "top.json");
    }

    #[test]
    fn deleting_removes_files() {
        let mut fixture = Fixture::new("delete");
        fixture.write("gone.txt", b"x");
        assert!(
            fixture
                .call("delete", &path("gone.txt"))
                .unwrap()
                .is_empty()
        );
        assert!(!fixture.root().join("gone.txt").exists());
    }

    fn transfer(from: &str, to: &str, overwrite: bool) -> WireMap {
        WireMap::new()
            .with("from", from)
            .with("to", to)
            .with("overwrite", overwrite)
    }

    #[test]
    fn moving_and_copying_follow_the_overwrite_rules() {
        let mut fixture = Fixture::new("transfer");
        fixture.write("a.txt", b"alpha");
        fixture.write("b.txt", b"beta!");
        fixture.write("folder/inner.txt", b"i");

        let copied = fixture
            .call("copy", &transfer("a.txt", "copies/a-copy.txt", false))
            .unwrap();
        assert_eq!(copied.text("path"), Some("copies/a-copy.txt"));
        assert_eq!(copied.integer("size"), Some(5));
        assert_eq!(
            fs::read(fixture.root().join("copies/a-copy.txt")).unwrap(),
            b"alpha"
        );
        assert_eq!(
            fs::read_dir(fixture.root().join("copies")).unwrap().count(),
            1
        );

        assert_eq!(
            fixture.fails("copy", &transfer("a.txt", "b.txt", false)),
            "Destination already exists"
        );
        fixture
            .call("copy", &transfer("a.txt", "b.txt", true))
            .unwrap();
        assert_eq!(fs::read(fixture.root().join("b.txt")).unwrap(), b"alpha");

        assert_eq!(
            fixture.fails("copy", &transfer("missing.txt", "x.txt", false)),
            "Source does not exist"
        );
        assert_eq!(
            fixture.fails("copy", &transfer("a.txt", "a.txt", true)),
            "Source and destination are the same"
        );
        assert_eq!(
            fixture.fails("copy", &transfer("folder", "folder-copy", false)),
            "Only files can be copied"
        );
        assert_eq!(
            fixture.fails("move", &transfer("folder", "folder/inside", false)),
            "Cannot move a directory inside itself"
        );
        assert_eq!(
            fixture.fails("move", &transfer("a.txt", "folder", true)),
            "Only files can replace an existing destination"
        );
        assert_eq!(
            fixture.fails("move", &transfer("a.txt", "../a.txt", false)),
            ESCAPES_SANDBOX
        );
        assert_eq!(
            fixture.fails("move", &WireMap::new().with("from", "a.txt")),
            "Missing text field to"
        );

        let moved = fixture
            .call("move", &transfer("a.txt", "moved/deep/a.txt", false))
            .unwrap();
        assert_eq!(moved.text("path"), Some("moved/deep/a.txt"));
        assert!(!fixture.root().join("a.txt").exists());
        // Directories move too.
        let moved = fixture
            .call("move", &transfer("folder", "renamed", false))
            .unwrap();
        assert_eq!(moved.text("path"), Some("renamed"));
        assert_eq!(moved.integer("size"), Some(0));
        assert!(fixture.root().join("renamed/inner.txt").is_file());
    }

    #[test]
    fn directories_are_created_with_their_parents() {
        let mut fixture = Fixture::new("directories");
        let made = fixture.call("makeDirectory", &path("a/b/c")).unwrap();
        assert_eq!(made.text("path"), Some("a/b/c"));
        assert!(fixture.root().join("a/b/c").is_dir());
        // Creating it again succeeds.
        fixture.call("makeDirectory", &path("a/b/c")).unwrap();
        fixture.write("a/file.txt", b"x");
        assert_eq!(
            fixture.fails("makeDirectory", &path("a/file.txt")),
            "A file already exists at this path"
        );
    }

    #[test]
    fn bundled_assets_are_copied_into_the_sandbox() {
        let mut fixture = Fixture::new("assets");
        let request = WireMap::new()
            .with("assetPath", "img\\logo.png")
            .with("path", "branding/logo.png");
        let copied = fixture.call("copyAsset", &request).unwrap();
        assert_eq!(copied.text("path"), Some("branding/logo.png"));
        assert_eq!(copied.text("mimeType"), Some("image/png"));
        assert_eq!(copied.integer("size"), Some(4));
        let asset = |asset: &str| {
            WireMap::new()
                .with("assetPath", asset)
                .with("path", "out.bin")
        };
        assert_eq!(
            fixture.fails("copyAsset", &asset("")),
            "Invalid bundled asset path"
        );
        assert_eq!(
            fixture.fails("copyAsset", &asset("/etc/passwd")),
            "Invalid bundled asset path"
        );
        assert_eq!(
            fixture.fails("copyAsset", &asset(&"a".repeat(1025))),
            "Invalid bundled asset path"
        );
        assert_eq!(
            fixture.fails("copyAsset", &asset("../data/pam-files/branding/logo.png")),
            "Bundled asset path escapes application bundle"
        );
        assert_eq!(
            fixture.fails("copyAsset", &asset("img/missing.png")),
            "Bundled asset does not exist"
        );
        assert_eq!(
            fixture.fails("copyAsset", &asset("img")),
            "Bundled asset does not exist"
        );
        assert!(!fixture.root().join("out.bin").exists());
    }

    #[cfg(unix)]
    #[test]
    fn bundled_assets_cannot_be_reached_through_escaping_links() {
        use std::os::unix::fs::symlink;
        let mut fixture = Fixture::new("asset-links");
        fs::write(fixture.directory.join("private.key"), b"key").unwrap();
        symlink(
            fixture.directory.join("private.key"),
            fixture.directory.join("assets/link.key"),
        )
        .unwrap();
        let request = WireMap::new()
            .with("assetPath", "link.key")
            .with("path", "stolen.key");
        assert_eq!(
            fixture.fails("copyAsset", &request),
            "Bundled asset path escapes application bundle"
        );
    }

    #[test]
    fn platform_only_methods_answer_their_desktop_results() {
        let mut fixture = Fixture::new("platform");
        assert_eq!(
            fixture.fails("share", &WireMap::new().with("paths", "[\"a.txt\"]")),
            "Sharing is not available on this platform"
        );
        let installed = fixture
            .call(
                "isPackageInstalled",
                &WireMap::new().with("packageName", "com.whatsapp"),
            )
            .unwrap();
        assert_eq!(
            (
                installed.boolean("supported"),
                installed.boolean("installed")
            ),
            (Some(false), Some(false))
        );
        assert_eq!(
            fixture.fails(
                "importUri",
                &WireMap::new().with("uri", "content://media/1")
            ),
            "Only content URIs can be imported"
        );
        assert_eq!(
            fixture.fails("capture", &WireMap::new().with("type", 1)),
            "Camera is unavailable"
        );
        // The dialog-backed methods are the runtime's.
        assert_eq!(
            fixture.fails("pick", &WireMap::new()),
            "Unknown files method pick"
        );
    }

    #[test]
    fn opening_launches_the_default_handler_for_existing_files() {
        fn opened(file: &Path) -> Result<(), String> {
            assert!(file.is_absolute() && file.ends_with("docs/report.pdf"));
            Ok(())
        }
        fn refused(_: &Path) -> Result<(), String> {
            Err(NO_APPLICATION.into())
        }
        let mut fixture = Fixture::new("open");
        fixture.write("docs/report.pdf", b"%PDF");
        let request = path("docs/report.pdf").with("mimeType", "");
        fixture.files.opener = opened;
        assert!(fixture.call("open", &request).unwrap().is_empty());
        assert_eq!(
            fixture.fails("open", &path("docs/missing.pdf")),
            MISSING_FILE
        );
        assert_eq!(
            fixture.fails("open", &path("../report.pdf")),
            ESCAPES_SANDBOX
        );
        fixture.files.opener = refused;
        assert_eq!(fixture.fails("open", &request), NO_APPLICATION);
    }

    #[test]
    fn the_opener_passes_the_path_as_one_argument() {
        let command = opener_command(Path::new("/data/pam-files/a b; rm -rf.txt"));
        let arguments: Vec<_> = command.get_args().collect();
        assert_eq!(
            arguments.last().unwrap().to_string_lossy(),
            "/data/pam-files/a b; rm -rf.txt"
        );
        if cfg!(target_os = "linux") {
            assert_eq!(command.get_program(), "xdg-open");
            assert_eq!(arguments.len(), 1);
        }
    }

    #[test]
    fn picker_requests_are_validated() {
        let request = pick_request(&WireMap::new()).unwrap();
        assert_eq!(
            request,
            PickRequest {
                kind: PickKind::Any,
                mime_type: None,
                maximum_bytes: 64 * MEBIBYTE,
                limit: 10,
            }
        );
        assert!(request.extensions().is_empty());
        let request = pick_request(
            &WireMap::new()
                .with("type", 5)
                .with("limit", 500)
                .with("maximumBytes", 1024),
        )
        .unwrap();
        assert_eq!(
            (request.kind, request.limit, request.maximum_bytes),
            (PickKind::Media, 50, 1024)
        );
        assert!(request.extensions().contains(&"png") && request.extensions().contains(&"mp4"));
        assert!(!request.extensions().contains(&"mp3"));
        assert_eq!(request.selection_bytes(), 256 * MEBIBYTE);
        let request = pick_request(
            &WireMap::new()
                .with("type", 1)
                .with("mimeType", "application/pdf")
                .with("limit", 0),
        )
        .unwrap();
        assert_eq!(request.extensions(), ["pdf"]);
        assert_eq!(request.limit, 1);
        let largest =
            pick_request(&WireMap::new().with("maximumBytes", 8_589_934_592_i64)).unwrap();
        assert_eq!(largest.selection_bytes(), 8 * 1024 * MEBIBYTE);

        for bytes in [0_i64, -1, 8_589_934_593] {
            assert_eq!(
                pick_request(&WireMap::new().with("maximumBytes", bytes)).unwrap_err(),
                "Picker import limit must be between 1 byte and 8 GiB"
            );
        }
        for mime in [
            "image",
            "image/",
            "/png",
            "image/p ng",
            "im*ge/png",
            "a/b/c",
        ] {
            assert_eq!(
                pick_request(&WireMap::new().with("mimeType", mime)).unwrap_err(),
                "Picker MIME type is invalid",
                "{mime}"
            );
        }
        assert!(pick_request(&WireMap::new().with("mimeType", "image/*")).is_ok());
    }

    #[test]
    fn picked_files_are_copied_into_imports() {
        let fixture = Fixture::new("import");
        let root = fixture.root();
        let source = fixture.directory.join("Relatório final (1).pdf");
        fs::write(&source, b"%PDF-1.7").unwrap();
        let reference = import_picked(&root, &source, 1024).unwrap();
        assert_eq!(reference.name, "Relatório final (1).pdf");
        assert_eq!(reference.mime_type, "application/pdf");
        assert_eq!(reference.size, 8);
        let (directory, file) = reference.path.split_once('/').unwrap();
        assert_eq!(directory, IMPORTS_DIRECTORY);
        // `<uuid>-<name with unsafe characters replaced>`.
        assert_eq!(&file[36..], "-Relat_rio_final__1_.pdf");
        assert_eq!(
            fs::read(resolve(&root, &reference.path).unwrap()).unwrap(),
            b"%PDF-1.7"
        );
        // The source stays where it was.
        assert!(source.is_file());

        assert_eq!(
            import_picked(&root, &source, 7).unwrap_err(),
            "Selected file exceeds 0 MiB"
        );
        assert_eq!(
            import_picked(&root, &fixture.directory.join("missing.bin"), 10).unwrap_err(),
            UNREADABLE_SELECTION
        );
        assert_eq!(
            import_picked(&root, &fixture.directory, 10).unwrap_err(),
            UNREADABLE_SELECTION
        );
        assert_eq!(
            fs::read_dir(root.join(IMPORTS_DIRECTORY)).unwrap().count(),
            1
        );

        let map = reference.to_map();
        assert_eq!(map.text("name"), Some("Relatório final (1).pdf"));
        assert_eq!(map.integer("size"), Some(8));
    }

    #[test]
    fn selections_import_in_order_and_roll_back_on_failure() {
        let fixture = Fixture::new("selection");
        let root = fixture.root();
        let sources: Vec<PathBuf> = ["one.txt", "two.txt", "three.txt"]
            .iter()
            .map(|name| {
                let source = fixture.directory.join(name);
                fs::write(&source, name.as_bytes()).unwrap();
                source
            })
            .collect();
        let request = PickRequest {
            kind: PickKind::Any,
            mime_type: None,
            maximum_bytes: 64,
            limit: 2,
        };
        let imported = import_selection(&root, &sources, &request).unwrap();
        let names: Vec<&str> = imported
            .iter()
            .map(|reference| reference.name.as_str())
            .collect();
        assert_eq!(names, ["one.txt", "two.txt"]);
        let items: serde_json::Value =
            serde_json::from_str(items_map(&imported).text("items").unwrap()).unwrap();
        assert_eq!(items[1]["name"], "two.txt");
        assert_eq!(items[1]["size"], 7);
        assert_eq!(items_map(&[]).text("items"), Some("[]"));

        assert_eq!(
            import_selection(&root, &[], &request).unwrap_err(),
            "No file was selected"
        );
        // A failure in the middle removes what was already imported.
        let before = fs::read_dir(root.join(IMPORTS_DIRECTORY)).unwrap().count();
        let broken = [sources[0].clone(), fixture.directory.join("missing.txt")];
        assert_eq!(
            import_selection(&root, &broken, &request).unwrap_err(),
            UNREADABLE_SELECTION
        );
        assert_eq!(
            fs::read_dir(root.join(IMPORTS_DIRECTORY)).unwrap().count(),
            before
        );
        let strict = PickRequest {
            maximum_bytes: 6,
            ..request
        };
        assert_eq!(
            import_selection(&root, &sources, &strict).unwrap_err(),
            "Selected file exceeds 0 MiB"
        );
        assert_eq!(
            fs::read_dir(root.join(IMPORTS_DIRECTORY)).unwrap().count(),
            before
        );
    }

    fn download_map(url: &str, path: &str) -> WireMap {
        WireMap::new()
            .with("url", url)
            .with("path", path)
            .with("headers", "{}")
    }

    #[test]
    fn download_requests_are_validated() {
        let valid = download_request(
            &download_map("https://cdn.example.com/a.zip", "downloads/a.zip")
                .with("headers", r#"{"Authorization":"Bearer t","X-Trace":"1"}"#),
        )
        .unwrap();
        assert_eq!(valid.maximum_bytes, 64 * MEBIBYTE);
        assert_eq!(
            valid.headers,
            [
                (String::from("Authorization"), String::from("Bearer t")),
                (String::from("X-Trace"), String::from("1")),
            ]
        );
        for url in [
            "http://cdn.example.com/a.zip",
            "ftp://cdn.example.com/a.zip",
            "cdn.example.com/a.zip",
            "https:///a.zip",
            "https://exa mple.com/a.zip",
            "",
        ] {
            assert_eq!(
                download_request(&download_map(url, "a.zip")).unwrap_err(),
                "Download URL must be an absolute HTTPS URL",
                "{url}"
            );
        }
        let long = format!("https://example.com/{}", "a".repeat(2048));
        assert_eq!(
            download_request(&download_map(&long, "a.zip")).unwrap_err(),
            "Download URL must be an absolute HTTPS URL"
        );
        assert_eq!(
            download_request(&download_map(
                "https://user:pass@example.com/a.zip",
                "a.zip"
            ))
            .unwrap_err(),
            "Download URL cannot contain credentials"
        );
        for bytes in [0_i64, -5, 268_435_457] {
            assert_eq!(
                download_request(
                    &download_map("https://example.com/a", "a").with("maximumBytes", bytes)
                )
                .unwrap_err(),
                "Invalid download size limit"
            );
        }
        assert!(
            download_request(
                &download_map("https://example.com/a", "a").with("maximumBytes", 268_435_456_i64)
            )
            .is_ok()
        );
        assert_eq!(
            download_request(&download_map("https://example.com/a", "../a")).unwrap_err(),
            ESCAPES_SANDBOX
        );
        assert_eq!(
            download_request(&WireMap::new().with("path", "a")).unwrap_err(),
            "Missing text field url"
        );
    }

    #[test]
    fn download_headers_are_checked_for_injection() {
        const UNSAFE: &str = "Download request headers are invalid or unsafe";
        assert_eq!(download_headers("").unwrap(), []);
        assert_eq!(download_headers("{}").unwrap(), []);
        assert_eq!(download_headers("[]").unwrap(), []);
        for unsafe_headers in [
            r#"{"X-A":"line\r\nInjected: 1"}"#,
            r#"{"X-A":"line\nbreak"}"#,
            r#"{"X-A":"nul\u0000"}"#,
            r#"{"Bad Name":"v"}"#,
            r#"{"Bad:Name":"v"}"#,
            r#"{"":"v"}"#,
            r#"{"X-A":5}"#,
            r#"["X-A"]"#,
            "not json",
        ] {
            assert_eq!(
                download_headers(unsafe_headers).unwrap_err(),
                UNSAFE,
                "{unsafe_headers}"
            );
        }
        let long_name = format!(r#"{{"{}":"v"}}"#, "n".repeat(65));
        assert_eq!(download_headers(&long_name).unwrap_err(), UNSAFE);
        let long_value = format!(r#"{{"X-A":"{}"}}"#, "v".repeat(8193));
        assert_eq!(download_headers(&long_value).unwrap_err(), UNSAFE);
        let longest_value = format!(r#"{{"X-A":"{}"}}"#, "v".repeat(8192));
        assert!(download_headers(&longest_value).is_ok());
        for blocked in ["Host", "content-length", "Transfer-Encoding", "CONNECTION"] {
            assert_eq!(
                download_headers(&format!(r#"{{"{blocked}":"x"}}"#)).unwrap_err(),
                "Download request header is not allowed"
            );
        }
        let many: serde_json::Map<String, serde_json::Value> = (0..33)
            .map(|index| (format!("X-H{index}"), serde_json::Value::from("v")))
            .collect();
        assert_eq!(
            download_headers(&serde_json::Value::Object(many).to_string()).unwrap_err(),
            "Downloads accept at most 32 request headers"
        );
    }

    #[test]
    fn response_media_types_drop_their_parameters() {
        assert_eq!(
            response_mime(Some("Text/HTML; charset=utf-8")).as_deref(),
            Some("text/html")
        );
        assert_eq!(
            response_mime(Some("  image/png  ")).as_deref(),
            Some("image/png")
        );
        assert_eq!(response_mime(Some("")), None);
        assert_eq!(response_mime(Some("nonsense")), None);
        assert_eq!(response_mime(Some("a/b c")), None);
        assert_eq!(response_mime(None), None);
    }

    /// The module only accepts HTTPS; the test client speaks plain HTTP to
    /// the loopback server whatever the scheme says.
    fn url(server: &TestServer, path: &str) -> String {
        format!("https://127.0.0.1:{}{path}", server.port())
    }

    #[test]
    fn downloads_are_written_atomically_with_the_response_type() {
        let server = TestServer::start(|request| match request.path.as_str() {
            "/report" => {
                Served::ok(b"%PDF-1.7 body".to_vec()).header("Content-Type", "application/pdf; q=1")
            }
            "/plain" => Served::ok(b"text".to_vec()),
            "/missing" => Served::status(404, b"nope".to_vec()),
            "/moved" => Served::status(302, Vec::new()).header("Location", "/report"),
            _ => Served::ok(vec![b'x'; 5000]),
        });
        let mut fixture = Fixture::new("download");
        let request = download_map(&url(&server, "/report"), "downloads/report.bin")
            .with("headers", r#"{"Authorization":"Bearer secret"}"#);
        let reference = fixture.call("download", &request).unwrap();
        assert_eq!(reference.text("path"), Some("downloads/report.bin"));
        assert_eq!(reference.text("name"), Some("report.bin"));
        assert_eq!(reference.text("mimeType"), Some("application/pdf"));
        assert_eq!(reference.integer("size"), Some(13));
        assert_eq!(
            fs::read(fixture.root().join("downloads/report.bin")).unwrap(),
            b"%PDF-1.7 body"
        );
        let sent = server.requests();
        assert_eq!(sent[0].method, "GET");
        assert_eq!(sent[0].header("accept-encoding"), Some("identity"));
        assert_eq!(sent[0].header("authorization"), Some("Bearer secret"));

        // Without a response type the extension decides.
        let reference = fixture
            .call(
                "download",
                &download_map(&url(&server, "/plain"), "downloads/notes.txt"),
            )
            .unwrap();
        assert_eq!(reference.text("mimeType"), Some("text/plain"));

        assert_eq!(
            fixture.fails(
                "download",
                &download_map(&url(&server, "/missing"), "downloads/x.bin")
            ),
            "Download failed with HTTP 404"
        );
        // Redirects are not followed.
        assert_eq!(
            fixture.fails(
                "download",
                &download_map(&url(&server, "/moved"), "downloads/x.bin")
            ),
            "Download failed with HTTP 302"
        );
        assert_eq!(
            fixture.fails(
                "download",
                &download_map(&url(&server, "/big"), "downloads/x.bin").with("maximumBytes", 4999)
            ),
            DOWNLOAD_LIMIT
        );
        assert!(!fixture.root().join("downloads/x.bin").exists());
        // Failed downloads leave no temporary file behind.
        assert_eq!(
            fs::read_dir(fixture.root().join("downloads"))
                .unwrap()
                .count(),
            2
        );
        assert_eq!(
            fixture.fails("download", &download_map("http://127.0.0.1:1/x", "x.bin")),
            "Download URL must be an absolute HTTPS URL"
        );
    }

    #[test]
    fn undeclared_lengths_are_limited_while_streaming() {
        let server = TestServer::start(|_| Served::ok(vec![b'x'; 70_000]).without_length());
        let mut fixture = Fixture::new("download-stream-limit");
        assert_eq!(
            fixture.fails(
                "download",
                &download_map(&url(&server, "/big"), "big.bin").with("maximumBytes", 69_999)
            ),
            DOWNLOAD_LIMIT
        );
        assert_eq!(fs::read_dir(fixture.root()).unwrap().count(), 0);
        let reference = fixture
            .call(
                "download",
                &download_map(&url(&server, "/big"), "big.bin").with("maximumBytes", 70_000),
            )
            .unwrap();
        assert_eq!(reference.integer("size"), Some(70_000));
    }

    #[test]
    fn a_download_replaces_the_previous_file_only_when_it_completes() {
        let server = TestServer::start(|request| match request.path.as_str() {
            "/good" => Served::ok(b"new contents".to_vec()),
            _ => Served::status(500, Vec::new()),
        });
        let mut fixture = Fixture::new("download-replace");
        fixture.write("file.txt", b"old");
        fixture.fails("download", &download_map(&url(&server, "/bad"), "file.txt"));
        assert_eq!(fs::read(fixture.root().join("file.txt")).unwrap(), b"old");
        fixture
            .call(
                "download",
                &download_map(&url(&server, "/good"), "file.txt"),
            )
            .unwrap();
        assert_eq!(
            fs::read(fixture.root().join("file.txt")).unwrap(),
            b"new contents"
        );
    }

    fn next(fixture: &mut Fixture, subscription: i64) -> Result<WireMap, String> {
        fixture.call(
            "downloadNext",
            &WireMap::new().with("subscription", subscription),
        )
    }

    #[test]
    fn streamed_downloads_report_progress_and_complete() {
        let body: Vec<u8> = (0..200_000_u32).map(|index| (index % 251) as u8).collect();
        let expected = body.clone();
        let server = TestServer::start(move |_| {
            Served::ok(body.clone()).header("Content-Type", "video/mp4")
        });
        let mut fixture = Fixture::new("download-streamed");
        let started = fixture
            .call(
                "downloadStart",
                &download_map(&url(&server, "/clip"), "media/clip.bin"),
            )
            .unwrap();
        let subscription = started.integer("subscription").unwrap();
        assert!(subscription >= 1);

        let mut last_written = 0;
        let complete = loop {
            let event = next(&mut fixture, subscription).unwrap();
            match event.integer("state") {
                Some(DOWNLOAD_PROGRESS) => {
                    let written = event.integer("bytesWritten").unwrap();
                    assert!(written >= last_written);
                    assert_eq!(event.integer("totalBytes"), Some(200_000));
                    last_written = written;
                }
                Some(DOWNLOAD_COMPLETE) => break event,
                other => panic!("unexpected state {other:?}: {:?}", event.text("message")),
            }
        };
        assert_eq!(complete.text("path"), Some("media/clip.bin"));
        assert_eq!(complete.text("name"), Some("clip.bin"));
        assert_eq!(complete.text("mimeType"), Some("video/mp4"));
        assert_eq!(complete.integer("size"), Some(200_000));
        assert_eq!(
            fs::read(fixture.root().join("media/clip.bin")).unwrap(),
            expected
        );
        // The terminal event closed the channel.
        assert_eq!(
            next(&mut fixture, subscription).unwrap_err(),
            "Observation is closed"
        );
        let cancel = WireMap::new().with("subscription", subscription);
        assert!(fixture.call("downloadCancel", &cancel).unwrap().is_empty());
        assert_eq!(
            next(&mut fixture, subscription).unwrap_err(),
            "File download not found"
        );
        // Cancelling is idempotent.
        assert!(fixture.call("downloadCancel", &cancel).unwrap().is_empty());
        assert_eq!(
            fixture.fails("downloadNext", &WireMap::new()),
            "File download subscription is required"
        );
    }

    #[test]
    fn streamed_downloads_report_failures_as_events() {
        let server = TestServer::start(|_| Served::status(503, Vec::new()));
        let mut fixture = Fixture::new("download-failed");
        let started = fixture
            .call("downloadStart", &download_map(&url(&server, "/x"), "x.bin"))
            .unwrap();
        let subscription = started.integer("subscription").unwrap();
        let event = next(&mut fixture, subscription).unwrap();
        assert_eq!(event.integer("state"), Some(DOWNLOAD_FAILED));
        assert_eq!(event.text("message"), Some("Download failed with HTTP 503"));
        // Invalid requests fail the start itself.
        assert_eq!(
            fixture.fails(
                "downloadStart",
                &download_map("http://example.com/x", "x.bin")
            ),
            "Download URL must be an absolute HTTPS URL"
        );
    }

    #[test]
    fn cancelling_a_streamed_download_stops_it_and_its_reader() {
        // The server trickles the body, so the download is still running when cancelled.
        let server = TestServer::start(|_| {
            Served::ok(vec![b'z'; 400_000]).trickle(4096, Duration::from_millis(15))
        });
        let mut fixture = Fixture::new("download-cancel");
        let started = fixture
            .call(
                "downloadStart",
                &download_map(&url(&server, "/slow"), "slow.bin"),
            )
            .unwrap();
        let subscription = started.integer("subscription").unwrap();
        let first = next(&mut fixture, subscription).unwrap();
        assert_eq!(first.integer("state"), Some(DOWNLOAD_PROGRESS));
        // Drain what is queued, then park a reader.
        let parked = loop {
            let answer = fixture.start(
                "downloadNext",
                &WireMap::new().with("subscription", subscription),
            );
            match answer.recv_timeout(Duration::from_millis(5)) {
                Ok(_) => {}
                Err(_) => break answer,
            }
        };
        let cancel = WireMap::new().with("subscription", subscription);
        assert!(fixture.call("downloadCancel", &cancel).unwrap().is_empty());
        // The reader may have been answered by a progress event just before the cancel.
        let (status, payload) = parked.recv_timeout(WAIT).unwrap();
        if status == MODULE_FAILURE {
            assert_eq!(payload, b"Observation stopped");
        }
        // The worker notices the flag, removes its temporary file and never publishes.
        let deadline = Instant::now() + WAIT;
        while fs::read_dir(fixture.root()).unwrap().count() > 0 {
            assert!(Instant::now() < deadline, "temporary file was not removed");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!fixture.root().join("slow.bin").exists());
    }

    #[test]
    fn a_restarted_worker_cancels_its_downloads() {
        let server = TestServer::start(|_| {
            Served::ok(vec![b'z'; 400_000]).trickle(4096, Duration::from_millis(15))
        });
        let mut fixture = Fixture::new("download-reset");
        let started = fixture
            .call(
                "downloadStart",
                &download_map(&url(&server, "/slow"), "slow.bin"),
            )
            .unwrap();
        let subscription = started.integer("subscription").unwrap();
        let one_shot = fixture.start(
            "download",
            &download_map(&url(&server, "/slow"), "other.bin"),
        );
        next(&mut fixture, subscription).unwrap();
        fixture.files.reset();
        assert_eq!(
            next(&mut fixture, subscription).unwrap_err(),
            "File download not found"
        );
        let (status, payload) = one_shot.recv_timeout(WAIT).unwrap();
        assert_eq!(
            (status, payload.as_slice()),
            (MODULE_FAILURE, DOWNLOAD_CANCELLED.as_bytes())
        );
        assert!(!fixture.root().join("other.bin").exists());
    }
}

/// Bounded content for a native save dialog; external paths cannot be supplied by callers.
/// Returns the default file name, the dialog title (given by PHP, already
/// localized; empty leaves the platform's own title) and the bytes.
pub(crate) fn save_request(map: &WireMap) -> Result<(String, String, Vec<u8>), String> {
    let title = map.text("title").unwrap_or_default();
    if title.len() > 512 || title.chars().any(char::is_control) {
        return Err("Invalid save dialog title".into());
    }
    let name = required_text(map, "defaultName")?;
    if name.is_empty()
        || name.len() > 255
        || name
            .chars()
            .any(|c| c.is_control() || c == '/' || c == '\\')
    {
        return Err("Invalid save filename".into());
    }
    let encoded = required_text(map, "data")?;
    if encoded.len() > MAX_BRIDGE_BASE64 {
        return Err(BRIDGE_LIMIT.into());
    }
    let bytes = BASE64
        .decode(encoded)
        .map_err(|_| String::from("Invalid file payload"))?;
    if bytes.len() as u64 > MAX_BRIDGE_BYTES {
        return Err(BRIDGE_LIMIT.into());
    }
    Ok((name.into(), title.into(), bytes))
}

pub(crate) fn save_selected(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() as u64 > MAX_BRIDGE_BYTES {
        return Err(BRIDGE_LIMIT.into());
    }
    // Do not follow a symlink introduced at the selected leaf. Atomic replacement preserves old data on failure.
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink() || !m.is_file()) {
        return Err("Selected destination is not a regular file".into());
    }
    let parent = path
        .parent()
        .ok_or_else(|| String::from("Invalid save destination"))?;
    let (staged, mut output) =
        Staged::create(staging_path(parent, path)).map_err(|e| io_message(&e))?;
    output
        .write_all(bytes)
        .and_then(|()| output.sync_all())
        .map_err(|e| io_message(&e))?;
    drop(output);
    staged.publish(path).map_err(|e| io_message(&e))
}

#[cfg(test)]
mod save_tests {
    use super::*;
    #[test]
    fn save_payload_is_bounded_and_filename_only() {
        let valid = WireMap::new()
            .with("defaultName", "extrato.csv")
            .with("data", BASE64.encode(b"a,b\n1,2"));
        let (_, title, bytes) = save_request(&valid).unwrap();
        assert_eq!(
            (title.as_str(), bytes.as_slice()),
            ("", b"a,b\n1,2".as_slice())
        );
        let titled = valid.clone().with("title", "Guardar archivo");
        assert_eq!(save_request(&titled).unwrap().1, "Guardar archivo");
        assert!(save_request(&valid.clone().with("title", "a\nb")).is_err());
        for name in ["", "../a.csv", "a/b", "a\\b"] {
            assert!(
                save_request(&WireMap::new().with("defaultName", name).with("data", "")).is_err()
            );
        }
        assert!(
            save_request(&WireMap::new().with("defaultName", "a.csv").with(
                "data",
                BASE64.encode(vec![0; MAX_BRIDGE_BYTES as usize + 1])
            ))
            .is_err()
        );
    }
    #[test]
    fn selected_save_replaces_atomically_and_preserves_on_failure() {
        let directory = std::env::temp_dir().join(format!("pam-save-test-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("statement.csv");
        fs::write(&path, b"old").unwrap();
        save_selected(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert!(save_selected(&path, &vec![0; MAX_BRIDGE_BYTES as usize + 1]).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert!(save_selected(&directory, b"no").is_err());
        fs::remove_dir_all(directory).unwrap();
    }
}
