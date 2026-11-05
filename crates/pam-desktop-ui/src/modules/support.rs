//! Pieces shared by the I/O modules: a bounded worker pool, unique names,
//! digests and bounded copies.

use std::collections::BinaryHeap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::wiremap::WireMap;

/// How long a pool thread waits for work before it exits.
const IDLE_EXIT: Duration = Duration::from_secs(30);

type Work = Box<dyn FnOnce() + Send>;

struct Job {
    priority: u8,
    sequence: u64,
    work: Work,
}

impl PartialEq for Job {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.sequence == other.sequence
    }
}

impl Eq for Job {}

impl PartialOrd for Job {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Job {
    /// Higher priority first; first come, first served within a priority.
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

#[derive(Default)]
struct PoolState {
    queue: BinaryHeap<Job>,
    /// Threads waiting for work.
    idle: usize,
    /// Threads alive.
    threads: usize,
    sequence: u64,
    closed: bool,
}

struct PoolShared {
    state: Mutex<PoolState>,
    wake: Condvar,
    name: String,
    limit: usize,
}

/// Background threads that run jobs concurrently, at most `limit` at a time.
///
/// Threads start on demand and exit after a while without work, so an idle
/// pool holds no thread. Jobs still queued when the pool is dropped never run.
pub struct WorkerPool {
    shared: Arc<PoolShared>,
}

impl WorkerPool {
    #[must_use]
    pub fn new(name: &str, limit: usize) -> Self {
        Self {
            shared: Arc::new(PoolShared {
                state: Mutex::new(PoolState::default()),
                wake: Condvar::new(),
                name: name.to_owned(),
                limit: limit.max(1),
            }),
        }
    }

    /// Queues `job` behind the jobs already waiting.
    pub fn run(&self, job: impl FnOnce() + Send + 'static) {
        self.run_with_priority(0, job);
    }

