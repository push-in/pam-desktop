//! The host's HTTP stack: `ureq` over the system's TLS.
//!
//! macOS and Windows use the TLS implementation of the system (Secure
//! Transport, SChannel); other systems use rustls. In both cases server
//! certificates are verified against the trust store of the system, so
//! certificate authorities installed by the user or an administrator work.
//!
//! Nothing here logs: header values routinely carry credentials.

use std::io::{Cursor, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use pam_desktop_ui::modules::fetch::{
    HttpBody, HttpClient, HttpRequest, HttpResponse, UploadProgress,
};
use ureq::config::AutoHeaderValue;
use ureq::http;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use ureq::{Agent, SendBody};

/// Redirects followed when a request asks for them.
const MAX_REDIRECTS: u32 = 10;
const CANCELLED: &str = "Request cancelled";

/// A blocking HTTP/1.1 client with pooled connections.
pub struct UreqClient {
    agent: Agent,
}

impl Default for UreqClient {
    fn default() -> Self {
        Self::new()
    }
}

impl UreqClient {
    #[must_use]
    pub fn new() -> Self {
        let provider = if cfg!(any(target_os = "macos", target_os = "windows")) {
            TlsProvider::NativeTls
        } else {
            TlsProvider::Rustls
        };
        let tls = TlsConfig::builder()
            .provider(provider)
            .root_certs(RootCerts::PlatformVerifier)
            .build();
        let config = Agent::config_builder()
            .tls_config(tls)
            // Every status is a response: the application decides what an error is.
            .http_status_as_error(false)
            .max_redirects(0)
            .user_agent(concat!("pam-desktop/", env!("CARGO_PKG_VERSION")))
            // Only the headers the request carries are sent.
            .accept(AutoHeaderValue::None)
            .accept_encoding(AutoHeaderValue::None)
            .build();
        Self {
            agent: Agent::new_with_config(config),
        }
    }
}

/// The request body as `ureq` reads it: exactly `total` bytes, with progress
/// reported after every chunk and cancellation checked before each.
struct Outgoing<'a> {
    source: Box<dyn Read + Send>,
    sent: u64,
    total: u64,
    progress: Option<UploadProgress<'a>>,
    cancel: &'a AtomicBool,
}

impl Read for Outgoing<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other(CANCELLED));
        }
        let remaining = self.total - self.sent;
        if remaining == 0 || buffer.is_empty() {
            return Ok(0);
        }
        let wanted = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        let count = self.source.read(&mut buffer[..wanted])?;
        if count == 0 {
            // A short body would leave the server waiting for the rest.
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "Request body ended before its declared length",
            ));
        }
        self.sent += count as u64;
        if let Some(progress) = self.progress {
            progress(self.sent, self.total);
        }
        Ok(count)
    }
}

/// The transport failure as the application reads it.
fn message(error: &ureq::Error) -> String {
    match error {
        ureq::Error::Timeout(ureq::Timeout::Resolve | ureq::Timeout::Connect) => {
            String::from("Connection timed out")
        }
        ureq::Error::Timeout(_) => String::from("Request timed out"),
        ureq::Error::HostNotFound => String::from("Host not found"),
        ureq::Error::ConnectionFailed => String::from("Connection failed"),
        ureq::Error::Io(error) => error.to_string(),
        other => other.to_string(),
    }
}

/// Whether the caller's header is one the client writes itself to frame the body.
fn frames_body(name: &str) -> bool {
    name.eq_ignore_ascii_case("content-length") || name.eq_ignore_ascii_case("transfer-encoding")
}

/// The limits of one exchange.
struct Exchange {
    connect_timeout: Duration,
    read_timeout: Duration,
    total_timeout: Option<Duration>,
    follow_redirects: bool,
}

impl UreqClient {
    fn run(
        &self,
        builder: http::request::Builder,
        body: SendBody<'_>,
        exchange: &Exchange,
    ) -> Result<http::Response<ureq::Body>, String> {
        // The reason is left out: it could quote a header value.
        let request = builder
            .body(body)
            .map_err(|_| String::from("Invalid HTTP request"))?;
        let request = self
            .agent
            .configure_request(request)
            .timeout_resolve(Some(exchange.connect_timeout))
            .timeout_connect(Some(exchange.connect_timeout))
            // Each read and write has this long: a stalled peer ends the
            // exchange, a slow but steady one does not.
            .timeout_per_read(Some(exchange.read_timeout))
            .timeout_per_write(Some(exchange.read_timeout))
            .timeout_global(exchange.total_timeout)
            .max_redirects(if exchange.follow_redirects {
                MAX_REDIRECTS
            } else {
                0
            })
            .build();
        self.agent.run(request).map_err(|error| message(&error))
    }
}

