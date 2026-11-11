//! The `ssh` module: SSH sessions, remote commands, SFTP, local port
//! forwarding and interactive shells, over russh on a tokio runtime that only
//! runs while a session is open.
//!
//! Every call is validated and bounded on the UI thread and then runs on the
//! runtime; its reply comes from there. Events (shell output, transfer
//! progress, forwards, the end of the session) are read by PHP with `next`,
//! one per call, like the `websocket` module.
//!
//! Shell output also goes to the process [`TerminalHub`]: a native terminal
//! element that subscribes to a channel receives it there directly, and PHP
//! stops receiving it through `next`.
//!
//! Nothing here logs hosts, users, secrets, commands, paths or payloads.
//! Failure messages are short codes (`auth-failed`, `host-key-mismatch:…`,
//! `sftp: no-such-file`).

mod events;
mod exec;
mod forward;
mod runtime;
mod session;
mod sftp;
mod shell;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use pam_desktop_ui::modules::terminal::TerminalHub;
use pam_desktop_ui::modules::{Module, Reply, request_map};
use pam_desktop_ui::wiremap::WireMap;

use events::Events;
use session::Connection;

/// Sessions open or connecting at once.
pub const MAX_SESSIONS: usize = 16;
/// Remote commands running at once in one session.
pub const MAX_EXECS: usize = 8;
/// Interactive shells open at once in one session.
pub const MAX_SHELLS: usize = 8;
/// Local forwards listening at once in one session.
pub const MAX_FORWARDS: usize = 16;
/// Uploads and downloads running at once in one session.
pub const MAX_TRANSFERS: usize = 4;
/// The largest payload PHP and the host exchange is a 1 MiB WireMap; byte
/// values stay below it with room for the other keys.
pub const MAX_BYTES_VALUE: usize = 960 * 1024;
/// The longest remote or local path.
pub const MAX_PATH: usize = 4096;
/// Ended sessions whose events PHP has not read yet.
const MAX_ENDED: usize = 16;

pub(crate) struct Slot {
    events: Arc<Events>,
    live: Option<Arc<Connection>>,
}

/// State shared by the module (UI thread) and its tasks (runtime).
pub(crate) struct Shared {
    runtime: Arc<runtime::Runtime>,
    hub: &'static TerminalHub,
    sessions: Mutex<HashMap<String, Slot>>,
    /// Sessions connecting or open.
    active: Arc<AtomicUsize>,
    shells: Mutex<HashMap<String, shell::Entry>>,
    transfers: Mutex<HashMap<String, sftp::Transfer>>,
    forwards: Mutex<HashMap<String, forward::Entry>>,
}

