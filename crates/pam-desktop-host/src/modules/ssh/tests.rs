//! Module-level tests. The end-to-end test needs a real OpenSSH server and is
//! ignored by default; see `docs/ssh.md` for the container it expects.

use std::net::TcpListener;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pam_desktop_ui::modules::terminal::{TerminalEvent, TerminalHub};
use pam_desktop_ui::modules::{Module, ModuleResult, Reply};
use pam_desktop_ui::runtime::{MODULE_FAILURE, MODULE_SUCCESS};
use pam_desktop_ui::wiremap::{WireMap, WireValue};

use super::events::{
    FORWARD_CONNECTIONS, FORWARD_OPENED, SESSION_CLOSED, SHELL_DATA, SHELL_EXIT, TRANSFER_DONE,
    TRANSFER_PROGRESS,
};
use super::{MAX_BYTES_VALUE, Permit, Ssh};

fn hub() -> &'static TerminalHub {
    Box::leak(Box::new(TerminalHub::new()))
}

fn send(module: &mut Ssh, method: &str, map: &WireMap) -> Receiver<ModuleResult> {
    let (reply, receiver) = Reply::channel();
    module.call(method, &map.encode(), reply);
    receiver
}

fn wait(receiver: &Receiver<ModuleResult>, seconds: u64) -> (u8, Vec<u8>) {
    receiver
        .recv_timeout(Duration::from_secs(seconds))
        .expect("the module answers")
}

fn ok(module: &mut Ssh, method: &str, map: &WireMap) -> WireMap {
    let (status, bytes) = wait(&send(module, method, map), 30);
    assert_eq!(
        status,
        MODULE_SUCCESS,
        "{method}: {}",
        String::from_utf8_lossy(&bytes)
    );
    if bytes.is_empty() {
        WireMap::new()
    } else {
        WireMap::decode(&bytes).unwrap()
    }
}

fn failure(module: &mut Ssh, method: &str, map: &WireMap) -> String {
    let (status, bytes) = wait(&send(module, method, map), 30);
    assert_eq!(status, MODULE_FAILURE, "{method} should fail");
    String::from_utf8(bytes).unwrap()
}

fn map(fields: &[(&str, &str)]) -> WireMap {
    let mut map = WireMap::new();
    for (key, value) in fields {
        map.insert(key, *value);
    }
    map
}

fn wait_until(mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn calls_without_a_session_fail_and_nothing_starts() {
    let mut module = Ssh::with_hub(hub());
    for method in [
        "exec",
        "next",
        "shellOpen",
        "forwardLocal",
        "sftpList",
        "sftpRead",
    ] {
        let message = failure(&mut module, method, &map(&[("id", "missing")]));
        assert!(message.contains("unknown session"), "{method}: {message}");
    }
    assert!(failure(&mut module, "nope", &WireMap::new()).contains("unknown method"));
    assert!(failure(&mut module, "sftpCancel", &map(&[("transfer", "x")])).contains("unknown"));
    assert!(
        failure(
            &mut module,
            "shellWrite",
            &map(&[("channel", "x"), ("bytes", "a")])
        )
        .contains("unknown")
    );
    ok(&mut module, "disconnect", &map(&[("id", "missing")]));
    assert!(!module.shared.runtime.is_running());
}

#[test]
fn a_refused_connection_fails_and_the_runtime_stops_again() {
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let mut module = Ssh::with_hub(hub());
    let mut request = map(&[
        ("host", "127.0.0.1"),
        ("username", "nobody"),
        ("password", "secret"),
    ]);
    request.insert("port", i64::from(port));
    let message = failure(&mut module, "connect", &request);
    assert!(message.starts_with("connect-failed"), "{message}");
    assert!(!message.contains("secret") && !message.contains("127.0.0.1"));
    assert!(wait_until(|| !module.shared.runtime.is_running()));
    assert_eq!(
        module
            .shared
            .active
            .load(std::sync::atomic::Ordering::Acquire),
        0
    );
}

#[test]
fn a_server_that_never_answers_times_out() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut module = Ssh::with_hub(hub());
    let mut request = map(&[("host", "127.0.0.1"), ("username", "u"), ("password", "p")]);
    request.insert("port", i64::from(port));
    request.insert("timeoutMs", 1000_i64);
    assert_eq!(failure(&mut module, "connect", &request), "timeout");
    drop(listener);
    assert!(wait_until(|| !module.shared.runtime.is_running()));
}

