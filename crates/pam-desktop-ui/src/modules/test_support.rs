//! Test doubles for the network modules: a loopback HTTP/1.1 server and, in
//! this crate's own tests, an [`HttpClient`](super::fetch::HttpClient) that
//! speaks plain HTTP over TCP.
//!
//! The server is public so the host binary can test its real client against
//! it. Nothing here is meant for an application.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// A fresh directory under the system's temporary directory.
#[must_use]
pub fn temporary_directory(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "pam-desktop-test-{}-{name}-{}",
        std::process::id(),
        super::support::uuid()
    ));
    let _ = std::fs::create_dir_all(&directory);
    directory
}

/// A request the server received.
#[derive(Clone, Debug)]
pub struct Received {
    pub method: String,
    /// Path and query, as sent.
    pub path: String,
    /// Names lower-cased, in the order received.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Received {
    /// First value of a header (lower-case name).
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header == name)
            .map(|(_, value)| value.as_str())
    }
}

/// What the server answers.
#[derive(Clone, Debug)]
pub struct Served {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    declare_length: bool,
    chunk: usize,
    pause: Duration,
}

impl Served {
    #[must_use]
    pub fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self::status(200, body)
    }

    #[must_use]
    pub fn status(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into(),
            declare_length: true,
            chunk: usize::MAX,
            pause: Duration::ZERO,
        }
    }

    #[must_use]
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    /// Sends no `Content-Length`: the body ends when the connection closes.
    #[must_use]
    pub fn without_length(mut self) -> Self {
        self.declare_length = false;
        self
    }

    /// Sends the body `chunk` bytes at a time, pausing between chunks.
    #[must_use]
    pub fn trickle(mut self, chunk: usize, pause: Duration) -> Self {
        self.chunk = chunk.max(1);
        self.pause = pause;
        self
    }
}

type Handler = dyn Fn(&Received) -> Served + Send + Sync;

/// An HTTP/1.1 server on `127.0.0.1` that answers every request through a
/// closure and remembers what it received. One request per connection.
pub struct TestServer {
    port: u16,
    requests: Arc<Mutex<Vec<Received>>>,
    stop: Arc<AtomicBool>,
}

impl TestServer {
    /// Starts listening on a free loopback port.
    ///
    /// # Panics
    ///
    /// When no loopback port can be bound.
    #[must_use]
    pub fn start(handler: impl Fn(&Received) -> Served + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind a loopback port");
        let port = listener.local_addr().expect("local address").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let handler: Arc<Handler> = Arc::new(handler);
        let (seen, stopping) = (Arc::clone(&requests), Arc::clone(&stop));
        thread::spawn(move || {
            for stream in listener.incoming() {
                if stopping.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let (handler, seen) = (Arc::clone(&handler), Arc::clone(&seen));
                thread::spawn(move || {
                    let _ = serve(stream, handler.as_ref(), &seen);
                });
            }
        });
        Self {
            port,
            requests,
            stop,
        }
    }

    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// `http://127.0.0.1:<port><path>`.
    #[must_use]
    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// The requests received so far, oldest first.
    #[must_use]
    pub fn requests(&self) -> Vec<Received> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .unwrap_or_default()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

fn read_head(reader: &mut impl BufRead) -> std::io::Result<Vec<String>> {
    let mut lines = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            return Ok(lines);
        }
        lines.push(line.to_owned());
    }
}

fn parse_headers(lines: &[String]) -> Vec<(String, String)> {
    lines
        .iter()
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect()
}

fn serve(stream: TcpStream, handler: &Handler, seen: &Mutex<Vec<Received>>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let head = read_head(&mut reader)?;
    let mut request_line = head
        .first()
        .map(String::as_str)
        .unwrap_or_default()
        .split(' ');
    let method = request_line.next().unwrap_or_default().to_owned();
    let path = request_line.next().unwrap_or_default().to_owned();
    let headers = parse_headers(head.get(1..).unwrap_or_default());
    let length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body)?;
    let received = Received {
        method,
        path,
        headers,
        body,
    };
    let served = handler(&received);
    if let Ok(mut seen) = seen.lock() {
        seen.push(received);
    }

    let mut stream = stream;
    let mut head = format!("HTTP/1.1 {} Status\r\nConnection: close\r\n", served.status);
    for (name, value) in &served.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if served.declare_length {
        head.push_str(&format!("Content-Length: {}\r\n", served.body.len()));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    for chunk in served
        .body
        .chunks(served.chunk.min(served.body.len().max(1)))
    {
        stream.write_all(chunk)?;
        stream.flush()?;
        if !served.pause.is_zero() {
            thread::sleep(served.pause);
        }
    }
    stream.shutdown(Shutdown::Write)?;
    // Let the client finish reading before the socket goes away.
    let _ = reader.read(&mut [0_u8; 64]);
    Ok(())
}

