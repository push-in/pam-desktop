//! Bounded native RFC6455 transport. No URLs, credentials or message bodies are logged.
//! TLS and framing belong to tungstenite/rustls; PHP receives only explicit events.
use std::collections::{HashMap, VecDeque};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use pam_desktop_ui::modules::http::parse_url;
use pam_desktop_ui::modules::support::uuid;
use pam_desktop_ui::modules::{Module, Reply, request_map};
use pam_desktop_ui::wiremap::WireMap;
use tungstenite::{Error, Message, protocol::WebSocketConfig};

const MAX_CONNECTIONS: usize = 4;
const MAX_MESSAGE: usize = 64 * 1024;
const MAX_QUEUED_BYTES: usize = 256 * 1024;
const MAX_EVENTS: usize = 32;
const OPEN: i64 = 1;
const TEXT: i64 = 2;
const CLOSED: i64 = 3;
const FAILED: i64 = 4;

struct Event {
    kind: i64,
    text: String,
}
impl Event {
    fn map(&self) -> WireMap {
        let mut map = WireMap::new();
        map.insert("kind", self.kind);
        map.insert("text", self.text.as_str());
        map
    }
}
#[derive(Default)]
struct Queue {
    events: VecDeque<Event>,
    bytes: usize,
    waiter: Option<Reply>,
    terminal: bool,
}
#[derive(Default)]
struct Events(Mutex<Queue>);
impl Events {
    /// Overflow fails the stream visibly; it never silently drops messages.
    fn offer(&self, mut event: Event) -> bool {
        let (waiter, keep) = {
            let Ok(mut queue) = self.0.lock() else {
                return false;
            };
            if queue.terminal {
                return false;
            }
            let overflow = queue.events.len() >= MAX_EVENTS
                || queue.bytes.saturating_add(event.text.len()) > MAX_QUEUED_BYTES;
            if overflow {
                queue.events.clear();
                queue.bytes = 0;
                event = Event {
                    kind: FAILED,
                    text: "WebSocket receive queue exceeded its limit".into(),
                };
            }
            let terminal = matches!(event.kind, CLOSED | FAILED);
            queue.terminal = terminal;
            let waiter = queue.waiter.take();
            if waiter.is_none() {
                queue.bytes += event.text.len();
                queue.events.push_back(Event {
                    kind: event.kind,
                    text: std::mem::take(&mut event.text),
                });
            }
            (waiter, !terminal)
        };
        if let Some(waiter) = waiter {
            waiter.map(&event.map());
        }
        keep
    }
    fn next(&self, reply: Reply) {
        let event = {
            let Ok(mut queue) = self.0.lock() else {
                reply.failure("WebSocket queue unavailable");
                return;
            };
            if let Some(event) = queue.events.pop_front() {
                queue.bytes -= event.text.len();
                event
            } else if queue.terminal {
                reply.failure("WebSocket is closed");
                return;
            } else if queue.waiter.is_some() {
                reply.failure("WebSocket already has a pending read");
                return;
            } else {
                queue.waiter = Some(reply);
                return;
            }
        };
        reply.map(&event.map());
    }
    fn stop(&self) {
        let waiter = {
            let Ok(mut queue) = self.0.lock() else { return };
            queue.events.clear();
            queue.bytes = 0;
            queue.terminal = true;
            queue.waiter.take()
        };
        if let Some(reply) = waiter {
            reply.failure("WebSocket stopped");
        }
    }
}
struct Connection {
    stopped: Arc<AtomicBool>,
    socket: Arc<Mutex<Option<TcpStream>>>,
    events: Arc<Events>,
    sender: mpsc::SyncSender<String>,
}
impl Connection {
    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Ok(socket) = self.socket.lock()
            && let Some(socket) = socket.as_ref()
        {
            let _ = socket.shutdown(Shutdown::Both);
        }
        self.events.stop();
    }
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

