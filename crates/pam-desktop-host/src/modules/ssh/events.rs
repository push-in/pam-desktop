//! The bounded event queue of one SSH session, read by PHP with `next`.
//!
//! One `next` may wait at a time; it is answered with the first queued event
//! or parked until one arrives. Shell output is coalesced per channel and
//! applies back-pressure (the reader of the channel waits for room) instead
//! of growing; progress and connection counts replace their previous queued
//! value. Anything else that does not fit fails the session visibly.

use std::collections::VecDeque;
use std::sync::Mutex;

use pam_desktop_ui::modules::Reply;
use pam_desktop_ui::wiremap::{WireMap, WireValue};
use tokio::sync::Notify;

/// Events waiting for PHP, shell output included.
pub const MAX_EVENTS: usize = 256;
/// Shell output waiting for PHP, over all channels of the session.
pub const MAX_DATA_BYTES: usize = 512 * 1024;
/// The largest shell data event.
pub const MAX_CHUNK: usize = 64 * 1024;

pub const SHELL_DATA: i64 = 1;
pub const SHELL_EXIT: i64 = 2;
pub const TRANSFER_PROGRESS: i64 = 3;
pub const TRANSFER_DONE: i64 = 4;
pub const TRANSFER_FAILED: i64 = 5;
pub const FORWARD_OPENED: i64 = 6;
pub const FORWARD_FAILED: i64 = 7;
pub const FORWARD_CONNECTIONS: i64 = 8;
pub const SESSION_CLOSED: i64 = 9;
pub const SESSION_FAILED: i64 = 10;

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    ShellData {
        channel: String,
        bytes: Vec<u8>,
    },
    ShellExit {
        channel: String,
        exit_code: Option<i64>,
    },
    TransferProgress {
        transfer: String,
        bytes: u64,
        total: u64,
    },
    TransferDone {
        transfer: String,
        bytes: u64,
    },
    TransferFailed {
        transfer: String,
        message: String,
    },
    ForwardOpened {
        forward: String,
        port: u16,
    },
    ForwardFailed {
        forward: String,
        message: String,
    },
    ForwardConnections {
        forward: String,
        connections: usize,
        failures: u64,
    },
    SessionClosed,
    SessionFailed {
        message: String,
    },
}