#[test]
fn permits_bound_their_counter() {
    let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let first = Permit::acquire(&counter, 2).unwrap();
    let _second = Permit::acquire(&counter, 2).unwrap();
    assert!(Permit::acquire(&counter, 2).is_none());
    drop(first);
    assert!(Permit::acquire(&counter, 2).is_some());
}

/// Reads one session's events, keeping a parked `next` across timeouts.
struct Pump {
    id: String,
    pending: Option<Receiver<ModuleResult>>,
}

impl Pump {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            pending: None,
        }
    }

    fn next(&mut self, module: &mut Ssh, timeout: Duration) -> Option<WireMap> {
        let receiver = self
            .pending
            .take()
            .unwrap_or_else(|| send(module, "next", &map(&[("id", &self.id)])));
        match receiver.recv_timeout(timeout) {
            Ok((status, bytes)) => {
                assert_eq!(
                    status,
                    MODULE_SUCCESS,
                    "{}",
                    String::from_utf8_lossy(&bytes)
                );
                Some(WireMap::decode(&bytes).unwrap())
            }
            Err(_) => {
                self.pending = Some(receiver);
                None
            }
        }
    }

    /// Events until `done` accepts one, or panics after 20 seconds.
    #[track_caller]
    fn until(&mut self, module: &mut Ssh, mut done: impl FnMut(&WireMap) -> bool) -> Vec<WireMap> {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut seen = Vec::new();
        while Instant::now() < deadline {
            if let Some(event) = self.next(module, Duration::from_millis(200)) {
                let stop = done(&event);
                seen.push(event);
                if stop {
                    return seen;
                }
            }
        }
        panic!("event not seen; got {} events", seen.len());
    }
}

struct Server {
    host: String,
    port: i64,
    user: String,
    password: String,
    key: String,
}