impl Shared {
    fn connection(&self, data: &WireMap) -> Result<Arc<Connection>, &'static str> {
        let id = data.text("id").unwrap_or("");
        if id.is_empty() || id.len() > 64 {
            return Err("ssh: session id required");
        }
        self.sessions
            .lock()
            .map_err(|_| "ssh: unavailable")?
            .get(id)
            .and_then(|slot| slot.live.clone())
            .ok_or("ssh: unknown session")
    }

    /// The session ended: tell its children to stop and forget it once PHP
    /// read its last event.
    fn ended(&self, id: &str) {
        for entry in lock(&self.shells).values() {
            if entry.session == id {
                entry.close();
            }
        }
        for transfer in lock(&self.transfers).values() {
            if transfer.session == id {
                transfer.cancel();
            }
        }
        for forward in lock(&self.forwards).values() {
            if forward.session == id {
                forward.stop();
            }
        }
        let mut sessions = lock(&self.sessions);
        if let Some(slot) = sessions.get_mut(id) {
            slot.live = None;
            if slot.events.finished() {
                sessions.remove(id);
            }
        }
        let ended: Vec<String> = sessions
            .iter()
            .filter(|(_, slot)| slot.live.is_none())
            .map(|(id, _)| id.clone())
            .collect();
        if ended.len() > MAX_ENDED {
            for id in &ended[..ended.len() - MAX_ENDED] {
                sessions.remove(id);
            }
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A slot in a bounded count, released on drop.
pub(crate) struct Permit(Arc<AtomicUsize>);

impl Permit {
    pub(crate) fn acquire(counter: &Arc<AtomicUsize>, limit: usize) -> Option<Self> {
        let mut count = counter.load(Ordering::Acquire);
        loop {
            if count >= limit {
                return None;
            }
            match counter.compare_exchange_weak(
                count,
                count + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(Self(Arc::clone(counter))),
                Err(actual) => count = actual,
            }
        }
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The `ssh` module.
pub struct Ssh {
    shared: Arc<Shared>,
}

impl Default for Ssh {
    fn default() -> Self {
        Self::new()
    }
}

impl Ssh {
    #[must_use]
    pub fn new() -> Self {
        Self::with_hub(TerminalHub::shared())
    }

    /// A module that routes terminal output through `hub`.
    #[must_use]
    pub fn with_hub(hub: &'static TerminalHub) -> Self {
        Self {
            shared: Arc::new(Shared {
                runtime: Arc::new(runtime::Runtime::default()),
                hub,
                sessions: Mutex::new(HashMap::new()),
                active: Arc::new(AtomicUsize::new(0)),
                shells: Mutex::new(HashMap::new()),
                transfers: Mutex::new(HashMap::new()),
                forwards: Mutex::new(HashMap::new()),
            }),
        }
    }

    fn disconnect(&self, data: &WireMap, reply: Reply) {
        let id = data.text("id").unwrap_or("");
        let slot = lock(&self.shared.sessions).remove(id);
        if let Some(slot) = slot {
            match slot.live {
                // The session says goodbye and offers the closed event.
                Some(connection) => connection.stop(),
                None => slot.events.stop(),
            }
        }
        reply.done();
    }

    fn next(&self, data: &WireMap, reply: Reply) {
        let id = data.text("id").unwrap_or("");
        let events = lock(&self.shared.sessions)
            .get(id)
            .map(|slot| Arc::clone(&slot.events));
        let Some(events) = events else {
            reply.failure("ssh: unknown session");
            return;
        };
        events.next(reply);
        let mut sessions = lock(&self.shared.sessions);
        if sessions
            .get(id)
            .is_some_and(|slot| slot.live.is_none() && slot.events.finished())
        {
            sessions.remove(id);
        }
    }
}

impl Module for Ssh {
    fn call(&mut self, method: &str, payload: &[u8], reply: Reply) {
        let data = match request_map(payload) {
            Ok(data) => data,
            Err(error) => {
                reply.failure(error);
                return;
            }
        };
        let shared = &self.shared;
        match method {
            "connect" => session::connect(shared, &data, reply),
            "disconnect" => self.disconnect(&data, reply),
            "next" => self.next(&data, reply),
            "sftpCancel" => sftp::cancel(shared, &data, reply),
            "forwardStop" => forward::stop(shared, &data, reply),
            "shellWrite" => shell::write(shared, &data, reply),
            "shellResize" => shell::resize(shared, &data, reply),
            "shellClose" => shell::close(shared, &data, reply),
            "exec" | "shellOpen" | "forwardLocal" => {
                let connection = match shared.connection(&data) {
                    Ok(connection) => connection,
                    Err(error) => {
                        reply.failure(error);
                        return;
                    }
                };
                match method {
                    "exec" => exec::run(&connection, &data, reply),
                    "shellOpen" => shell::open(shared, &connection, &data, reply),
                    _ => forward::open(shared, &connection, &data, reply),
                }
            }
            _ if method.starts_with("sftp") => match shared.connection(&data) {
                Ok(connection) => sftp::call(shared, &connection, method, &data, reply),
                Err(error) => reply.failure(error),
            },
            _ => reply.failure("ssh: unknown method"),
        }
    }

    fn reset(&mut self) {
        let slots: Vec<Slot> = lock(&self.shared.sessions)
            .drain()
            .map(|(_, slot)| slot)
            .collect();
        for slot in slots {
            slot.events.stop();
            if let Some(connection) = slot.live {
                connection.stop();
            }
        }
    }
}

impl Drop for Ssh {
    fn drop(&mut self) {
        self.reset();
    }
}

/// Input validation shared by the submodules. Messages name the field, never
/// its value.
pub(crate) mod input {
    use pam_desktop_ui::wiremap::WireMap;

    pub fn text<'a>(
        data: &'a WireMap,
        key: &'static str,
        max: usize,
    ) -> Result<Option<&'a str>, String> {
        match data.text(key) {
            None | Some("") => Ok(None),
            Some(value) if value.len() > max => Err(format!("ssh: {key} is too long")),
            Some(value) if value.contains('\0') => Err(format!("ssh: {key} is invalid")),
            Some(value) => Ok(Some(value)),
        }
    }

    pub fn required<'a>(
        data: &'a WireMap,
        key: &'static str,
        max: usize,
    ) -> Result<&'a str, String> {
        text(data, key, max)?.ok_or_else(|| format!("ssh: {key} required"))
    }

    pub fn integer(
        data: &WireMap,
        key: &'static str,
        default: i64,
        min: i64,
        max: i64,
    ) -> Result<i64, String> {
        match data.get(key) {
            None => Ok(default),
            Some(_) => match data.integer(key) {
                Some(value) if (min..=max).contains(&value) => Ok(value),
                _ => Err(format!("ssh: {key} must be between {min} and {max}")),
            },
        }
    }

    pub fn port(data: &WireMap, key: &'static str, min: i64) -> Result<u16, String> {
        let value = integer(data, key, -1, min, 65535)?;
        u16::try_from(value).map_err(|_| format!("ssh: {key} required"))
    }

    /// A host name or an IP address: no spaces, controls or option-like
    /// leading dash. Brackets around an IPv6 address are removed.
    pub fn host(data: &WireMap, key: &'static str) -> Result<String, String> {
        let host = required(data, key, 255)?;
        let host = host
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .unwrap_or(host);
        if host.is_empty()
            || host.len() > 253
            || host.starts_with('-')
            || host
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '/' | '@' | '\\'))
        {
            return Err(format!("ssh: {key} is invalid"));
        }
        Ok(host.to_owned())
    }

    pub fn path<'a>(data: &'a WireMap, key: &'static str) -> Result<&'a str, String> {
        let path = required(data, key, super::MAX_PATH)?;
        if path.chars().any(|c| c.is_control()) {
            return Err(format!("ssh: {key} is invalid"));
        }
        Ok(path)
    }

    pub fn bool(data: &WireMap, key: &'static str) -> bool {
        data.boolean(key).unwrap_or(false)
    }

    /// An opaque bytes value (text is accepted as its UTF-8 bytes).
    pub fn bytes<'a>(data: &'a WireMap, key: &'static str, max: usize) -> Result<&'a [u8], String> {
        match data.bytes(key) {
            Some(bytes) if bytes.len() > max => Err(format!("ssh: {key} is too large")),
            Some(bytes) => Ok(bytes),
            None => Err(format!("ssh: {key} required")),
        }
    }

    /// An `id`-like handle.
    pub fn handle<'a>(data: &'a WireMap, key: &'static str) -> Result<&'a str, String> {
        required(data, key, 64)
    }
}

#[cfg(test)]
mod tests;
