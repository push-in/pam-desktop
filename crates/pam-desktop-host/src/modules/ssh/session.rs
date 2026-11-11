//! Connecting, authenticating, the host key policy and the life of a session.

use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pam_desktop_ui::modules::Reply;
use pam_desktop_ui::modules::support::uuid;
use pam_desktop_ui::wiremap::WireMap;
use russh::client::{self, AuthResult, DisconnectReason, KeyboardInteractiveAuthResponse};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate, decode_secret_key};
use russh::{Channel, Disconnect};
use russh_sftp::client::RawSftpSession;
use tokio::sync::{oneshot, watch};

use super::events::{Event, Events};
use super::runtime::Lease;
use super::{MAX_SESSIONS, Permit, Shared, Slot, input, lock};

const MAX_SECRET: usize = 4096;
const MAX_PRIVATE_KEY: usize = 64 * 1024;

/// What the server presented.
#[derive(Clone, Debug, Default)]
struct HostKey {
    fingerprint: String,
    algorithm: String,
}

/// The SHA-256 fingerprint of a key, base64 without padding or prefix (what
/// `ssh-keygen -l` prints after `SHA256:`).
pub fn fingerprint(key: &russh::keys::ssh_key::public::KeyData) -> String {
    let text = key.fingerprint(HashAlg::Sha256).to_string();
    text.strip_prefix("SHA256:").unwrap_or(&text).to_owned()
}

/// Accepts `SHA256:` and trailing `=` in an expected fingerprint.
pub fn normalize_fingerprint(text: &str) -> String {
    text.trim()
        .trim_start_matches("SHA256:")
        .trim_end_matches('=')
        .to_owned()
}

enum Ended {
    Closed,
    Failed(&'static str),
}

pub struct Client {
    expected: Option<String>,
    seen: Arc<Mutex<Option<HostKey>>>,
    ended: Option<oneshot::Sender<Ended>>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let presented = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => HostKey {
                fingerprint: fingerprint(key.key_data()),
                algorithm: key.algorithm().to_string(),
            },
            PublicKeyOrCertificate::Certificate(certificate) => HostKey {
                fingerprint: fingerprint(certificate.public_key()),
                algorithm: certificate.algorithm().to_string(),
            },
        };
        let accepted = self
            .expected
            .as_deref()
            .is_none_or(|expected| expected == presented.fingerprint);
        *lock(&self.seen) = Some(presented);
        Ok(accepted)
    }

    async fn disconnected(
        &mut self,
        reason: DisconnectReason<Self::Error>,
    ) -> Result<(), Self::Error> {
        let ended = match &reason {
            DisconnectReason::ReceivedDisconnect(_) => Ended::Closed,
            DisconnectReason::Error(_) => Ended::Failed("connection-lost"),
        };
        if let Some(sender) = self.ended.take() {
            let _ = sender.send(ended);
        }
        match reason {
            DisconnectReason::ReceivedDisconnect(_) => Ok(()),
            DisconnectReason::Error(error) => Err(error),
        }
    }
}

/// An authenticated session.
pub struct Connection {
    pub id: String,
    pub handle: client::Handle<Client>,
    pub events: Arc<Events>,
    pub runtime: tokio::runtime::Handle,
    stop: watch::Sender<bool>,
    sftp: tokio::sync::Mutex<Option<Arc<RawSftpSession>>>,
    pub execs: Arc<AtomicUsize>,
    pub shells: Arc<AtomicUsize>,
    pub forwards: Arc<AtomicUsize>,
    pub transfers: Arc<AtomicUsize>,
}

impl Connection {
    /// Asks the session to end; its supervisor says goodbye to the server.
    pub fn stop(&self) {
        self.stop.send_replace(true);
    }

    /// Completes when the session is told to stop.
    pub async fn stopping(&self) {
        let mut receiver = self.stop.subscribe();
        let _ = receiver.wait_for(|stop| *stop).await;
    }

    pub async fn open_channel(&self) -> Result<Channel<client::Msg>, String> {
        tokio::select! {
            channel = self.handle.channel_open_session() => {
                channel.map_err(|error| channel_error(&error))
            }
            () = self.stopping() => Err("ssh: session closed".into()),
        }
    }

