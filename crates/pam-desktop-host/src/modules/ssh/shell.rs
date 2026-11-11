//! Interactive shells: a pseudo-terminal channel registered in the
//! [`TerminalHub`](pam_desktop_ui::modules::terminal::TerminalHub).
//!
//! The channel id PHP receives is the hub id. Keystrokes and resizes reach
//! the remote end through the hub (`shellWrite` and `shellResize` use the
//! same path a native element would); output goes to the hub subscriber when
//! there is one, and to PHP's `next` otherwise.

use std::sync::Arc;
use std::time::Duration;

use pam_desktop_ui::modules::Reply;
use pam_desktop_ui::modules::support::uuid;
use pam_desktop_ui::modules::terminal::{TerminalHub, TerminalInput};
use pam_desktop_ui::wiremap::WireMap;
use russh::client::Msg;
use russh::{Channel, ChannelMsg};
use tokio::sync::{mpsc, watch};

use super::events::Event;
use super::session::Connection;
use super::{MAX_SHELLS, Permit, Shared, input, lock};

/// Keystrokes accepted per write.
pub const MAX_WRITE: usize = 64 * 1024;
/// Writes and resizes waiting for the remote end.
const QUEUE: usize = 256;
const MAX_CELLS: i64 = 1000;
/// How long a closed shell waits for the server's confirmation.
const CLOSE_GRACE: Duration = Duration::from_secs(2);

enum Input {
    Data(Vec<u8>),
    Resize(u16, u16),
    Close,
}

struct Writer(mpsc::Sender<Input>);

impl TerminalInput for Writer {
    fn write(&self, bytes: &[u8]) -> bool {
        bytes.len() <= MAX_WRITE && self.0.try_send(Input::Data(bytes.to_vec())).is_ok()
    }
    fn resize(&self, cols: u16, rows: u16) -> bool {
        cols > 0 && rows > 0 && self.0.try_send(Input::Resize(cols, rows)).is_ok()
    }
}

/// A shell known to the module.
pub struct Entry {
    pub session: String,
    sender: mpsc::Sender<Input>,
    closing: watch::Sender<bool>,
}

impl Entry {
    /// Sends EOF and close to the server; the shell is dropped when the
    /// server confirms, or after [`CLOSE_GRACE`].
    pub fn close(&self) {
        self.closing.send_replace(true);
        let _ = self.sender.try_send(Input::Close);
    }
}

/// Removes the shell from the module and the hub however its task ends.
struct Registration {
    shared: Arc<Shared>,
    hub: &'static TerminalHub,
    id: String,
    status: Option<i64>,
}

impl Drop for Registration {
    fn drop(&mut self) {
        lock(&self.shared.shells).remove(&self.id);
        self.hub.unregister(&self.id, self.status);
    }
}

fn size(data: &WireMap) -> Result<(u16, u16), String> {
    let cols = input::integer(data, "cols", 80, 1, MAX_CELLS)?;
    let rows = input::integer(data, "rows", 24, 1, MAX_CELLS)?;
    Ok((
        u16::try_from(cols).unwrap_or(80),
        u16::try_from(rows).unwrap_or(24),
    ))
}

pub fn open(shared: &Arc<Shared>, connection: &Arc<Connection>, data: &WireMap, reply: Reply) {
    let parsed = (|| {
        let (cols, rows) = size(data)?;
        let term = input::text(data, "term", 64)?.unwrap_or("xterm-256color");
        if !term
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'))
        {
            return Err("ssh: term is invalid".to_owned());
        }
        Ok((cols, rows, term.to_owned()))
    })();
    let (cols, rows, term) = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            reply.failure(error);
            return;
        }
    };
    let Some(permit) = Permit::acquire(&connection.shells, MAX_SHELLS) else {
        reply.failure("ssh: too many shells open");
        return;
    };
    let shared = Arc::clone(shared);
    let connection = Arc::clone(connection);
    let runtime = connection.runtime.clone();
    runtime.spawn(async move {
        let channel = match start(&connection, &term, cols, rows).await {
            Ok(channel) => channel,
            Err(error) => {
                reply.failure(error);
                return;
            }
        };
        let id = uuid();
        let (sender, receiver) = mpsc::channel(QUEUE);
        let (closing, mut closed) = watch::channel(false);
        let hub = shared.hub;
        let Some(terminal) = hub.register(&id, Arc::new(Writer(sender.clone()))) else {
            reply.failure("ssh: terminal limit reached");
            return;
        };
        lock(&shared.shells).insert(
            id.clone(),
            Entry {
                session: connection.id.clone(),
                sender,
                closing,
            },
        );
        let mut registration = Registration {
            shared: Arc::clone(&shared),
            hub,
            id: id.clone(),
            status: None,
        };
        let (mut reader, writer) = channel.split();
        let mut result = WireMap::new();
        result.insert("channel", id.as_str());
        reply.map(&result);

        let writing = tokio::spawn(pump_input(writer, receiver));
        let mut status = None;
        {
            let reading = async {
                while let Some(message) = reader.wait().await {
                    match message {
                        ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                            if !terminal.output(&data)
                                && !connection.events.offer_data(&id, &data).await
                            {
                                break;
                            }
                        }
                        ChannelMsg::ExitStatus { exit_status } => {
                            status = Some(i64::from(exit_status));
                        }
                        ChannelMsg::ExitSignal { .. } => status = status.or(Some(-1)),
                        ChannelMsg::Close => break,
                        _ => {}
                    }
                }
            };
            tokio::pin!(reading);
            tokio::select! {
                () = &mut reading => {}
                () = async { let _ = closed.wait_for(|closing| *closing).await; } => {
                    let _ = tokio::time::timeout(CLOSE_GRACE, &mut reading).await;
                }
            }
        }
        writing.abort();
        registration.status = status;
        drop(registration);
        connection.events.offer(Event::ShellExit {
            channel: id,
            exit_code: status,
        });
        drop(permit);
    });
}