pub struct WebSockets {
    connections: HashMap<String, Connection>,
    active: Arc<AtomicUsize>,
    development: bool,
}
impl WebSockets {
    pub fn new(development: bool) -> Self {
        Self {
            connections: HashMap::new(),
            active: Arc::new(AtomicUsize::new(0)),
            development,
        }
    }
    fn open(&mut self, data: &WireMap, reply: Reply) {
        let Some(url) = data.text("url") else {
            reply.failure("WebSocket URL required");
            return;
        };
        let Some(parsed) = parse_url(url) else {
            reply.failure("Invalid WebSocket URL");
            return;
        };
        if url.len() > 2048
            || url.contains('#')
            || parsed.credentials
            || parsed.port == Some(0)
            || !matches!(parsed.scheme.as_str(), "ws" | "wss")
            || (parsed.scheme == "ws" && !self.development && !parsed.is_loopback())
        {
            reply.failure("WebSocket requires a secure URL without credentials");
            return;
        }
        if self.connections.len() >= MAX_CONNECTIONS
            || self.active.load(Ordering::Acquire) >= MAX_CONNECTIONS
        {
            reply.failure("WebSocket connection limit reached");
            return;
        }
        let url = url.to_owned();
        let host = parsed.host;
        let port = parsed
            .port
            .unwrap_or(if parsed.scheme == "wss" { 443 } else { 80 });
        let id = uuid();
        let stopped = Arc::new(AtomicBool::new(false));
        let socket = Arc::new(Mutex::new(None));
        let events = Arc::new(Events::default());
        let (sender, receiver) = mpsc::sync_channel(4);
        self.connections.insert(
            id.clone(),
            Connection {
                stopped: Arc::clone(&stopped),
                socket: Arc::clone(&socket),
                events: Arc::clone(&events),
                sender,
            },
        );
        self.active.fetch_add(1, Ordering::AcqRel);
        let active = Active(Arc::clone(&self.active));
        let spawned = std::thread::Builder::new()
            .name("pam-websocket".into())
            .spawn(move || {
                let _active = active;
                let result = run(&url, &host, port, &stopped, &socket, &events, receiver);
                if !stopped.load(Ordering::Acquire)
                    && let Err(message) = result
                {
                    events.offer(Event {
                        kind: FAILED,
                        text: message.into(),
                    });
                }
                if let Ok(socket) = socket.lock()
                    && let Some(socket) = socket.as_ref()
                {
                    let _ = socket.shutdown(Shutdown::Both);
                }
            });
        if spawned.is_err() {
            if let Some(connection) = self.connections.remove(&id) {
                connection.stop();
            }
            reply.failure("WebSocket worker unavailable");
            return;
        }
        let mut result = WireMap::new();
        result.insert("id", id.as_str());
        reply.map(&result);
    }
}
impl Module for WebSockets {
    fn call(&mut self, method: &str, payload: &[u8], reply: Reply) {
        let data = match request_map(payload) {
            Ok(data) => data,
            Err(error) => {
                reply.failure(error);
                return;
            }
        };
        if method == "open" {
            self.open(&data, reply);
            return;
        }
        let id = data.text("id").unwrap_or("");
        if method == "close" {
            if let Some(connection) = self.connections.remove(id) {
                connection.stop();
            }
            reply.done();
            return;
        }
        let Some(connection) = self.connections.get(id) else {
            reply.failure("Unknown WebSocket");
            return;
        };
        match method {
            "next" => connection.events.next(reply),
            "send" => {
                let Some(text) = data.text("text") else {
                    reply.failure("WebSocket text required");
                    return;
                };
                if text.len() > MAX_MESSAGE {
                    reply.failure("WebSocket message exceeds 64 KiB");
                    return;
                }
                match connection.sender.try_send(text.into()) {
                    Ok(()) => reply.done(),
                    Err(_) => reply.failure("WebSocket send queue full or closed"),
                }
            }
            _ => reply.failure("Unknown WebSocket method"),
        }
    }
    fn reset(&mut self) {
        for (_, connection) in self.connections.drain() {
            connection.stop();
        }
    }
}
impl Drop for WebSockets {
    fn drop(&mut self) {
        self.reset();
    }
}