    /// The SFTP subsystem of the session, opened on first use and reused.
    pub async fn sftp(&self) -> Result<Arc<RawSftpSession>, String> {
        let mut slot = self.sftp.lock().await;
        if let Some(sftp) = slot.as_ref() {
            return Ok(Arc::clone(sftp));
        }
        let channel = self.open_channel().await?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|_| "sftp: subsystem unavailable")?;
        let config = russh_sftp::client::Config {
            request_timeout_secs: 30,
            ..russh_sftp::client::Config::default()
        };
        let sftp = RawSftpSession::new_with_config(channel.into_stream(), config);
        tokio::time::timeout(Duration::from_secs(15), sftp.init())
            .await
            .map_err(|_| "sftp: subsystem timed out")?
            .map_err(|_| "sftp: subsystem unavailable")?;
        let sftp = Arc::new(sftp);
        *slot = Some(Arc::clone(&sftp));
        Ok(sftp)
    }

    /// Forgets an SFTP subsystem whose channel broke, so the next call opens
    /// a new one.
    pub async fn sftp_broken(&self, broken: &Arc<RawSftpSession>) {
        let mut slot = self.sftp.lock().await;
        if slot.as_ref().is_some_and(|sftp| Arc::ptr_eq(sftp, broken)) {
            *slot = None;
        }
    }
}

pub fn channel_error(error: &russh::Error) -> String {
    match error {
        russh::Error::ChannelOpenFailure(reason) => {
            format!("ssh: channel refused ({reason:?})")
        }
        _ => "ssh: channel unavailable".into(),
    }
}

struct Options {
    host: String,
    port: u16,
    username: String,
    password: Option<String>,
    private_key: Option<String>,
    passphrase: Option<String>,
    expected: Option<String>,
    timeout: Duration,
    keepalive: Option<Duration>,
}

fn options(data: &WireMap) -> Result<Options, String> {
    let host = input::host(data, "host")?;
    let port = u16::try_from(input::integer(data, "port", 22, 1, 65535)?).unwrap_or(22);
    let username = input::required(data, "username", 256)?;
    if username.chars().any(char::is_control) {
        return Err("ssh: username is invalid".into());
    }
    let password = input::text(data, "password", MAX_SECRET)?.map(str::to_owned);
    let private_key = input::text(data, "privateKey", MAX_PRIVATE_KEY)?.map(str::to_owned);
    let passphrase = input::text(data, "passphrase", MAX_SECRET)?.map(str::to_owned);
    if password.is_none() && private_key.is_none() {
        return Err("ssh: password or privateKey required".into());
    }
    let expected = match input::text(data, "expectedFingerprint", 128)? {
        Some(text) => {
            let normalized = normalize_fingerprint(text);
            if normalized.is_empty()
                || !normalized
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/')
            {
                return Err("ssh: expectedFingerprint is invalid".into());
            }
            Some(normalized)
        }
        None => None,
    };
    let timeout = input::integer(data, "timeoutMs", 15_000, 1_000, 120_000)?;
    let keepalive = input::integer(data, "keepaliveSeconds", 30, 0, 3600)?;
    Ok(Options {
        host,
        port,
        username: username.to_owned(),
        password,
        private_key,
        passphrase,
        expected,
        timeout: Duration::from_millis(u64::try_from(timeout).unwrap_or(15_000)),
        keepalive: (keepalive > 0)
            .then(|| Duration::from_secs(u64::try_from(keepalive).unwrap_or(30))),
    })
}