impl HttpClient for UreqClient {
    fn send(
        &self,
        request: HttpRequest,
        progress: Option<UploadProgress<'_>>,
        cancel: &AtomicBool,
    ) -> Result<HttpResponse, String> {
        let cancelled = || cancel.load(Ordering::Relaxed);
        if cancelled() {
            return Err(CANCELLED.into());
        }
        let method = http::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| String::from("Unsupported HTTP method"))?;
        let mut builder = http::Request::builder()
            .method(method)
            .uri(request.url.as_str());
        for (name, value) in &request.headers {
            if !frames_body(name) {
                builder = builder.header(name.as_str(), value.as_str());
            }
        }
        let source: Option<(Box<dyn Read + Send>, u64)> = match request.body {
            HttpBody::None => None,
            HttpBody::Bytes(bytes) => {
                let length = bytes.len() as u64;
                Some((Box::new(Cursor::new(bytes)), length))
            }
            HttpBody::File { path, length } => {
                let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
                Some((Box::new(file), length))
            }
            HttpBody::Stream { reader, length } => Some((reader, length)),
        };
        let exchange = Exchange {
            connect_timeout: request.connect_timeout,
            read_timeout: request.read_timeout,
            total_timeout: request.total_timeout,
            follow_redirects: request.follow_redirects,
        };
        let outcome = match source {
            Some((source, total)) => {
                let mut outgoing = Outgoing {
                    source,
                    sent: 0,
                    total,
                    progress,
                    cancel,
                };
                // A declared length makes the body fixed-length, not chunked.
                let builder = builder.header(http::header::CONTENT_LENGTH, total);
                self.run(builder, SendBody::from_reader(&mut outgoing), &exchange)
            }
            None => self.run(builder, SendBody::none(), &exchange),
        };
        let response = outcome.map_err(|error| {
            if cancelled() {
                String::from(CANCELLED)
            } else {
                error
            }
        })?;
        if cancelled() {
            return Err(CANCELLED.into());
        }
        let (parts, body) = response.into_parts();
        let headers: Vec<(String, String)> = parts
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_ascii_lowercase(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
        let content_length = parts
            .headers
            .get(http::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok());
        Ok(HttpResponse {
            status: parts.status.as_u16(),
            headers,
            content_length,
            body: Box::new(body.into_reader()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pam_desktop_ui::modules::test_support::{Served, TestServer, temporary_directory};
    use std::sync::Mutex;
    use std::time::Instant;

    fn request(method: &str, url: String) -> HttpRequest {
        HttpRequest {
            method: method.to_owned(),
            ..HttpRequest::get(url)
        }
    }

    fn text(response: HttpResponse) -> String {
        let mut body = String::new();
        let mut reader = response.body;
        reader.read_to_string(&mut body).unwrap();
        body
    }

    fn never() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn responses_carry_status_lower_case_headers_and_the_body() {
        let server = TestServer::start(|request| match request.path.as_str() {
            "/hello" => Served::ok("olá")
                .header("Content-Type", "text/plain; charset=utf-8")
                .header("X-Request-ID", "r-1")
                .header("Set-Cookie", "a=1")
                .header("Set-Cookie", "b=2"),
            _ => Served::status(404, "missing"),
        });
        let client = UreqClient::new();
        let response = client
            .send(request("GET", server.url("/hello")), None, &never())
            .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.content_length, Some(4));
        assert_eq!(
            response.header("content-type"),
            Some("text/plain; charset=utf-8")
        );
        assert_eq!(response.header("x-request-id"), Some("r-1"));
        let cookies: Vec<&str> = response
            .headers
            .iter()
            .filter(|(name, _)| name == "set-cookie")
            .map(|(_, value)| value.as_str())
            .collect();
        assert_eq!(cookies, ["a=1", "b=2"]);
        assert_eq!(text(response), "olá");

        // An error status is still a response.
        let missing = client
            .send(request("GET", server.url("/nope")), None, &never())
            .unwrap();
        assert_eq!(missing.status, 404);
        assert_eq!(text(missing), "missing");

        // Only the request's own headers are sent, besides the user agent.
        let received = &server.requests()[0];
        assert_eq!(received.method, "GET");
        assert_eq!(received.header("accept"), None);
        assert_eq!(received.header("accept-encoding"), None);
        assert!(
            received
                .header("user-agent")
                .is_some_and(|agent| agent.starts_with("pam-desktop/"))
        );
    }

    #[test]
    fn redirects_are_returned_unless_asked_to_follow() {
        let server = TestServer::start(|request| match request.path.as_str() {
            "/old" => Served::status(302, "").header("Location", "/new"),
            _ => Served::ok("arrived"),
        });
        let client = UreqClient::new();
        let response = client
            .send(request("GET", server.url("/old")), None, &never())
            .unwrap();
        assert_eq!(response.status, 302);
        assert_eq!(response.header("location"), Some("/new"));
        assert_eq!(server.requests().len(), 1);

        let following = HttpRequest {
            follow_redirects: true,
            ..request("GET", server.url("/old"))
        };
        let response = client.send(following, None, &never()).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(text(response), "arrived");
    }

    #[test]
    fn bodies_are_sent_with_a_fixed_length_and_the_given_headers() {
        let server = TestServer::start(|_| Served::status(201, "created"));
        let client = UreqClient::new();
        let post = HttpRequest {
            headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("Authorization".into(), "Bearer secret".into()),
                // The client frames the body itself.
                ("Content-Length".into(), "999".into()),
            ],
            body: HttpBody::Bytes(br#"{"name":"Ana"}"#.to_vec()),
            ..request("POST", server.url("/users"))
        };
        let response = client.send(post, None, &never()).unwrap();
        assert_eq!(response.status, 201);
        assert_eq!(text(response), "created");
        // Methods that usually have no body can carry one too.
        let delete = HttpRequest {
            body: HttpBody::Bytes(b"reason".to_vec()),
            ..request("DELETE", server.url("/users/1"))
        };
        assert_eq!(client.send(delete, None, &never()).unwrap().status, 201);

        let received = server.requests();
        assert_eq!(received[0].method, "POST");
        assert_eq!(received[0].path, "/users");
        assert_eq!(received[0].body, br#"{"name":"Ana"}"#);
        assert_eq!(received[0].header("content-length"), Some("14"));
        assert_eq!(received[0].header("content-type"), Some("application/json"));
        assert_eq!(received[0].header("authorization"), Some("Bearer secret"));
        assert_eq!(received[0].header("transfer-encoding"), None);
        assert_eq!(received[1].method, "DELETE");
        assert_eq!(received[1].body, b"reason");
    }

    #[test]
    fn files_and_streams_upload_with_progress() {
        let server = TestServer::start(|_| Served::ok("stored"));
        let client = UreqClient::new();
        let directory = temporary_directory("ureq-upload");
        let contents: Vec<u8> = (0..300_000_u32).map(|index| (index % 251) as u8).collect();
        let path = directory.join("upload.bin");
        std::fs::write(&path, &contents).unwrap();

        let reports = Mutex::new(Vec::new());
        let progress = |sent: u64, total: u64| reports.lock().unwrap().push((sent, total));
        let upload = HttpRequest {
            body: HttpBody::File {
                path,
                length: 300_000,
            },
            ..request("PUT", server.url("/file"))
        };
        let response = client.send(upload, Some(&progress), &never()).unwrap();
        assert_eq!(response.status, 200);
        let seen = reports.lock().unwrap().clone();
        assert!(seen.len() > 1);
        assert!(seen.iter().all(|(_, total)| *total == 300_000));
        assert!(seen.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert_eq!(seen.last(), Some(&(300_000, 300_000)));

        let stream = HttpRequest {
            body: HttpBody::Stream {
                reader: Box::new(Cursor::new(contents.clone())),
                length: 300_000,
            },
            ..request("POST", server.url("/stream"))
        };
        assert_eq!(client.send(stream, None, &never()).unwrap().status, 200);

        let received = server.requests();
        assert_eq!(received[0].method, "PUT");
        assert_eq!(received[0].header("content-length"), Some("300000"));
        assert!(received[0].body == contents);
        assert!(received[1].body == contents);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_body_shorter_than_declared_fails_the_request() {
        let server = TestServer::start(|_| Served::ok("never"));
        let client = UreqClient::new();
        let short = HttpRequest {
            body: HttpBody::Stream {
                reader: Box::new(Cursor::new(b"abc".to_vec())),
                length: 10,
            },
            read_timeout: Duration::from_secs(2),
            ..request("POST", server.url("/short"))
        };
        let error = client.send(short, None, &never()).err().unwrap();
        assert!(error.contains("declared length"), "{error}");
    }

    #[test]
    fn cancellation_stops_before_and_during_the_upload() {
        let server = TestServer::start(|_| Served::ok("never"));
        let client = UreqClient::new();
        let cancelled = AtomicBool::new(true);
        let error = client
            .send(request("GET", server.url("/")), None, &cancelled)
            .err()
            .unwrap();
        assert_eq!(error, CANCELLED);
        assert_eq!(server.requests().len(), 0);

        // The flag is raised by the progress of the first chunk.
        let cancel = AtomicBool::new(false);
        let progress = |_: u64, _: u64| cancel.store(true, Ordering::Relaxed);
        let upload = HttpRequest {
            body: HttpBody::Bytes(vec![7_u8; 2_000_000]),
            ..request("POST", server.url("/big"))
        };
        let error = client.send(upload, Some(&progress), &cancel).err().unwrap();
        assert_eq!(error, CANCELLED);
    }

    #[test]
    fn transport_failures_are_messages() {
        let client = UreqClient::new();
        // Nothing listens on this port.
        let closed = {
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            listener.local_addr().unwrap().port()
        };
        let refused = client
            .send(
                request("GET", format!("http://127.0.0.1:{closed}/")),
                None,
                &never(),
            )
            .err()
            .unwrap();
        assert!(!refused.is_empty());

        for invalid in ["not a url", "http://exa mple.com/"] {
            assert!(
                client
                    .send(request("GET", invalid.to_owned()), None, &never())
                    .is_err()
            );
        }
        let injected = HttpRequest {
            headers: vec![("X-Bad".into(), "a\r\nInjected: secret-value".into())],
            ..request("GET", format!("http://127.0.0.1:{closed}/"))
        };
        let error = client.send(injected, None, &never()).err().unwrap();
        assert_eq!(error, "Invalid HTTP request");
        let unsupported = client
            .send(
                request("BAD METHOD", format!("http://127.0.0.1:{closed}/")),
                None,
                &never(),
            )
            .err()
            .unwrap();
        assert_eq!(unsupported, "Unsupported HTTP method");
    }

    /// Run it on purpose (`--ignored`): it needs the network. It shows that
    /// HTTPS works with the trust store of the system and that a certificate
    /// the system does not trust ends the request.
    #[test]
    #[ignore = "needs network access"]
    fn https_is_verified_against_the_system_trust_store() {
        let client = UreqClient::new();
        let response = client
            .send(
                request("GET", String::from("https://example.com/")),
                None,
                &never(),
            )
            .unwrap();
        assert_eq!(response.status, 200);
        assert!(text(response).contains("Example Domain"));
        for untrusted in [
            "https://self-signed.badssl.com/",
            "https://expired.badssl.com/",
            "https://wrong.host.badssl.com/",
        ] {
            let outcome = client.send(request("GET", untrusted.to_owned()), None, &never());
            assert!(outcome.is_err(), "{untrusted}");
        }
    }

    #[test]
    fn a_silent_server_times_out() {
        let server = TestServer::start(|_| {
            std::thread::sleep(Duration::from_millis(1500));
            Served::ok("late")
        });
        let client = UreqClient::new();
        let started = Instant::now();
        let impatient = HttpRequest {
            read_timeout: Duration::from_millis(200),
            ..request("GET", server.url("/slow"))
        };
        let error = client.send(impatient, None, &never()).err().unwrap();
        assert_eq!(error, "Request timed out");
        assert!(started.elapsed() < Duration::from_millis(1400));

        // The read timeout restarts with every read; the total one does not.
        let trickling = TestServer::start(|_| {
            Served::ok(vec![b'x'; 40_000]).trickle(2000, Duration::from_millis(40))
        });
        let patient = HttpRequest {
            read_timeout: Duration::from_millis(400),
            ..request("GET", trickling.url("/trickle"))
        };
        let response = client.send(patient, None, &never()).unwrap();
        assert_eq!(text(response).len(), 40_000);

        let bounded = HttpRequest {
            read_timeout: Duration::from_millis(400),
            total_timeout: Some(Duration::from_millis(250)),
            ..request("GET", trickling.url("/trickle"))
        };
        let outcome = client.send(bounded, None, &never()).and_then(|response| {
            let mut body = Vec::new();
            let mut reader = response.body;
            reader
                .read_to_end(&mut body)
                .map_err(|error| error.to_string())
        });
        assert!(outcome.is_err());
    }
}