fn int(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

impl Event {
    pub fn kind(&self) -> i64 {
        match self {
            Self::ShellData { .. } => SHELL_DATA,
            Self::ShellExit { .. } => SHELL_EXIT,
            Self::TransferProgress { .. } => TRANSFER_PROGRESS,
            Self::TransferDone { .. } => TRANSFER_DONE,
            Self::TransferFailed { .. } => TRANSFER_FAILED,
            Self::ForwardOpened { .. } => FORWARD_OPENED,
            Self::ForwardFailed { .. } => FORWARD_FAILED,
            Self::ForwardConnections { .. } => FORWARD_CONNECTIONS,
            Self::SessionClosed => SESSION_CLOSED,
            Self::SessionFailed { .. } => SESSION_FAILED,
        }
    }

    fn terminal(&self) -> bool {
        matches!(self, Self::SessionClosed | Self::SessionFailed { .. })
    }

    fn data_len(&self) -> usize {
        match self {
            Self::ShellData { bytes, .. } => bytes.len(),
            _ => 0,
        }
    }

    pub fn map(&self) -> WireMap {
        let mut map = WireMap::new();
        map.insert("kind", self.kind());
        match self {
            Self::ShellData { channel, bytes } => {
                map.insert("channel", channel.as_str());
                map.insert("bytes", WireValue::Bytes(bytes.clone()));
            }
            Self::ShellExit { channel, exit_code } => {
                map.insert("channel", channel.as_str());
                map.insert("exitCode", exit_code.unwrap_or(-1));
            }
            Self::TransferProgress {
                transfer,
                bytes,
                total,
            } => {
                map.insert("transfer", transfer.as_str());
                map.insert("bytes", int(*bytes));
                map.insert("total", int(*total));
            }
            Self::TransferDone { transfer, bytes } => {
                map.insert("transfer", transfer.as_str());
                map.insert("bytes", int(*bytes));
            }
            Self::TransferFailed { transfer, message } => {
                map.insert("transfer", transfer.as_str());
                map.insert("message", message.as_str());
            }
            Self::ForwardOpened { forward, port } => {
                map.insert("forward", forward.as_str());
                map.insert("port", i64::from(*port));
            }
            Self::ForwardFailed { forward, message } => {
                map.insert("forward", forward.as_str());
                map.insert("message", message.as_str());
            }
            Self::ForwardConnections {
                forward,
                connections,
                failures,
            } => {
                map.insert("forward", forward.as_str());
                map.insert("connections", *connections);
                map.insert("failures", int(*failures));
            }
            Self::SessionClosed => {}
            Self::SessionFailed { message } => {
                map.insert("message", message.as_str());
            }
        }
        map
    }

    /// Whether `self`, queued, may be replaced by `next`.
    fn replaced_by(&self, next: &Self) -> bool {
        match (self, next) {
            (
                Self::TransferProgress { transfer: a, .. },
                Self::TransferProgress { transfer: b, .. },
            ) => a == b,
            (
                Self::ForwardConnections { forward: a, .. },
                Self::ForwardConnections { forward: b, .. },
            ) => a == b,
            _ => false,
        }
    }
}

#[derive(Default)]
struct Queue {
    events: VecDeque<Event>,
    data_bytes: usize,
    waiter: Option<Reply>,
    terminal: bool,
    overflowed: bool,
}

/// Outcome of offering shell output.
#[derive(Debug, PartialEq, Eq)]
enum Push {
    Done,
    Closed,
    Full,
}

#[derive(Default)]
pub struct Events {
    queue: Mutex<Queue>,
    space: Notify,
    overflow: Notify,
}

impl Events {
    /// Queues an event, or hands it to the waiting `next`. False when the
    /// session already ended. A queue that cannot take the event is replaced
    /// by a terminal failure.
    pub fn offer(&self, event: Event) -> bool {
        debug_assert!(!matches!(event, Event::ShellData { .. }));
        let (waiter, event) = {
            let Ok(mut queue) = self.queue.lock() else {
                return false;
            };
            if queue.terminal {
                return false;
            }
            if event.terminal() {
                queue.terminal = true;
            }
            if let Some(waiter) = queue.waiter.take() {
                (waiter, event)
            } else {
                if let Some(slot) = queue
                    .events
                    .iter_mut()
                    .rev()
                    .find(|queued| queued.replaced_by(&event))
                {
                    *slot = event;
                    return true;
                }
                if queue.events.len() >= MAX_EVENTS && !event.terminal() {
                    queue.events.clear();
                    queue.data_bytes = 0;
                    queue.terminal = true;
                    queue.overflowed = true;
                    queue.events.push_back(Event::SessionFailed {
                        message: "ssh event queue exceeded its limit".into(),
                    });
                    drop(queue);
                    self.space.notify_waiters();
                    self.overflow.notify_waiters();
                    return false;
                }
                queue.events.push_back(event);
                return true;
            }
        };
        let terminal = event.terminal();
        waiter.map(&event.map());
        if terminal {
            self.space.notify_waiters();
        }
        true
    }

    fn push_data(&self, channel: &str, bytes: &mut Vec<u8>) -> Push {
        let Ok(mut queue) = self.queue.lock() else {
            return Push::Closed;
        };
        if queue.terminal {
            return Push::Closed;
        }
        if let Some(waiter) = queue.waiter.take() {
            drop(queue);
            let event = Event::ShellData {
                channel: channel.to_owned(),
                bytes: std::mem::take(bytes),
            };
            waiter.map(&event.map());
            return Push::Done;
        }
        if queue.data_bytes + bytes.len() > MAX_DATA_BYTES {
            return Push::Full;
        }
        let length = bytes.len();
        if let Some(Event::ShellData {
            channel: last,
            bytes: queued,
        }) = queue.events.back_mut()
            && last == channel
            && queued.len() + length <= MAX_CHUNK
        {
            queued.append(bytes);
            queue.data_bytes += length;
            return Push::Done;
        }
        if queue.events.len() >= MAX_EVENTS {
            return Push::Full;
        }
        queue.events.push_back(Event::ShellData {
            channel: channel.to_owned(),
            bytes: std::mem::take(bytes),
        });
        queue.data_bytes += length;
        Push::Done
    }

    /// Queues shell output, waiting while the queue is full. False when the
    /// session ended.
    pub async fn offer_data(&self, channel: &str, bytes: &[u8]) -> bool {
        for piece in bytes.chunks(MAX_CHUNK) {
            let mut piece = piece.to_vec();
            loop {
                let room = self.space.notified();
                tokio::pin!(room);
                room.as_mut().enable();
                match self.push_data(channel, &mut piece) {
                    Push::Done => break,
                    Push::Closed => return false,
                    Push::Full => room.await,
                }
            }
        }
        true
    }

    /// Completes when the queue overflowed.
    pub async fn overflowed(&self) {
        loop {
            let notified = self.overflow.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.queue.lock().map_or(true, |queue| queue.overflowed) {
                return;
            }
            notified.await;
        }
    }

    /// Answers `reply` with the next event, now or when one arrives.
    pub fn next(&self, reply: Reply) {
        let event = {
            let Ok(mut queue) = self.queue.lock() else {
                reply.failure("ssh: event queue unavailable");
                return;
            };
            if let Some(event) = queue.events.pop_front() {
                queue.data_bytes -= event.data_len();
                event
            } else if queue.terminal {
                reply.failure("ssh: session is closed");
                return;
            } else if queue.waiter.is_some() {
                reply.failure("ssh: session already has a pending next");
                return;
            } else {
                queue.waiter = Some(reply);
                return;
            }
        };
        if event.data_len() > 0 {
            self.space.notify_waiters();
        }
        reply.map(&event.map());
    }

    /// The session ended and PHP read everything.
    pub fn finished(&self) -> bool {
        self.queue
            .lock()
            .map_or(true, |queue| queue.terminal && queue.events.is_empty())
    }

    /// Drops everything: a parked `next` fails, waiting producers give up.
    pub fn stop(&self) {
        let waiter = {
            let Ok(mut queue) = self.queue.lock() else {
                return;
            };
            queue.events.clear();
            queue.data_bytes = 0;
            queue.terminal = true;
            queue.waiter.take()
        };
        self.space.notify_waiters();
        if let Some(reply) = waiter {
            reply.failure("ssh: session stopped");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pam_desktop_ui::modules::ModuleResult;
    use pam_desktop_ui::runtime::{MODULE_FAILURE, MODULE_SUCCESS};
    use std::sync::Arc;
    use std::sync::mpsc::Receiver;
    use std::time::Duration;

    fn next(events: &Events) -> Receiver<ModuleResult> {
        let (reply, receiver) = Reply::channel();
        events.next(reply);
        receiver
    }

    fn event(receiver: &Receiver<ModuleResult>) -> WireMap {
        let (status, bytes) = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            status,
            MODULE_SUCCESS,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        WireMap::decode(&bytes).unwrap()
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn a_parked_next_receives_the_next_event_and_a_second_one_fails() {
        let events = Events::default();
        let parked = next(&events);
        let (status, _) = next(&events).recv().unwrap();
        assert_eq!(status, MODULE_FAILURE);
        events.offer(Event::ForwardOpened {
            forward: "f".into(),
            port: 2222,
        });
        let map = event(&parked);
        assert_eq!(map.integer("kind"), Some(FORWARD_OPENED));
        assert_eq!(map.integer("port"), Some(2222));
    }

    #[test]
    fn progress_and_connection_counts_replace_their_queued_value() {
        let events = Events::default();
        for bytes in 0..1000 {
            assert!(events.offer(Event::TransferProgress {
                transfer: "t".into(),
                bytes,
                total: 1000,
            }));
        }
        events.offer(Event::TransferProgress {
            transfer: "u".into(),
            bytes: 1,
            total: 2,
        });
        let first = event(&next(&events));
        assert_eq!(first.text("transfer"), Some("t"));
        assert_eq!(first.integer("bytes"), Some(999));
        assert_eq!(event(&next(&events)).text("transfer"), Some("u"));
    }

    #[test]
    fn overflow_is_a_visible_terminal_failure() {
        let events = Events::default();
        for index in 0..MAX_EVENTS {
            assert!(events.offer(Event::TransferDone {
                transfer: index.to_string(),
                bytes: 0,
            }));
        }
        assert!(!events.offer(Event::TransferDone {
            transfer: "late".into(),
            bytes: 0,
        }));
        block_on(events.overflowed());
        let map = event(&next(&events));
        assert_eq!(map.integer("kind"), Some(SESSION_FAILED));
        assert!(events.finished());
        assert_eq!(next(&events).recv().unwrap().0, MODULE_FAILURE);
    }

    #[test]
    fn shell_output_is_coalesced_and_waits_for_room() {
        let events = Arc::new(Events::default());
        block_on(async {
            assert!(events.offer_data("c", b"ab").await);
            assert!(events.offer_data("c", b"cd").await);
        });
        let map = event(&next(&events));
        assert_eq!(map.bytes("bytes"), Some(&b"abcd"[..]));
        assert_eq!(map.text("channel"), Some("c"));

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let producer = Arc::clone(&events);
        let task = runtime.spawn(async move {
            let block = vec![b'x'; MAX_CHUNK];
            for _ in 0..(MAX_DATA_BYTES / MAX_CHUNK) * 2 {
                if !producer.offer_data("c", &block).await {
                    return false;
                }
            }
            true
        });
        let mut received = 0;
        while received < MAX_DATA_BYTES * 2 {
            let map = event(&next(&events));
            received += map.bytes("bytes").unwrap().len();
            assert!(
                events.queue.lock().unwrap().data_bytes <= MAX_DATA_BYTES,
                "queued shell output stays bounded"
            );
        }
        assert!(runtime.block_on(task).unwrap());
    }

    #[test]
    fn stop_fails_the_waiter_and_releases_blocked_producers() {
        let events = Arc::new(Events::default());
        let parked = next(&events);
        events.stop();
        assert_eq!(
            parked.recv_timeout(Duration::from_secs(1)).unwrap().0,
            MODULE_FAILURE
        );
        assert!(!block_on(events.offer_data("c", b"late")));
        assert!(!events.offer(Event::SessionClosed));
    }
}