#[cfg(test)]
pub(crate) use plain::PlainClient;

#[cfg(test)]
mod plain {
    use super::{parse_headers, read_head};
    use crate::modules::fetch::{HttpBody, HttpClient, HttpRequest, HttpResponse, UploadProgress};
    use crate::modules::http::parse_url;
    use std::io::{BufReader, Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};

    /// An `HttpClient` over plain TCP. It ignores the scheme, so modules
    /// that insist on `https://` can be exercised against the loopback
    /// server.
    pub(crate) struct PlainClient;

    fn text(error: std::io::Error) -> String {
        error.to_string()
    }

    impl HttpClient for PlainClient {
        fn send(
            &self,
            request: HttpRequest,
            progress: Option<UploadProgress<'_>>,
            cancel: &AtomicBool,
        ) -> Result<HttpResponse, String> {
            let url = parse_url(&request.url).ok_or("invalid URL")?;
            let port = url.port.ok_or("the test client needs an explicit port")?;
            let address: SocketAddr = format!("{}:{port}", url.host)
                .parse()
                .map_err(|_| String::from("the test client needs an IP address"))?;
            let mut stream =
                TcpStream::connect_timeout(&address, request.connect_timeout).map_err(text)?;
            stream
                .set_read_timeout(Some(request.read_timeout))
                .map_err(text)?;
            let after_scheme = request.url.split_once("://").map_or("", |(_, rest)| rest);
            let path = after_scheme
                .find('/')
                .map_or("/", |start| &after_scheme[start..]);
            let (mut body, length): (Box<dyn Read>, u64) = match request.body {
                HttpBody::None => (Box::new(std::io::empty()), 0),
                HttpBody::Bytes(bytes) => {
                    let length = bytes.len() as u64;
                    (Box::new(std::io::Cursor::new(bytes)), length)
                }
                HttpBody::File { path, length } => {
                    (Box::new(std::fs::File::open(path).map_err(text)?), length)
                }
                HttpBody::Stream { reader, length } => (reader, length),
            };
            let mut head = format!(
                "{} {path} HTTP/1.1\r\nHost: {}:{port}\r\nConnection: close\r\nContent-Length: {length}\r\n",
                request.method, url.host
            );
            for (name, value) in &request.headers {
                head.push_str(&format!("{name}: {value}\r\n"));
            }
            head.push_str("\r\n");
            stream.write_all(head.as_bytes()).map_err(text)?;
            let mut buffer = vec![0_u8; 16 * 1024];
            let mut sent = 0_u64;
            while sent < length {
                if cancel.load(Ordering::Relaxed) {
                    return Err("cancelled".into());
                }
                let wanted = usize::try_from(length - sent)
                    .unwrap_or(usize::MAX)
                    .min(buffer.len());
                let count = body.read(&mut buffer[..wanted]).map_err(text)?;
                if count == 0 {
                    return Err("request body ended early".into());
                }
                stream.write_all(&buffer[..count]).map_err(text)?;
                sent += count as u64;
                if let Some(progress) = progress {
                    progress(sent, length);
                }
            }
            let mut reader = BufReader::new(stream);
            let head = read_head(&mut reader).map_err(text)?;
            let status = head
                .first()
                .and_then(|line| line.split(' ').nth(1))
                .and_then(|code| code.parse::<u16>().ok())
                .ok_or("malformed response")?;
            let headers = parse_headers(head.get(1..).unwrap_or_default());
            let content_length = headers
                .iter()
                .find(|(name, _)| name == "content-length")
                .and_then(|(_, value)| value.parse::<u64>().ok());
            let body: Box<dyn Read + Send> = match content_length {
                Some(length) => Box::new(reader.take(length)),
                None => Box::new(reader),
            };
            Ok(HttpResponse {
                status,
                headers,
                content_length,
                body,
            })
        }
    }
}
