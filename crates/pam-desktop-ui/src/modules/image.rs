//! `image`: the on-disk image cache and its `prefetch` method.
//!
//! # Cache layout
//!
//! The cache is one flat directory, `<cache dir>/pam-images-v1/`
//! ([`cache_root`]). Each image is a single file holding the bytes exactly as
//! the server sent them, named by the lower-case hexadecimal SHA-256 of its
//! *key* ([`cache_path`]). The key is the cache key the application gave the
//! image, or the image's URL when it gave none. There is no index and no
//! extension: the decoder recognises the format from the contents.
//!
//! A download is written to `<name>.tmp-<uuid>` in the same directory and
//! renamed over `<name>` once complete, so a file without the `.tmp-` suffix
//! is always a whole image and concurrent downloads of one key cannot
//! corrupt each other. The `cache` module clears the directory as a whole.
//!
//! The renderer's image loader and `prefetch` both go through
//! [`fetch_to_cache`], which is what makes a prefetched image a cache hit
//! later.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::fetch::{HttpClient, HttpRequest};
use super::http::parse_url;
use super::support::{CopyError, WorkerPool, copy_bounded, sha256_hex, uuid};
use super::system::IMAGE_CACHE_DIRECTORY;
use super::{Module, Reply, request_map};
use crate::wiremap::WireMap;

/// Largest image the cache accepts.
pub const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_URL_BYTES: usize = 8192;
const MAX_URLS: usize = 100;
const MAX_REQUEST_HEADERS: usize = 32;
const MAX_HEADER_VALUE_BYTES: usize = 4096;
const DEFAULT_ACCEPT: &str = "image/*,*/*;q=0.8";
/// Prefetch downloads running at once.
const PREFETCH_THREADS: usize = 2;

/// The image cache directory inside the application's cache directory.
#[must_use]
pub fn cache_root(cache_dir: &Path) -> PathBuf {
    cache_dir.join(IMAGE_CACHE_DIRECTORY)
}

/// Where the image with `key` lives inside the image cache directory
/// `cache_root`: a file named by the lower-case hexadecimal SHA-256 of the
/// key. The key is the image's explicit cache key, or its URL.
#[must_use]
pub fn cache_path(cache_root: &Path, key: &str) -> PathBuf {
    cache_root.join(sha256_hex(key.as_bytes()))
}

fn valid_header(name: &str, value: &str) -> bool {
    let token = (1..=64).contains(&name.len())
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
        });
    let transport = matches!(
        name.to_ascii_lowercase().as_str(),
        "host" | "content-length" | "transfer-encoding" | "connection"
    );
    token
        && !transport
        && value.len() <= MAX_HEADER_VALUE_BYTES
        && !value
            .chars()
            .any(|character| character.is_control() && character != '\t')
}

/// Decodes the packed request headers of an image (`Name:Value` lines
/// separated by `\n`, as the `ImageRequestHeaders` property carries them).
///
/// Lines that are not a safe header (a malformed name, a value over 4096
/// bytes or with control characters, a header that belongs to the
/// connection) are left out; at most 32 headers are returned.
#[must_use]
pub fn parse_request_headers(packed: &str) -> Vec<(String, String)> {
    packed
        .split('\n')
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name, value.trim_matches(' ')))
        .filter(|(name, value)| valid_header(name, value))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .take(MAX_REQUEST_HEADERS)
        .collect()
}

/// Removes the temporary file of a download that did not finish.
struct Partial(PathBuf);