    /// Queues `job` ahead of every waiting job of a lower priority.
    pub fn run_with_priority(&self, priority: u8, job: impl FnOnce() + Send + 'static) {
        let Ok(mut state) = self.shared.state.lock() else {
            return;
        };
        if state.closed {
            return;
        }
        state.sequence += 1;
        let sequence = state.sequence;
        state.queue.push(Job {
            priority,
            sequence,
            work: Box::new(job),
        });
        if state.queue.len() > state.idle && state.threads < self.shared.limit {
            let shared = Arc::clone(&self.shared);
            let spawned = thread::Builder::new()
                .name(self.shared.name.clone())
                .spawn(move || work(&shared));
            if spawned.is_ok() {
                state.threads += 1;
                return;
            }
        }
        drop(state);
        self.shared.wake.notify_one();
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.closed = true;
            state.queue.clear();
        }
        self.shared.wake.notify_all();
    }
}

/// The loop of one pool thread: run jobs until the pool closes or no work
/// arrives for a while.
///
/// The thread leaves the count of live threads under the same lock it used
/// to decide to stop, so a job queued at that moment always finds either
/// this thread still working or room to start another.
fn work(shared: &PoolShared) {
    loop {
        let job = {
            let Ok(mut state) = shared.state.lock() else {
                return;
            };
            loop {
                if state.closed {
                    state.threads = state.threads.saturating_sub(1);
                    return;
                }
                if let Some(job) = state.queue.pop() {
                    break job;
                }
                state.idle += 1;
                let Ok((mut woken, timeout)) = shared.wake.wait_timeout(state, IDLE_EXIT) else {
                    return;
                };
                woken.idle = woken.idle.saturating_sub(1);
                if timeout.timed_out() && woken.queue.is_empty() {
                    woken.threads = woken.threads.saturating_sub(1);
                    return;
                }
                state = woken;
            }
        };
        // A job that panics must not take the thread, and the jobs queued
        // behind it, down with it.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job.work));
    }
}

/// Lower-case hexadecimal of `bytes`.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

/// Lower-case hexadecimal SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

static UNIQUE: AtomicU64 = AtomicU64::new(0);

/// A random version 4 UUID (`xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx`).
///
/// It names temporary files, imports and multipart boundaries. Should the
/// system have no entropy to give, the id is still unique within the process.
#[must_use]
pub fn uuid() -> String {
    let mut bytes = [0_u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let count = UNIQUE.fetch_add(1, Ordering::Relaxed);
        bytes[..8].copy_from_slice(&(nanos as u64).to_le_bytes());
        bytes[8..12].copy_from_slice(&std::process::id().to_le_bytes());
        bytes[12..].copy_from_slice(&(count as u32).to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let digits = hex(&bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &digits[..8],
        &digits[8..12],
        &digits[12..16],
        &digits[16..20],
        &digits[20..]
    )
}

/// A required text value of a request, with the SDK's message when absent.
pub fn required_text<'a>(request: &'a WireMap, key: &str) -> Result<&'a str, String> {
    request
        .text(key)
        .ok_or_else(|| format!("Missing text field {key}"))
}

/// Why a bounded copy stopped early.
#[derive(Debug, PartialEq, Eq)]
pub enum CopyError {
    /// The cancellation flag was set.
    Cancelled,
    /// The source holds more than the limit.
    TooLarge,
    /// Reading or writing failed; the text is the system's.
    Io(String),
}

/// Copies `reader` into `writer`, at most `limit` bytes, checking `cancel`
/// before every read. `progress` receives the running total after each chunk.
///
/// Returns the number of bytes copied. Nothing beyond `limit` is written.
pub fn copy_bounded(
    reader: &mut dyn Read,
    writer: &mut dyn Write,
    limit: u64,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<u64, CopyError> {
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(CopyError::Cancelled);
        }
        let count = match reader.read(&mut buffer) {
            Ok(0) => return Ok(total),
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(CopyError::Io(error.to_string())),
        };
        total += count as u64;
        if total > limit {
            return Err(CopyError::TooLarge);
        }
        writer
            .write_all(&buffer[..count])
            .map_err(|error| CopyError::Io(error.to_string()))?;
        progress(total);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn hex_and_digest_are_lower_case() {
        assert_eq!(hex(&[0x00, 0xab, 0xff]), "00abff");
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn uuids_are_version_four_and_distinct() {
        let first = uuid();
        let second = uuid();
        assert_ne!(first, second);
        assert_eq!(first.len(), 36);
        let bytes = first.as_bytes();
        assert_eq!(bytes[14], b'4');
        assert!(matches!(bytes[19], b'8' | b'9' | b'a' | b'b'));
        assert!(
            first
                .chars()
                .all(|character| character == '-' || character.is_ascii_hexdigit())
        );
    }

    #[test]
    fn the_pool_runs_every_job_with_bounded_concurrency() {
        let pool = WorkerPool::new("pam-test-pool", 2);
        let running = Arc::new(AtomicU64::new(0));
        let peak = Arc::new(AtomicU64::new(0));
        let (done, finished) = mpsc::channel();
        for _ in 0..8 {
            let running = Arc::clone(&running);
            let peak = Arc::clone(&peak);
            let done = done.clone();
            pool.run(move || {
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(15));
                running.fetch_sub(1, Ordering::SeqCst);
                let _ = done.send(());
            });
        }
        for _ in 0..8 {
            finished.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        assert!(peak.load(Ordering::SeqCst) <= 2);
        assert!(peak.load(Ordering::SeqCst) >= 1);
    }

    #[test]
    fn a_panicking_job_does_not_stop_the_pool() {
        let pool = WorkerPool::new("pam-test-panic", 1);
        let (done, finished) = mpsc::channel();
        pool.run(|| panic!("a job failed on purpose"));
        for value in 0..3 {
            let done = done.clone();
            pool.run(move || {
                let _ = done.send(value);
            });
        }
        let values: Vec<_> = (0..3)
            .map(|_| finished.recv_timeout(Duration::from_secs(5)).unwrap())
            .collect();
        assert_eq!(values, [0, 1, 2]);
    }

    #[test]
    fn jobs_queued_on_a_dropped_pool_never_run() {
        let pool = WorkerPool::new("pam-test-drop", 1);
        let (gate, opened) = mpsc::channel::<()>();
        let (done, finished) = mpsc::channel();
        let (running, started) = mpsc::channel();
        pool.run(move || {
            let _ = running.send(());
            let _ = opened.recv_timeout(Duration::from_secs(5));
        });
        pool.run(move || {
            let _ = done.send(());
        });
        // The first job holds the only thread; the second is still queued.
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(pool);
        gate.send(()).unwrap();
        // The sender is dropped with the job: the channel ends without a value.
        assert!(finished.recv_timeout(Duration::from_secs(5)).is_err());
    }

    #[test]
    fn higher_priorities_run_first() {
        let pool = WorkerPool::new("pam-test-priority", 1);
        let (gate, opened) = mpsc::channel::<()>();
        let (done, finished) = mpsc::channel();
        // The first job holds the only thread while the others queue up.
        pool.run(move || {
            let _ = opened.recv_timeout(Duration::from_secs(5));
        });
        for (priority, label) in [(1, "low"), (5, "high"), (1, "low-2"), (3, "middle")] {
            let done = done.clone();
            pool.run_with_priority(priority, move || {
                let _ = done.send(label);
            });
        }
        gate.send(()).unwrap();
        let order: Vec<_> = (0..4)
            .map(|_| finished.recv_timeout(Duration::from_secs(5)).unwrap())
            .collect();
        assert_eq!(order, ["high", "middle", "low", "low-2"]);
    }

    #[test]
    fn bounded_copies_stop_at_the_limit_and_on_cancel() {
        let cancel = AtomicBool::new(false);
        let mut output = Vec::new();
        let copied = copy_bounded(&mut &b"hello"[..], &mut output, 5, &cancel, |_| {}).unwrap();
        assert_eq!((copied, output.as_slice()), (5, &b"hello"[..]));

        let mut output = Vec::new();
        let result = copy_bounded(&mut &b"hello!"[..], &mut output, 5, &cancel, |_| {});
        assert_eq!(result, Err(CopyError::TooLarge));
        assert!(output.is_empty());

        cancel.store(true, Ordering::Relaxed);
        let result = copy_bounded(&mut &b"hello"[..], &mut Vec::new(), 5, &cancel, |_| {});
        assert_eq!(result, Err(CopyError::Cancelled));
    }

    #[test]
    fn required_text_names_the_missing_key() {
        let request = WireMap::new().with("path", "a.txt").with("size", 3);
        assert_eq!(required_text(&request, "path"), Ok("a.txt"));
        assert_eq!(
            required_text(&request, "size"),
            Err(String::from("Missing text field size"))
        );
        assert_eq!(
            required_text(&request, "data"),
            Err(String::from("Missing text field data"))
        );
    }
}
