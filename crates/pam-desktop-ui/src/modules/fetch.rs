//! The HTTP client contract.
//!
//! The render core and its modules (network requests, downloads, images,
//! updates) describe what they need; the host binary provides the
//! implementation with a real TLS stack. Everything here is blocking and is
//! called from worker threads, never from the UI thread.

use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub enum HttpBody {
    None,
    Bytes(Vec<u8>),
    /// Streamed from disk with a fixed `Content-Length`.
    File { path: PathBuf, length: u64 },
    /// Streamed from a reader with a fixed `Content-Length` (multipart bodies).
    Stream { reader: Box<dyn Read + Send>, length: u64 },
}

pub struct HttpRequest {
    /// Upper-case method.
    pub method: String,
    pub url: String,
    /// Sent as given, in order.
    pub headers: Vec<(String, String)>,
    pub body: HttpBody,
    pub connect_timeout: Duration,
    /// Maximum time without receiving data.
    pub read_timeout: Duration,
    /// Deadline for the whole exchange, when one applies.
    pub total_timeout: Option<Duration>,
    /// Redirects are returned to the caller unless this is set.
    pub follow_redirects: bool,
}

impl HttpRequest {
    #[must_use]
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: String::from("GET"),
            url: url.into(),
            headers: Vec::new(),
            body: HttpBody::None,
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(30),
            total_timeout: None,
            follow_redirects: false,
        }
    }
}

pub struct HttpResponse {
    pub status: u16,
    /// Names lower-cased; repeated headers appear once per value.
    pub headers: Vec<(String, String)>,
    /// `Content-Length`, when the server declared one.
    pub content_length: Option<u64>,
    /// The body, read lazily. Reading fails once the request is cancelled.
    pub body: Box<dyn Read + Send>,
}

impl HttpResponse {
    /// First value of a header (lower-case name).
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(header, _)| header == name).map(|(_, value)| value.as_str())
    }
}

/// Upload progress: bytes sent so far and the total body length.
pub type UploadProgress<'a> = &'a (dyn Fn(u64, u64) + Sync);

pub trait HttpClient: Send + Sync {
    /// Performs the request and returns once the response headers arrived.
    /// Any HTTP status is a success; `Err` carries a transport error message
    /// (DNS, TLS, timeout…). `cancel` aborts the exchange when set.
    fn send(&self, request: HttpRequest, progress: Option<UploadProgress<'_>>, cancel: &AtomicBool) -> Result<HttpResponse, String>;
}