/// `connect`: validates, then connects and authenticates on the runtime.
pub fn connect(shared: &Arc<Shared>, data: &WireMap, reply: Reply) {
    let options = match options(data) {
        Ok(options) => options,
        Err(error) => {
            reply.failure(error);
            return;
        }
    };
    let Some(permit) = Permit::acquire(&shared.active, MAX_SESSIONS) else {
        reply.failure("ssh: session limit reached");
        return;
    };
    let lease = match shared.runtime.lease() {
        Ok(lease) => lease,
        Err(error) => {
            reply.failure(error);
            return;
        }
    };
    let shared = Arc::clone(shared);
    let runtime = lease.handle().clone();
    runtime.spawn(async move {
        let seen = Arc::new(Mutex::new(None));
        let (ended, ended_receiver) = oneshot::channel();
        let expected = options.expected.clone();
        let timeout = options.timeout;
        let attempt = establish(options, Arc::clone(&seen), ended);
        let result = match tokio::time::timeout(timeout, attempt).await {
            Ok(result) => result,
            Err(_) => Err("timeout".into()),
        };
        let handle = match result {
            Ok(handle) => handle,
            Err(error) => {
                let seen = lock(&seen).clone();
                let message = match (expected, seen) {
                    (Some(expected), Some(seen)) if expected != seen.fingerprint => {
                        format!("host-key-mismatch:{}", seen.fingerprint)
                    }
                    _ => error,
                };
                reply.failure(message);
                return;
            }
        };
        let host_key = lock(&seen).clone().unwrap_or_default();
        let id = uuid();
        let events = Arc::new(Events::default());
        let (stop, _) = watch::channel(false);
        let connection = Arc::new(Connection {
            id: id.clone(),
            handle,
            events: Arc::clone(&events),
            runtime: tokio::runtime::Handle::current(),
            stop,
            sftp: tokio::sync::Mutex::new(None),
            execs: Arc::new(AtomicUsize::new(0)),
            shells: Arc::new(AtomicUsize::new(0)),
            forwards: Arc::new(AtomicUsize::new(0)),
            transfers: Arc::new(AtomicUsize::new(0)),
        });
        lock(&shared.sessions).insert(
            id.clone(),
            Slot {
                events,
                live: Some(Arc::clone(&connection)),
            },
        );
        let mut result = WireMap::new();
        result.insert("id", id.as_str());
        result.insert("fingerprint", host_key.fingerprint.as_str());
        result.insert("algorithm", host_key.algorithm.as_str());
        reply.map(&result);
        supervise(shared, connection, ended_receiver, lease, permit).await;
    });
}

async fn establish(
    options: Options,
    seen: Arc<Mutex<Option<HostKey>>>,
    ended: oneshot::Sender<Ended>,
) -> Result<client::Handle<Client>, String> {
    let config = Arc::new(client::Config {
        keepalive_interval: options.keepalive,
        keepalive_max: 3,
        nodelay: true,
        ..client::Config::default()
    });
    let client = Client {
        expected: options.expected.clone(),
        seen,
        ended: Some(ended),
    };
    let stream = tokio::net::TcpStream::connect((options.host.as_str(), options.port))
        .await
        .map_err(|error| {
            match error.kind() {
                std::io::ErrorKind::ConnectionRefused => "connect-failed: refused",
                std::io::ErrorKind::TimedOut => "connect-failed: timeout",
                _ if error.raw_os_error().is_none() => "connect-failed: host not found",
                _ => "connect-failed: unreachable",
            }
            .to_owned()
        })?;
    let _ = stream.set_nodelay(true);
    let mut handle = client::connect_stream(config, stream, client)
        .await
        .map_err(|error| match error {
            russh::Error::UnknownKey => "host-key-rejected".to_owned(),
            russh::Error::NoCommonAlgo { .. } => "handshake-failed: no common algorithm".into(),
            _ => "handshake-failed".into(),
        })?;
    authenticate(&mut handle, &options).await?;
    Ok(handle)
}

async fn authenticate(
    handle: &mut client::Handle<Client>,
    options: &Options,
) -> Result<(), String> {
    let user = options.username.as_str();
    if let Some(text) = &options.private_key {
        let key = decode_secret_key(text, options.passphrase.as_deref()).map_err(|error| {
            match error {
                russh::keys::Error::KeyIsEncrypted => "invalid-key: passphrase required",
                _ => "invalid-key: wrong passphrase or unsupported format",
            }
            .to_owned()
        })?;
        let hash = if key.algorithm().is_rsa() {
            handle
                .best_supported_rsa_hash()
                .await
                .map_err(|_| "auth-failed")?
                .flatten()
        } else {
            None
        };
        let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
        let result = handle
            .authenticate_publickey(user, key)
            .await
            .map_err(|_| "auth-failed")?;
        if matches!(result, AuthResult::Success) {
            return Ok(());
        }
    }
    if let Some(password) = &options.password {
        let result = handle
            .authenticate_password(user, password)
            .await
            .map_err(|_| "auth-failed")?;
        if matches!(result, AuthResult::Success) {
            return Ok(());
        }
        // Servers that only take passwords through keyboard-interactive.
        let mut response = handle
            .authenticate_keyboard_interactive_start(user, None)
            .await
            .map_err(|_| "auth-failed")?;
        for _ in 0..4 {
            match response {
                KeyboardInteractiveAuthResponse::Success => return Ok(()),
                KeyboardInteractiveAuthResponse::Failure { .. } => break,
                KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                    let answers = prompts
                        .iter()
                        .map(|prompt| {
                            if prompt.echo {
                                String::new()
                            } else {
                                password.clone()
                            }
                        })
                        .collect();
                    response = handle
                        .authenticate_keyboard_interactive_respond(answers)
                        .await
                        .map_err(|_| "auth-failed")?;
                }
            }
        }
    }
    Err("auth-failed".into())
}