impl Drop for Partial {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Downloads `url` into the cache unless `key` is already there; returns the
/// cached path and the bytes downloaded now (zero on a cache hit).
fn fetch(
    client: &dyn HttpClient,
    cache_root: &Path,
    url: &str,
    key: &str,
    headers: &[(String, String)],
    cancel: &AtomicBool,
) -> Result<(PathBuf, u64), String> {
    fetch_reporting(
        client,
        cache_root,
        url,
        key,
        headers,
        cancel,
        &mut |_, _| {},
    )
}

/// [`fetch`], reporting the bytes received so far and the declared total
/// (zero when the server sent none) after every chunk.
fn fetch_reporting(
    client: &dyn HttpClient,
    cache_root: &Path,
    url: &str,
    key: &str,
    headers: &[(String, String)],
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(PathBuf, u64), String> {
    let cancelled = || cancel.load(Ordering::Relaxed);
    let path = cache_path(cache_root, if key.is_empty() { url } else { key });
    if path.is_file() {
        return Ok((path, 0));
    }
    let remote = url.len() <= MAX_URL_BYTES
        && parse_url(url).is_some_and(|parsed| {
            matches!(parsed.scheme.as_str(), "https" | "http") && !parsed.credentials
        });
    if !remote {
        return Err("Image URL is invalid".into());
    }
    if headers.len() > MAX_REQUEST_HEADERS
        || headers
            .iter()
            .any(|(name, value)| !valid_header(name, value))
    {
        return Err("Image request headers are invalid".into());
    }
    let mut outgoing = HttpRequest {
        connect_timeout: Duration::from_secs(15),
        read_timeout: Duration::from_secs(30),
        // Image hosts commonly answer through a CDN redirect.
        follow_redirects: true,
        ..HttpRequest::get(url)
    };
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("accept"))
    {
        outgoing
            .headers
            .push((String::from("Accept"), String::from(DEFAULT_ACCEPT)));
    }
    outgoing.headers.extend(headers.iter().cloned());
    if cancelled() {
        return Err("Image request cancelled".into());
    }
    let mut response = client.send(outgoing, None, cancel).map_err(|error| {
        if error.is_empty() {
            String::from("Image request failed")
        } else {
            error
        }
    })?;
    if !(200..300).contains(&response.status) {
        return Err(format!(
            "Image request failed with HTTP {}",
            response.status
        ));
    }
    let declared = response.content_length;
    if declared.is_some_and(|length| length > MAX_IMAGE_BYTES) {
        return Err("Image exceeds the 64 MiB cache limit".into());
    }
    fs::create_dir_all(cache_root).map_err(|error| error.to_string())?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let partial = Partial(cache_root.join(format!("{name}.tmp-{}", uuid())));
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial.0)
        .map_err(|error| error.to_string())?;
    let written = copy_bounded(
        &mut response.body,
        &mut output,
        MAX_IMAGE_BYTES,
        cancel,
        |received| progress(received, declared.unwrap_or(0)),
    )
    .map_err(|error| match error {
        CopyError::Cancelled => String::from("Image request cancelled"),
        CopyError::TooLarge => String::from("Image exceeds the 64 MiB cache limit"),
        CopyError::Io(message) => message,
    })?;
    drop(output);
    if written == 0 || declared.is_some_and(|length| length != written) {
        return Err("Image download was interrupted".into());
    }
    if cancelled() {
        return Err("Image request cancelled".into());
    }
    fs::rename(&partial.0, &path).map_err(|error| error.to_string())?;
    Ok((path, written))
}

/// Makes sure the image with `key` is in the cache directory `cache_root`
/// and returns its path.
///
/// When the file is already there it is returned at once, without a request.
/// Otherwise `url` (`https://` or `http://`, no credentials) is fetched with
/// a GET carrying `headers`; redirects are followed, any status outside
/// 2xx is an error, and a body over [`MAX_IMAGE_BYTES`] is refused. `key` is
/// the image's explicit cache key, or empty to use the URL. Setting `cancel`
/// stops the download; nothing partial is ever left under the final name.
pub fn fetch_to_cache(
    client: &dyn HttpClient,
    cache_root: &Path,
    url: &str,
    key: &str,
    headers: &[(String, String)],
    cancel: &AtomicBool,
) -> Result<PathBuf, String> {
    fetch(client, cache_root, url, key, headers, cancel).map(|(path, _)| path)
}

/// [`fetch_to_cache`] for the renderer: `progress` receives the bytes
/// received and the declared total (zero when unknown) while downloading.
pub fn fetch_to_cache_reporting(
    client: &dyn HttpClient,
    cache_root: &Path,
    url: &str,
    key: &str,
    headers: &[(String, String)],
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<PathBuf, String> {
    fetch_reporting(client, cache_root, url, key, headers, cancel, progress).map(|(path, _)| path)
}

/// The shared outcome of one `prefetch` call, answered when every image is done.
struct Batch {
    succeeded: AtomicI64,
    failed: AtomicI64,
    bytes: AtomicI64,
    remaining: AtomicUsize,
    reply: Mutex<Option<Reply>>,
}

impl Batch {
    fn finish_one(&self, outcome: Result<u64, String>) {
        match outcome {
            Ok(bytes) => {
                self.succeeded.fetch_add(1, Ordering::Relaxed);
                self.bytes
                    .fetch_add(i64::try_from(bytes).unwrap_or(i64::MAX), Ordering::Relaxed);
            }
            Err(_) => {
                self.failed.fetch_add(1, Ordering::Relaxed);
            }
        }
        if self.remaining.fetch_sub(1, Ordering::AcqRel) != 1 {
            return;
        }
        let reply = self.reply.lock().ok().and_then(|mut reply| reply.take());
        if let Some(reply) = reply {
            reply.map(
                &WireMap::new()
                    .with("succeeded", self.succeeded.load(Ordering::Relaxed))
                    .with("failed", self.failed.load(Ordering::Relaxed))
                    .with("bytes", self.bytes.load(Ordering::Relaxed)),
            );
        }
    }
}

/// Decodes a JSON array of strings; entries of another type become empty.
fn text_array(json: &str) -> Option<Vec<String>> {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(serde_json::Value::Array(items)) => Some(
            items
                .into_iter()
                .map(|item| match item {
                    serde_json::Value::String(text) => text,
                    _ => String::new(),
                })
                .collect(),
        ),
        _ => None,
    }
}

