//! The PHP worker process and its binary channel.
//!
//! The application runs in a separate `php`/`pam exec` process. Its stdout
//! carries frames, module calls and errors to the host; its stdin receives
//! events and module results. Both directions use
//! `u32 length (LE) · u8 type · payload`.

use std::ffi::OsString;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Sender};
use std::thread;

const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

// Worker -> host.
const COMMIT: u8 = 0x01;
const CALL: u8 = 0x02;
const CALL_TYPED: u8 = 0x03;
const ERROR: u8 = 0x04;
const READY: u8 = 0x05;
const IDLE: u8 = 0x06;

// Host -> worker.
const EVENT: u8 = 0x01;
const MODULE_RESULT: u8 = 0x02;
const FLUSH: u8 = 0x03;
const RESYNC: u8 = 0x04;
const SHUTDOWN: u8 = 0x05;
const CHECKPOINT: u8 = 0x06;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerMessage {
    /// A `PNT1` tree or `PNP1` patch frame.
    Commit(Vec<u8>),
    Call { request: u64, module: String, method: String, payload: Vec<u8> },
    CallTyped { request: u64, operation: u16, payload: Vec<u8> },
    Error(String),
    /// The entry script finished booting.
    Ready,
    /// The worker handled every message up to the last flush marker.
    Idle,
    /// The process ended (exit code when it exited normally).
    Exited(Option<i32>),
}

#[derive(Clone, Debug)]
pub struct WorkerCommand {
    pub program: PathBuf,
    /// Arguments before the entry script (interpreter flags, the bootstrap).
    pub arguments: Vec<OsString>,
    pub entry: PathBuf,
    pub working_directory: Option<PathBuf>,
    pub environment: Vec<(String, String)>,
}

pub struct Worker {
    child: Child,
    outbox: Option<Sender<Vec<u8>>>,
    events_since_flush: bool,
}

fn frame(kind: u8, payload_parts: &[&[u8]]) -> Vec<u8> {
    let length: usize = payload_parts.iter().map(|part| part.len()).sum::<usize>() + 1;
    let mut message = Vec::with_capacity(length + 4);
    message.extend_from_slice(&(length as u32).to_le_bytes());
    message.push(kind);
    for part in payload_parts {
        message.extend_from_slice(part);
    }
    message
}

fn read_message(reader: &mut impl Read) -> io::Result<Option<(u8, Vec<u8>)>> {
    let mut header = [0_u8; 5];
    match reader.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = u32::from_le_bytes([header[0], header[1], header[2], header[3]]) as usize;
    if length == 0 || length > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "malformed worker message"));
    }
    let mut payload = vec![0_u8; length - 1];
    reader.read_exact(&mut payload)?;
    Ok(Some((header[4], payload)))
}

fn decode(kind: u8, payload: Vec<u8>) -> Option<WorkerMessage> {
    match kind {
        COMMIT => Some(WorkerMessage::Commit(payload)),
        CALL => {
            let request = u64::from_le_bytes(payload.get(0..8)?.try_into().ok()?);
            let module_length = usize::from(u16::from_le_bytes(payload.get(8..10)?.try_into().ok()?));
            let method_length = usize::from(u16::from_le_bytes(payload.get(10..12)?.try_into().ok()?));
            let module_end = 12 + module_length;
            let method_end = module_end + method_length;
            let module = std::str::from_utf8(payload.get(12..module_end)?).ok()?.to_owned();
            let method = std::str::from_utf8(payload.get(module_end..method_end)?).ok()?.to_owned();
            Some(WorkerMessage::Call { request, module, method, payload: payload[method_end..].to_vec() })
        }
        CALL_TYPED => {
            let request = u64::from_le_bytes(payload.get(0..8)?.try_into().ok()?);
            let operation = u16::from_le_bytes(payload.get(8..10)?.try_into().ok()?);
            Some(WorkerMessage::CallTyped { request, operation, payload: payload[10..].to_vec() })
        }
        ERROR => Some(WorkerMessage::Error(String::from_utf8_lossy(&payload).into_owned())),
        READY => Some(WorkerMessage::Ready),
        IDLE => Some(WorkerMessage::Idle),
        _ => None,
    }
}