fn server() -> Option<Server> {
    let port = std::env::var("PAM_SSH_TEST_PORT").ok()?.parse().ok()?;
    let key = std::env::var("PAM_SSH_TEST_KEY").ok()?;
    Some(Server {
        host: std::env::var("PAM_SSH_TEST_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
        port,
        user: std::env::var("PAM_SSH_TEST_USER").unwrap_or_else(|_| "pam".into()),
        password: std::env::var("PAM_SSH_TEST_PASSWORD").ok()?,
        key: std::fs::read_to_string(key).ok()?,
    })
}

fn connect_request(server: &Server, fields: &[(&str, &str)]) -> WireMap {
    let mut request = map(&[("host", &server.host), ("username", &server.user)]);
    request.insert("port", server.port);
    for (key, value) in fields {
        request.insert(key, *value);
    }
    request
}

fn text(map: &WireMap, key: &str) -> String {
    String::from_utf8_lossy(map.bytes(key).unwrap_or_default()).into_owned()
}

/// Run with a server from `docs/ssh.md`:
/// `PAM_SSH_TEST_PORT=2222 PAM_SSH_TEST_PASSWORD=… PAM_SSH_TEST_KEY=… cargo test -p pam-desktop-host ssh -- --ignored`
#[test]
#[ignore = "needs an OpenSSH server (PAM_SSH_TEST_PORT, PAM_SSH_TEST_PASSWORD, PAM_SSH_TEST_KEY)"]
#[allow(clippy::too_many_lines)]
fn end_to_end_against_openssh() {
    let Some(server) = server() else {
        panic!("PAM_SSH_TEST_PORT, PAM_SSH_TEST_PASSWORD and PAM_SSH_TEST_KEY are required");
    };
    let hub = hub();
    let mut module = Ssh::with_hub(hub);

    // Authentication failures and the host key policy.
    let message = failure(
        &mut module,
        "connect",
        &connect_request(&server, &[("password", "wrong-password")]),
    );
    assert!(message.starts_with("auth-failed"), "{message}");
    assert!(!message.contains("wrong-password"));
    let first = ok(
        &mut module,
        "connect",
        &connect_request(&server, &[("password", &server.password)]),
    );
    let fingerprint = first.text("fingerprint").unwrap().to_owned();
    assert_eq!(fingerprint.len(), 43, "SHA-256 in unpadded base64");
    assert!(!first.text("algorithm").unwrap().is_empty());
    let message = failure(
        &mut module,
        "connect",
        &connect_request(
            &server,
            &[
                ("password", &server.password),
                (
                    "expectedFingerprint",
                    "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                ),
            ],
        ),
    );
    assert_eq!(message, format!("host-key-mismatch:{fingerprint}"));
    let pinned = format!("SHA256:{fingerprint}");
    let keyed = ok(
        &mut module,
        "connect",
        &connect_request(
            &server,
            &[
                ("privateKey", &server.key),
                ("expectedFingerprint", &pinned),
            ],
        ),
    );
    let id = keyed.text("id").unwrap().to_owned();
    let mut pump = Pump::new(&id);
    let session = |extra: &[(&str, &str)]| {
        let mut request = map(&[("id", id.as_str())]);
        for (key, value) in extra {
            request.insert(key, *value);
        }
        request
    };
    let first_id = first.text("id").unwrap().to_owned();
    ok(&mut module, "disconnect", &map(&[("id", &first_id)]));

    // exec
    let result = ok(
        &mut module,
        "exec",
        &session(&[("command", "echo hi; echo err >&2; exit 3")]),
    );
    assert_eq!(result.integer("exitCode"), Some(3));
    assert_eq!(text(&result, "stdout"), "hi\n");
    assert_eq!(text(&result, "stderr"), "err\n");
    assert_eq!(result.boolean("truncated"), Some(false));
    let mut capped = session(&[("command", "head -c 100000 /dev/zero")]);
    capped.insert("maxBytes", 10_i64);
    let result = ok(&mut module, "exec", &capped);
    assert_eq!(result.bytes("stdout").unwrap().len(), 10);
    assert_eq!(result.boolean("truncated"), Some(true));
    let mut slow = session(&[("command", "sleep 5")]);
    slow.insert("timeoutMs", 300_i64);
    assert_eq!(failure(&mut module, "exec", &slow), "exec-timeout");

    // SFTP
    let home = ok(&mut module, "sftpRealpath", &session(&[("path", "~")]))
        .text("path")
        .unwrap()
        .to_owned();
    assert!(home.starts_with('/'));
    let base = format!("{home}/pam-ssh-test");
    let mut recursive = session(&[("path", &base)]);
    recursive.insert("recursive", true);
    let _ = wait(&send(&mut module, "sftpRemove", &recursive), 30);
    ok(&mut module, "sftpMkdir", &session(&[("path", &base)]));
    let mut write = session(&[("path", &format!("{base}/hello.txt"))]);
    let content: Vec<u8> = (0..=255_u8).cycle().take(100_000).collect();
    write.insert("bytes", WireValue::Bytes(content.clone()));
    write.insert("mode", 0o640_i64);
    assert_eq!(
        ok(&mut module, "sftpWrite", &write).integer("bytes"),
        Some(100_000)
    );
    let read = ok(
        &mut module,
        "sftpRead",
        &session(&[("path", &format!("{base}/hello.txt"))]),
    );
    assert_eq!(read.bytes("bytes").unwrap(), content.as_slice());
    assert_eq!(read.boolean("truncated"), Some(false));
    let mut partial = session(&[("path", &format!("{base}/hello.txt"))]);
    partial.insert("maxBytes", 1000_i64);
    let read = ok(&mut module, "sftpRead", &partial);
    assert_eq!(read.bytes("bytes").unwrap(), &content[..1000]);
    assert_eq!(read.boolean("truncated"), Some(true));
    assert_eq!(read.integer("size"), Some(100_000));
    ok(
        &mut module,
        "sftpRename",
        &session(&[
            ("from", &format!("{base}/hello.txt")),
            ("to", &format!("{base}/renamed.txt")),
        ]),
    );
    let stat = ok(
        &mut module,
        "sftpStat",
        &session(&[("path", &format!("{base}/renamed.txt"))]),
    );
    assert_eq!(stat.text("kind"), Some("file"));
    assert_eq!(stat.text("permissions"), Some("0640"));
    assert_eq!(stat.integer("size"), Some(100_000));
    ok(
        &mut module,
        "sftpMkdir",
        &session(&[("path", &format!("{base}/sub"))]),
    );
    let listing = ok(&mut module, "sftpList", &session(&[("path", &base)]));
    let entries: serde_json::Value =
        serde_json::from_str(listing.text("entries").unwrap()).unwrap();
    let mut names: Vec<(String, String)> = entries
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["name"].as_str().unwrap().to_owned(),
                entry["kind"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            ("renamed.txt".to_owned(), "file".to_owned()),
            ("sub".to_owned(), "dir".to_owned()),
        ]
    );
    assert!(entries[0]["owner"].is_string());
    assert_eq!(listing.boolean("truncated"), Some(false));
    assert!(
        failure(
            &mut module,
            "sftpStat",
            &session(&[("path", &format!("{base}/missing"))])
        )
        .starts_with("sftp: no-such-file")
    );

    // Upload and download with progress.
    let local = std::env::temp_dir().join(format!("pam-ssh-{}", std::process::id()));
    std::fs::create_dir_all(&local).unwrap();
    let upload_path = local.join("upload.bin");
    let big: Vec<u8> = (0..3 * 1024 * 1024_u32)
        .map(|index| (index.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    std::fs::write(&upload_path, &big).unwrap();
    let transfer = ok(
        &mut module,
        "sftpUpload",
        &session(&[
            ("localPath", upload_path.to_str().unwrap()),
            ("remotePath", &format!("{base}/sub/big.bin")),
        ]),
    )
    .text("transfer")
    .unwrap()
    .to_owned();
    let events = pump.until(&mut module, |event| {
        event.integer("kind") == Some(TRANSFER_DONE) || event.integer("kind") == Some(5)
    });
    let done = events.last().unwrap();
    assert_eq!(
        done.integer("kind"),
        Some(TRANSFER_DONE),
        "{:?}",
        done.text("message")
    );
    assert_eq!(done.text("transfer"), Some(transfer.as_str()));
    assert_eq!(done.integer("bytes"), Some(big.len() as i64));
    let progress: Vec<&WireMap> = events
        .iter()
        .filter(|event| event.integer("kind") == Some(TRANSFER_PROGRESS))
        .collect();
    assert!(!progress.is_empty());
    assert!(
        progress
            .iter()
            .all(|event| event.integer("total") == Some(big.len() as i64))
    );
    let download_path = local.join("download.bin");
    ok(
        &mut module,
        "sftpDownload",
        &session(&[
            ("remotePath", &format!("{base}/sub/big.bin")),
            ("localPath", download_path.to_str().unwrap()),
        ]),
    );
    let events = pump.until(&mut module, |event| {
        matches!(event.integer("kind"), Some(TRANSFER_DONE | 5))
    });
    assert_eq!(events.last().unwrap().integer("kind"), Some(TRANSFER_DONE));
    assert_eq!(std::fs::read(&download_path).unwrap(), big);
    assert!(!local.join("download.bin.part").exists());
    let missing = ok(
        &mut module,
        "sftpDownload",
        &session(&[
            ("remotePath", &format!("{base}/nope.bin")),
            ("localPath", local.join("nope.bin").to_str().unwrap()),
        ]),
    );
    let events = pump.until(&mut module, |event| event.integer("kind") == Some(5));
    assert_eq!(
        events.last().unwrap().text("transfer"),
        missing.text("transfer")
    );
    assert_eq!(
        events.last().unwrap().text("message"),
        Some("sftp: no-such-file")
    );
    assert!(!local.join("nope.bin.part").exists());
    let _ = std::fs::remove_dir_all(&local);
    let mut recursive = session(&[("path", &base)]);
    recursive.insert("recursive", true);
    ok(&mut module, "sftpRemove", &recursive);
    assert!(
        failure(&mut module, "sftpStat", &session(&[("path", &base)]))
            .starts_with("sftp: no-such-file")
    );

    // Local forward to the server's own sshd.
    let mut forward = session(&[("remoteHost", "127.0.0.1")]);
    forward.insert("bindPort", 0_i64);
    forward.insert("remotePort", 22_i64);
    assert!(
        failure(&mut module, "forwardLocal", &{
            let mut wide = forward.clone();
            wide.insert("bindHost", "0.0.0.0");
            wide
        })
        .contains("loopback")
    );
    let opened = ok(&mut module, "forwardLocal", &forward);
    let forward_id = opened.text("forward").unwrap().to_owned();
    let port = opened.integer("port").unwrap();
    assert!(port > 0);
    pump.until(&mut module, |event| {
        event.integer("kind") == Some(FORWARD_OPENED)
    });
    {
        use std::io::Read;
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port as u16)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut banner = [0_u8; 8];
        stream.read_exact(&mut banner).unwrap();
        assert_eq!(&banner, b"SSH-2.0-");
    }
    pump.until(&mut module, |event| {
        event.integer("kind") == Some(FORWARD_CONNECTIONS)
            && event.integer("connections") == Some(0)
    });
    ok(
        &mut module,
        "forwardStop",
        &map(&[("forward", &forward_id)]),
    );
    assert!(wait_until(|| std::net::TcpStream::connect((
        "127.0.0.1",
        port as u16
    ))
    .is_err()));

    // Shell through PHP's events, then through the hub.
    let mut open = session(&[]);
    open.insert("cols", 100_i64);
    open.insert("rows", 30_i64);
    let channel = ok(&mut module, "shellOpen", &open)
        .text("channel")
        .unwrap()
        .to_owned();
    let mut output = String::new();
    let shell_write = |module: &mut Ssh, text: &str| {
        let mut request = map(&[("channel", channel.as_str())]);
        request.insert("bytes", WireValue::Bytes(text.as_bytes().to_vec()));
        ok(module, "shellWrite", &request);
    };
    shell_write(&mut module, "echo pam-$((40+2))\n");
    pump.until(&mut module, |event| {
        if event.integer("kind") == Some(SHELL_DATA) {
            output.push_str(&text(event, "bytes"));
        }
        output.contains("pam-42")
    });
    let mut resize = map(&[("channel", channel.as_str())]);
    resize.insert("cols", 120_i64);
    resize.insert("rows", 40_i64);
    ok(&mut module, "shellResize", &resize);
    std::thread::sleep(Duration::from_millis(200));
    shell_write(&mut module, "stty size\n");
    let mut output = String::new();
    pump.until(&mut module, |event| {
        if event.integer("kind") == Some(SHELL_DATA) {
            output.push_str(&text(event, "bytes"));
        }
        output.contains("40 120")
    });

    // The prompt that followed is still on its way to PHP; let it arrive.
    while pump.next(&mut module, Duration::from_millis(300)).is_some() {}
    let native = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&native);
    let subscription = hub
        .subscribe(
            &channel,
            Arc::new(move |event| {
                if let TerminalEvent::Output(bytes) = event {
                    sink.lock().unwrap().extend_from_slice(bytes);
                }
            }),
        )
        .unwrap();
    assert!(!subscription.backlog().is_empty());
    assert!(hub.write(&channel, b"echo native-$((6*7))\n"));
    assert!(wait_until(|| String::from_utf8_lossy(
        &native.lock().unwrap()
    )
    .contains("native-42")));
    assert!(
        pump.next(&mut module, Duration::from_millis(500)).is_none(),
        "PHP gets nothing while subscribed"
    );
    drop(subscription);

    ok(&mut module, "shellClose", &map(&[("channel", &channel)]));
    let events = pump.until(&mut module, |event| {
        event.integer("kind") == Some(SHELL_EXIT)
    });
    assert_eq!(
        events.last().unwrap().text("channel"),
        Some(channel.as_str())
    );
    assert!(wait_until(|| !hub.contains(&channel)));

    // A request larger than the bridge allows is refused before the network.
    let mut huge = session(&[("path", "/tmp/x")]);
    huge.insert("maxBytes", (MAX_BYTES_VALUE + 1) as i64);
    assert!(failure(&mut module, "sftpRead", &huge).contains("maxBytes"));

    // Disconnect: the parked next hears the end, then the runtime stops.
    assert!(pump.next(&mut module, Duration::from_millis(100)).is_none());
    ok(&mut module, "disconnect", &map(&[("id", &id)]));
    pump.until(&mut module, |event| {
        event.integer("kind") == Some(SESSION_CLOSED)
    });
    assert!(wait_until(|| !module.shared.runtime.is_running()));
    assert!(super::lock(&module.shared.sessions).is_empty());
    assert!(super::lock(&module.shared.shells).is_empty());
    assert!(super::lock(&module.shared.forwards).is_empty());
    assert!(super::lock(&module.shared.transfers).is_empty());
}