pub struct ImagePrefetch {
    root: PathBuf,
    client: Arc<dyn HttpClient>,
    pool: WorkerPool,
    /// Set when the worker that asked is gone: its downloads stop.
    stale: Arc<AtomicBool>,
}

impl ImagePrefetch {
    /// `cache_dir` is the application's cache directory; images go to its
    /// `pam-images-v1` subdirectory ([`cache_root`]).
    #[must_use]
    pub fn new(cache_dir: &Path, client: Arc<dyn HttpClient>) -> Self {
        Self {
            root: cache_root(cache_dir),
            client,
            pool: WorkerPool::new("pam-image-prefetch", PREFETCH_THREADS),
            stale: Arc::new(AtomicBool::new(false)),
        }
    }

    fn prefetch(&self, request: &WireMap, reply: Reply) {
        let Some(urls) = request.text("urls").and_then(text_array) else {
            return reply.failure("Image URLs are required");
        };
        if urls.is_empty() || urls.len() > MAX_URLS {
            return reply.failure("Prefetch between 1 and 100 images");
        }
        let priority =
            u8::try_from(request.integer("priority").unwrap_or(2).clamp(1, 5)).unwrap_or(2);
        let headers = Arc::new(parse_request_headers(
            request.text("headers").unwrap_or_default(),
        ));
        let keys = request
            .text("cacheKeys")
            .and_then(text_array)
            .unwrap_or_default();
        let batch = Arc::new(Batch {
            succeeded: AtomicI64::new(0),
            failed: AtomicI64::new(0),
            bytes: AtomicI64::new(0),
            remaining: AtomicUsize::new(urls.len()),
            reply: Mutex::new(Some(reply)),
        });
        for (index, url) in urls.into_iter().enumerate() {
            let key = keys
                .get(index)
                .filter(|key| !key.trim().is_empty())
                .cloned()
                .unwrap_or_default();
            let batch = Arc::clone(&batch);
            let headers = Arc::clone(&headers);
            let client = Arc::clone(&self.client);
            let root = self.root.clone();
            let stale = Arc::clone(&self.stale);
            self.pool.run_with_priority(priority, move || {
                // Packaged assets are already on disk: nothing to download.
                let packaged = url
                    .get(..8)
                    .is_some_and(|scheme| scheme.eq_ignore_ascii_case("asset://"))
                    && url.len() <= MAX_URL_BYTES;
                let outcome = if packaged {
                    Ok(0)
                } else {
                    fetch(client.as_ref(), &root, &url, &key, &headers, &stale)
                        .map(|(_, bytes)| bytes)
                };
                batch.finish_one(outcome);
            });
        }
    }
}

impl Module for ImagePrefetch {
    fn call(&mut self, method: &str, payload: &[u8], reply: Reply) {
        let request = match request_map(payload) {
            Ok(request) => request,
            Err(error) => return reply.failure(error),
        };
        match method {
            "prefetch" => self.prefetch(&request, reply),
            other => reply.failure(format!("Unknown image method {other}")),
        }
    }

    fn reset(&mut self) {
        self.stale.store(true, Ordering::Relaxed);
        self.stale = Arc::new(AtomicBool::new(false));
    }
}