impl Worker {
    /// Starts the worker. `sink` is called on a background thread for every
    /// message; it must hand the message to the UI thread and return quickly.
    pub fn spawn(command: &WorkerCommand, sink: impl Fn(WorkerMessage) + Send + 'static) -> io::Result<Self> {
        let mut process = Command::new(&command.program);
        process
            .args(&command.arguments)
            .arg(&command.entry)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if let Some(directory) = &command.working_directory {
            process.current_dir(directory);
        }
        for (key, value) in &command.environment {
            process.env(key, value);
        }
        let mut child = process.spawn()?;
        let stdout = child.stdout.take().ok_or_else(|| io::Error::other("worker stdout unavailable"))?;
        let stdin = child.stdin.take().ok_or_else(|| io::Error::other("worker stdin unavailable"))?;

        thread::Builder::new().name("pam-worker-read".into()).spawn(move || {
            let mut reader = BufReader::with_capacity(64 * 1024, stdout);
            loop {
                match read_message(&mut reader) {
                    Ok(Some((kind, payload))) => {
                        if let Some(message) = decode(kind, payload) {
                            sink(message);
                        }
                    }
                    Ok(None) | Err(_) => break,
                }
            }
            sink(WorkerMessage::Exited(None));
        })?;

        let (outbox, receiver) = mpsc::channel::<Vec<u8>>();
        thread::Builder::new().name("pam-worker-write".into()).spawn(move || {
            let mut writer: BufWriter<ChildStdin> = BufWriter::with_capacity(64 * 1024, stdin);
            while let Ok(message) = receiver.recv() {
                if writer.write_all(&message).is_err() {
                    break;
                }
                // Coalesce whatever is already queued into one write.
                let mut failed = false;
                while let Ok(next) = receiver.try_recv() {
                    if writer.write_all(&next).is_err() {
                        failed = true;
                        break;
                    }
                }
                if failed || writer.flush().is_err() {
                    break;
                }
            }
        })?;

        Ok(Self { child, outbox: Some(outbox), events_since_flush: false })
    }

    fn send(&mut self, message: Vec<u8>) {
        if let Some(outbox) = &self.outbox
            && outbox.send(message).is_err() {
                self.outbox = None;
            }
    }

    pub fn send_event(&mut self, node: u64, kind: u16, payload: &[u8]) {
        self.send(frame(EVENT, &[&node.to_le_bytes(), &kind.to_le_bytes(), payload]));
        self.events_since_flush = true;
    }

    pub fn send_module_result(&mut self, request: u64, status: u8, payload: &[u8]) {
        self.send(frame(MODULE_RESULT, &[&request.to_le_bytes(), &[status], payload]));
        self.events_since_flush = true;
    }

    /// Ends a batch: the worker renders once for everything sent since the
    /// previous marker. A no-op when nothing was sent.
    pub fn flush(&mut self) {
        if self.events_since_flush {
            self.events_since_flush = false;
            self.send(frame(FLUSH, &[]));
        }
    }

    /// The engine rejected a frame: ask for a complete tree.
    pub fn resynchronize(&mut self) {
        self.send(frame(RESYNC, &[]));
        self.send(frame(FLUSH, &[]));
    }

    pub fn checkpoint(&mut self) {
        self.send(frame(CHECKPOINT, &[]));
    }

    pub fn shutdown(&mut self) {
        self.send(frame(SHUTDOWN, &[]));
        self.outbox = None;
    }

    #[must_use]
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Closing stdin ends the worker loop; do not leave an orphan behind.
        self.outbox = None;
        for _ in 0..50 {
            match self.child.try_wait() {
                Ok(Some(_)) | Err(_) => return,
                Ok(None) => thread::sleep(std::time::Duration::from_millis(10)),
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_carry_length_type_and_payload() {
        let message = frame(EVENT, &[&7_u64.to_le_bytes(), &1_u16.to_le_bytes(), b"hi"]);
        assert_eq!(&message[..5], &[13, 0, 0, 0, EVENT]);
        assert_eq!(message.len(), 4 + 13);
    }

    #[test]
    fn decodes_worker_messages() {
        let mut call = Vec::new();
        call.extend_from_slice(&9_u64.to_le_bytes());
        call.extend_from_slice(&5_u16.to_le_bytes());
        call.extend_from_slice(&3_u16.to_le_bytes());
        call.extend_from_slice(b"filesgetXY");
        assert_eq!(
            decode(CALL, call),
            Some(WorkerMessage::Call { request: 9, module: "files".into(), method: "get".into(), payload: b"XY".to_vec() }),
        );
        assert_eq!(decode(CALL, vec![1, 2, 3]), None);
        assert_eq!(decode(READY, Vec::new()), Some(WorkerMessage::Ready));
    }

    #[test]
    fn reads_a_stream_of_messages() {
        let mut stream = frame(COMMIT, &[b"PNT1"]);
        stream.extend_from_slice(&frame(READY, &[]));
        let mut reader = io::Cursor::new(stream);
        assert_eq!(read_message(&mut reader).unwrap(), Some((COMMIT, b"PNT1".to_vec())));
        assert_eq!(read_message(&mut reader).unwrap(), Some((READY, Vec::new())));
        assert_eq!(read_message(&mut reader).unwrap(), None);
    }
}