async fn start(
    connection: &Connection,
    term: &str,
    cols: u16,
    rows: u16,
) -> Result<Channel<Msg>, String> {
    let mut channel = connection.open_channel().await?;
    channel
        .request_pty(true, term, u32::from(cols), u32::from(rows), 0, 0, &[])
        .await
        .map_err(|_| "ssh: pty refused")?;
    confirmed(&mut channel, "ssh: pty refused").await?;
    channel
        .request_shell(true)
        .await
        .map_err(|_| "ssh: shell refused")?;
    confirmed(&mut channel, "ssh: shell refused").await?;
    Ok(channel)
}

/// Waits for the answer to a request made with `want_reply`.
async fn confirmed(channel: &mut Channel<Msg>, refused: &'static str) -> Result<(), String> {
    let wait = async {
        while let Some(message) = channel.wait().await {
            match message {
                ChannelMsg::Success => return Ok(()),
                ChannelMsg::Failure | ChannelMsg::Close | ChannelMsg::Eof => break,
                _ => {}
            }
        }
        Err(refused.to_owned())
    };
    tokio::time::timeout(Duration::from_secs(10), wait)
        .await
        .map_err(|_| refused.to_owned())?
}

async fn pump_input(writer: russh::ChannelWriteHalf<Msg>, mut receiver: mpsc::Receiver<Input>) {
    while let Some(input) = receiver.recv().await {
        let sent = match input {
            Input::Data(bytes) => writer.data_bytes(bytes).await,
            Input::Resize(cols, rows) => {
                writer
                    .window_change(u32::from(cols), u32::from(rows), 0, 0)
                    .await
            }
            Input::Close => {
                let _ = writer.eof().await;
                let _ = writer.close().await;
                return;
            }
        };
        if sent.is_err() {
            return;
        }
    }
}

fn channel_id(data: &WireMap) -> Result<&str, String> {
    input::handle(data, "channel")
}

fn write_call(shared: &Shared, data: &WireMap) -> Result<(), String> {
    let id = channel_id(data)?;
    let bytes = input::bytes(data, "bytes", MAX_WRITE)?;
    if !lock(&shared.shells).contains_key(id) {
        return Err("ssh: unknown channel".into());
    }
    if shared.hub.write(id, bytes) {
        Ok(())
    } else {
        Err("ssh: channel input queue full or closed".into())
    }
}

pub fn write(shared: &Shared, data: &WireMap, reply: Reply) {
    match write_call(shared, data) {
        Ok(()) => reply.done(),
        Err(error) => reply.failure(error),
    }
}

pub fn resize(shared: &Shared, data: &WireMap, reply: Reply) {
    let result = (|| {
        let id = channel_id(data)?;
        let (cols, rows) = size(data)?;
        if !lock(&shared.shells).contains_key(id) {
            return Err("ssh: unknown channel".to_owned());
        }
        if shared.hub.resize(id, cols, rows) {
            Ok(())
        } else {
            Err("ssh: channel input queue full or closed".to_owned())
        }
    })();
    match result {
        Ok(()) => reply.done(),
        Err(error) => reply.failure(error),
    }
}

pub fn close(shared: &Shared, data: &WireMap, reply: Reply) {
    match channel_id(data) {
        Ok(id) => {
            if let Some(entry) = lock(&shared.shells).get(id) {
                entry.close();
            }
            reply.done();
        }
        Err(error) => reply.failure(error),
    }
}