impl Drop for ImagePrefetch {
    fn drop(&mut self) {
        self.stale.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::test_support::{PlainClient, Served, TestServer, temporary_directory};
    use crate::runtime::{MODULE_FAILURE, MODULE_SUCCESS};

    const WAIT: Duration = Duration::from_secs(10);

    fn pairs(headers: &[(&str, &str)]) -> Vec<(String, String)> {
        headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn call(
        module: &mut ImagePrefetch,
        method: &str,
        request: &WireMap,
    ) -> Result<WireMap, String> {
        let (reply, answer) = Reply::channel();
        module.call(method, &request.encode(), reply);
        let (status, payload) = answer.recv_timeout(WAIT).unwrap();
        if status == MODULE_SUCCESS {
            Ok(request_map(&payload).unwrap())
        } else {
            assert_eq!(status, MODULE_FAILURE);
            Err(String::from_utf8(payload).unwrap())
        }
    }

    #[test]
    fn cache_files_are_named_by_the_digest_of_the_key() {
        let root = cache_root(Path::new("/var/cache/app"));
        assert_eq!(root, Path::new("/var/cache/app/pam-images-v1"));
        assert_eq!(
            cache_path(&root, "abc"),
            root.join("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        assert_ne!(
            cache_path(&root, "https://a.example/x.png"),
            cache_path(&root, "https://a.example/y.png")
        );
    }

    #[test]
    fn packed_headers_keep_only_safe_lines() {
        assert_eq!(parse_request_headers(""), []);
        assert_eq!(
            parse_request_headers("Authorization:Bearer a:b\nX-Tenant: acme \nAccept:image/webp"),
            pairs(&[
                ("Authorization", "Bearer a:b"),
                ("X-Tenant", "acme"),
                ("Accept", "image/webp"),
            ])
        );
        assert_eq!(
            parse_request_headers(
                "no separator\nBad Name:v\n:empty\nHost:evil.example\nContent-Length:5\nX-Ok:1\nX-Cr:a\rb\nX-Nul:a\0b"
            ),
            pairs(&[("X-Ok", "1")])
        );
        let long = format!("X-Long:{}\nX-Fits:{}", "v".repeat(4097), "v".repeat(4096));
        assert_eq!(parse_request_headers(&long).len(), 1);
        let many: Vec<String> = (0..40).map(|index| format!("X-H{index}:v")).collect();
        assert_eq!(parse_request_headers(&many.join("\n")).len(), 32);
    }

    #[test]
    fn images_are_fetched_once_and_then_served_from_the_cache() {
        let server = TestServer::start(|request| match request.path.as_str() {
            "/logo.png" => {
                Served::ok(b"\x89PNG-bytes".to_vec()).header("Content-Type", "image/png")
            }
            "/moved.png" => Served::status(301, Vec::new()).header("Location", "/logo.png"),
            "/empty.png" => Served::ok(Vec::new()),
            _ => Served::status(404, b"missing".to_vec()),
        });
        let directory = temporary_directory("image-fetch");
        let root = cache_root(&directory);
        let cancel = AtomicBool::new(false);
        let url = server.url("/logo.png");
        let headers = pairs(&[("Authorization", "Bearer t")]);

        let path = fetch_to_cache(&PlainClient, &root, &url, "", &headers, &cancel).unwrap();
        assert_eq!(path, cache_path(&root, &url));
        assert_eq!(fs::read(&path).unwrap(), b"\x89PNG-bytes");
        let received = &server.requests()[0];
        assert_eq!(received.method, "GET");
        assert_eq!(received.header("authorization"), Some("Bearer t"));
        assert_eq!(received.header("accept"), Some(DEFAULT_ACCEPT));

        // A second call is a cache hit: no request.
        assert_eq!(
            fetch_to_cache(&PlainClient, &root, &url, "", &[], &cancel).unwrap(),
            path
        );
        assert_eq!(server.requests().len(), 1);

        // An explicit key names the file instead of the URL.
        let keyed = fetch_to_cache(&PlainClient, &root, &url, "avatar-7", &[], &cancel).unwrap();
        assert_eq!(keyed, cache_path(&root, "avatar-7"));
        assert_eq!(server.requests().len(), 2);
        // With the key cached, the URL is not even looked at.
        assert_eq!(
            fetch_to_cache(&PlainClient, &root, "not a url", "avatar-7", &[], &cancel).unwrap(),
            keyed
        );

        let error = |path: &str| {
            fetch_to_cache(&PlainClient, &root, &server.url(path), "", &[], &cancel).unwrap_err()
        };
        assert_eq!(error("/missing.png"), "Image request failed with HTTP 404");
        // Redirects are not followed.
        assert_eq!(error("/moved.png"), "Image request failed with HTTP 301");
        assert_eq!(error("/empty.png"), "Image download was interrupted");
        for invalid in [
            "ftp://example.com/a.png",
            "asset://logo.png",
            "/local/file.png",
            "https://user:pw@example.com/a.png",
        ] {
            assert_eq!(
                fetch_to_cache(&PlainClient, &root, invalid, "", &[], &cancel).unwrap_err(),
                "Image URL is invalid"
            );
        }
        assert_eq!(
            fetch_to_cache(
                &PlainClient,
                &root,
                &server.url("/other.png"),
                "",
                &pairs(&[("X-Bad", "a\r\nInjected: 1")]),
                &cancel
            )
            .unwrap_err(),
            "Image request headers are invalid"
        );
        // Only whole images are in the cache: two files, no leftovers.
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_cancelled_fetch_leaves_nothing_behind() {
        let server = TestServer::start(|_| {
            Served::ok(vec![b'i'; 300_000]).trickle(4096, Duration::from_millis(10))
        });
        let directory = temporary_directory("image-cancel");
        let root = cache_root(&directory);
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        let stopper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(60));
            flag.store(true, Ordering::Relaxed);
        });
        let error = fetch_to_cache(
            &PlainClient,
            &root,
            &server.url("/big.png"),
            "",
            &[],
            &cancel,
        )
        .unwrap_err();
        stopper.join().unwrap();
        assert_eq!(error, "Image request cancelled");
        assert_eq!(fs::read_dir(&root).map(Iterator::count).unwrap_or(0), 0);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn prefetch_counts_successes_failures_and_bytes() {
        let server = TestServer::start(|request| match request.path.as_str() {
            "/a.png" => Served::ok(vec![1_u8; 100]),
            "/b.png" => Served::ok(vec![2_u8; 250]),
            _ => Served::status(500, Vec::new()),
        });
        let directory = temporary_directory("image-prefetch");
        let mut module = ImagePrefetch::new(&directory, Arc::new(PlainClient));
        let urls = serde_json::json!([
            server.url("/a.png"),
            server.url("/b.png"),
            server.url("/broken.png"),
            "asset://images/logo.png",
            "ftp://example.com/x.png",
        ]);
        let keys = serde_json::json!(["", "banner", "", "", ""]);
        let request = WireMap::new()
            .with("urls", urls.to_string())
            .with("priority", 4)
            .with("headers", "X-Tenant:acme")
            .with("cacheKeys", keys.to_string());
        let result = call(&mut module, "prefetch", &request).unwrap();
        assert_eq!(result.integer("succeeded"), Some(3));
        assert_eq!(result.integer("failed"), Some(2));
        assert_eq!(result.integer("bytes"), Some(350));
        let root = cache_root(&directory);
        assert_eq!(
            fs::read(cache_path(&root, &server.url("/a.png")))
                .unwrap()
                .len(),
            100
        );
        assert_eq!(fs::read(cache_path(&root, "banner")).unwrap().len(), 250);
        assert!(
            server
                .requests()
                .iter()
                .all(|request| request.header("x-tenant") == Some("acme"))
        );

        // Prefetching again downloads nothing.
        let before = server.requests().len();
        let again = call(
            &mut module,
            "prefetch",
            &WireMap::new().with("urls", urls.to_string()),
        )
        .unwrap();
        // Without the cache key the second image is fetched under its URL.
        assert_eq!(again.integer("succeeded"), Some(3));
        assert_eq!(again.integer("bytes"), Some(250));
        assert_eq!(server.requests().len(), before + 2);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn prefetch_requests_are_validated() {
        let directory = temporary_directory("image-validation");
        let mut module = ImagePrefetch::new(&directory, Arc::new(PlainClient));
        assert_eq!(
            call(&mut module, "prefetch", &WireMap::new()).unwrap_err(),
            "Image URLs are required"
        );
        assert_eq!(
            call(
                &mut module,
                "prefetch",
                &WireMap::new().with("urls", "nope")
            )
            .unwrap_err(),
            "Image URLs are required"
        );
        assert_eq!(
            call(&mut module, "prefetch", &WireMap::new().with("urls", "[]")).unwrap_err(),
            "Prefetch between 1 and 100 images"
        );
        let many = serde_json::Value::from(vec!["asset://a.png"; 101]).to_string();
        assert_eq!(
            call(&mut module, "prefetch", &WireMap::new().with("urls", many)).unwrap_err(),
            "Prefetch between 1 and 100 images"
        );
        let hundred = serde_json::Value::from(vec!["asset://a.png"; 100]).to_string();
        let result = call(
            &mut module,
            "prefetch",
            &WireMap::new().with("urls", hundred),
        )
        .unwrap();
        assert_eq!(result.integer("succeeded"), Some(100));
        assert_eq!(result.integer("bytes"), Some(0));
        assert_eq!(
            call(&mut module, "decode", &WireMap::new()).unwrap_err(),
            "Unknown image method decode"
        );
        let _ = fs::remove_dir_all(&directory);
    }
}