/// Waits for the end of the session, then cleans up after it.
async fn supervise(
    shared: Arc<Shared>,
    connection: Arc<Connection>,
    ended: oneshot::Receiver<Ended>,
    lease: Lease,
    permit: Permit,
) {
    let outcome = tokio::select! {
        () = connection.stopping() => None,
        ended = ended => Some(ended.unwrap_or(Ended::Closed)),
        () = connection.events.overflowed() => Some(Ended::Failed("ssh event queue exceeded its limit")),
    };
    connection.stop();
    shared.ended(&connection.id);
    if outcome.is_none() || matches!(outcome, Some(Ended::Failed(_))) {
        let goodbye = connection
            .handle
            .disconnect(Disconnect::ByApplication, "", "en");
        let _ = tokio::time::timeout(Duration::from_secs(2), goodbye).await;
    }
    match outcome {
        Some(Ended::Failed(message)) => connection.events.offer(Event::SessionFailed {
            message: message.into(),
        }),
        _ => connection.events.offer(Event::SessionClosed),
    };
    {
        let mut sessions = lock(&shared.sessions);
        if sessions
            .get(&connection.id)
            .is_some_and(|slot| slot.live.is_none() && slot.events.finished())
        {
            sessions.remove(&connection.id);
        }
    }
    drop(connection);
    drop(permit);
    drop(lease);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(fields: &[(&str, &str)]) -> WireMap {
        let mut map = WireMap::new();
        for (key, value) in fields {
            map.insert(key, *value);
        }
        map
    }

    #[test]
    fn options_are_validated_without_echoing_values() {
        let good = [
            ("host", "example.test"),
            ("username", "root"),
            ("password", "secret"),
        ];
        assert!(options(&map(&good)).is_ok());
        let cases: [&[(&str, &str)]; 6] = [
            &[
                ("host", "bad host"),
                ("username", "root"),
                ("password", "secret"),
            ],
            &[
                ("host", "-oProxyCommand"),
                ("username", "root"),
                ("password", "secret"),
            ],
            &[
                ("host", "example.test"),
                ("username", "ro\not"),
                ("password", "secret"),
            ],
            &[("host", "example.test"), ("username", "root")],
            &[("username", "root"), ("password", "secret")],
            &[
                ("host", "example.test"),
                ("username", "root"),
                ("password", "secret"),
                ("expectedFingerprint", "not a fingerprint!"),
            ],
        ];
        for case in cases {
            let error = options(&map(case)).err().unwrap();
            assert!(
                !error.contains("secret") && !error.contains("example"),
                "{error}"
            );
        }
        let mut long = map(&good);
        long.insert("password", "x".repeat(MAX_SECRET + 1));
        assert!(options(&long).is_err());
        let mut port = map(&good);
        port.insert("port", 70_000_i64);
        assert!(options(&port).is_err());
        let mut timeout = map(&good);
        timeout.insert("timeoutMs", 10_i64);
        assert!(options(&timeout).is_err());
    }

    #[test]
    fn fingerprints_accept_the_ssh_keygen_forms() {
        assert_eq!(normalize_fingerprint("SHA256:abc+/="), "abc+/");
        assert_eq!(normalize_fingerprint(" abc "), "abc");
        let options = options(&map(&[
            ("host", "[::1]"),
            ("username", "root"),
            ("password", "x"),
            ("expectedFingerprint", "SHA256:AAAA"),
        ]))
        .unwrap();
        assert_eq!(options.host, "::1");
        assert_eq!(options.expected.as_deref(), Some("AAAA"));
    }
}