fn run(
    url: &str,
    host: &str,
    port: u16,
    stopped: &AtomicBool,
    shared: &Mutex<Option<TcpStream>>,
    events: &Events,
    receiver: mpsc::Receiver<String>,
) -> Result<(), &'static str> {
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|_| "WebSocket DNS lookup failed")?;
    let mut stream = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    for address in addresses.take(8) {
        if stopped.load(Ordering::Acquire) {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        if let Ok(socket) =
            TcpStream::connect_timeout(&address, remaining.min(Duration::from_secs(3)))
        {
            stream = Some(socket);
            break;
        }
    }
    let stream = stream.ok_or("WebSocket connection failed")?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| "WebSocket timeout setup failed")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| "WebSocket timeout setup failed")?;
    if let Ok(mut slot) = shared.lock() {
        *slot = Some(
            stream
                .try_clone()
                .map_err(|_| "WebSocket socket unavailable")?,
        );
    }
    if stopped.load(Ordering::Acquire) {
        return Ok(());
    }
    let config = WebSocketConfig::default()
        .read_buffer_size(4096)
        .write_buffer_size(0)
        .max_write_buffer_size(MAX_MESSAGE * 2)
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE));
    let (mut websocket, _) = tungstenite::client_tls_with_config(url, stream, Some(config), None)
        .map_err(|_| "WebSocket TLS or handshake failed")?;
    // A bounded blocking wait keeps all networking off the UI/PHP threads. Closing the
    // cloned descriptor interrupts it immediately; sends wait at most 200 ms.
    let tcp = match websocket.get_ref() {
        tungstenite::stream::MaybeTlsStream::Plain(s) => s,
        tungstenite::stream::MaybeTlsStream::Rustls(s) => &s.sock,
        _ => return Err("Unsupported WebSocket TLS transport"),
    };
    tcp.set_read_timeout(Some(Duration::from_millis(200)))
        .map_err(|_| "WebSocket timeout setup failed")?;
    if !events.offer(Event {
        kind: OPEN,
        text: String::new(),
    }) {
        return Ok(());
    }
    while !stopped.load(Ordering::Acquire) {
        while let Ok(text) = receiver.try_recv() {
            websocket
                .send(Message::Text(text.into()))
                .map_err(|_| "WebSocket send failed")?;
        }
        match websocket.read() {
            Ok(Message::Text(text)) => {
                if !events.offer(Event {
                    kind: TEXT,
                    text: text.to_string(),
                }) {
                    return Ok(());
                }
            }
            Ok(Message::Binary(_)) => {
                return Err("Binary WebSocket messages are not supported by the text transport");
            }
            Ok(Message::Ping(_)) => {
                websocket.flush().map_err(|_| "WebSocket pong failed")?;
            }
            Ok(Message::Close(_)) => {
                let _ = websocket.flush();
                events.offer(Event {
                    kind: CLOSED,
                    text: String::new(),
                });
                return Ok(());
            }
            Ok(_) => {}
            Err(Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(Error::ConnectionClosed) => {
                events.offer(Event {
                    kind: CLOSED,
                    text: String::new(),
                });
                return Ok(());
            }
            Err(_) => return Err("WebSocket receive failed"),
        }
    }
    let _ = websocket.close(None);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pam_desktop_ui::modules::ModuleResult;
    use std::net::TcpListener;
    use std::sync::mpsc::Receiver;
    fn call(
        module: &mut WebSockets,
        method: &str,
        fields: &[(&str, &str)],
    ) -> Receiver<ModuleResult> {
        let mut map = WireMap::new();
        for (key, value) in fields {
            map.insert(key, *value);
        }
        let (reply, receiver) = Reply::channel();
        module.call(method, &map.encode(), reply);
        receiver
    }
    fn answer(receiver: Receiver<ModuleResult>) -> WireMap {
        let (status, bytes) = receiver.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(
            status,
            pam_desktop_ui::runtime::MODULE_SUCCESS,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        WireMap::decode(&bytes).unwrap()
    }
    #[test]
    fn validates_transport_urls_without_echoing_sensitive_input() {
        let mut module = WebSockets::new(false);
        for url in [
            "https://example.test",
            "ws://example.test",
            "wss://user:secret@example.test",
            "wss://example.test/#secret",
            "wss://example.test:0",
        ] {
            let (status, bytes) = call(&mut module, "open", &[("url", url)]).recv().unwrap();
            assert_eq!(status, pam_desktop_ui::runtime::MODULE_FAILURE);
            assert!(!String::from_utf8_lossy(&bytes).contains("secret"));
        }
        assert_eq!(module.active.load(Ordering::Acquire), 0);
    }
    #[test]
    fn queue_overflow_is_terminal_instead_of_losing_business_events() {
        let events = Events::default();
        for _ in 0..MAX_EVENTS {
            assert!(events.offer(Event {
                kind: TEXT,
                text: "data".into()
            }));
        }
        assert!(!events.offer(Event {
            kind: TEXT,
            text: "overflow".into()
        }));
        let (reply, receiver) = Reply::channel();
        events.next(reply);
        let event = answer(receiver);
        assert_eq!(event.integer("kind"), Some(FAILED));
        assert!(!events.offer(Event {
            kind: TEXT,
            text: "late".into()
        }));
    }
    #[test]
    fn real_loopback_handshake_send_receive_close_and_reset() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("ws://{}/socket", server.local_addr().unwrap());
        let peer = std::thread::spawn(move || {
            let (stream, _) = server.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            assert_eq!(ws.read().unwrap().into_text().unwrap(), "hello");
            ws.send(Message::Text("world".into())).unwrap();
            let _ = ws.read();
        });
        let mut module = WebSockets::new(false);
        let opened = answer(call(&mut module, "open", &[("url", &url)]));
        let id = opened.text("id").unwrap();
        assert_eq!(
            answer(call(&mut module, "next", &[("id", id)])).integer("kind"),
            Some(OPEN)
        );
        let (status, _) = call(&mut module, "send", &[("id", id), ("text", "hello")])
            .recv()
            .unwrap();
        assert_eq!(status, pam_desktop_ui::runtime::MODULE_SUCCESS);
        let message = answer(call(&mut module, "next", &[("id", id)]));
        assert_eq!(message.text("text"), Some("world"));
        let parked = call(&mut module, "next", &[("id", id)]);
        module.reset();
        assert_eq!(
            parked.recv_timeout(Duration::from_secs(1)).unwrap().0,
            pam_desktop_ui::runtime::MODULE_FAILURE
        );
        peer.join().unwrap();
        for _ in 0..100 {
            if module.active.load(Ordering::Acquire) == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(module.active.load(Ordering::Acquire), 0);
        assert!(module.connections.is_empty());
    }
}
