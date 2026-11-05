//! The bounded long-poll channel behind every streamed API.
//!
//! PHP cannot receive several results for one request, so a stream is a
//! sequence of re-armed reads: the producer [`offer`](WatchChannel::offer)s
//! values and the application asks for the next one each time it handled the
//! previous. The channel keeps the few most recent values while nobody is
//! reading and parks at most one reader while nothing is available.

use std::collections::VecDeque;
use std::sync::Mutex;

use super::Reply;
use crate::wiremap::WireMap;

/// Values kept while no reader is parked; the oldest is dropped beyond it.
pub const CAPACITY: usize = 4;

/// A second read arrived while one is still parked.
pub const PENDING_READ: &str = "Observation already has a pending read";
/// The channel was closed while a read was parked.
pub const STOPPED: &str = "Observation stopped";
/// A read arrived after the channel closed.
pub const CLOSED: &str = "Observation is closed";

type Reader<T> = Box<dyn FnOnce(Result<T, &'static str>) + Send>;

struct State<T> {
    queue: VecDeque<T>,
    waiter: Option<Reader<T>>,
    /// The terminal value was offered: nothing else is accepted.
    finished: bool,
    closed: bool,
}

/// A single-reader channel holding at most [`CAPACITY`] values.
///
/// It is shared between the producer's thread and the thread that handles
/// module calls; readers are always answered outside the internal lock.
pub struct WatchChannel<T> {
    state: Mutex<State<T>>,
}

impl<T> Default for WatchChannel<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> WatchChannel<T> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                queue: VecDeque::with_capacity(CAPACITY),
                waiter: None,
                finished: false,
                closed: false,
            }),
        }
    }

    /// Publishes a value: it goes to the parked reader, or waits in the queue
    /// (replacing the oldest waiting value when the queue is full). Values
    /// offered after the terminal one or after closing are discarded.
    pub fn offer(&self, value: T) {
        self.publish(value, false);
    }

    /// Publishes the terminal value. It is never dropped to make room, and
    /// the channel closes once a reader received it.
    pub fn finish(&self, value: T) {
        self.publish(value, true);
    }

    fn publish(&self, value: T, terminal: bool) {
        let waiter = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            if state.closed || state.finished {
                return;
            }
            state.finished = terminal;
            match state.waiter.take() {
                Some(waiter) => {
                    // Delivered now: a terminal value leaves nothing to read.
                    state.closed = terminal;
                    waiter
                }
                None => {
                    if state.queue.len() >= CAPACITY {
                        state.queue.pop_front();
                    }
                    state.queue.push_back(value);
                    return;
                }
            }
        };
        waiter(Ok(value));
    }

    /// Reads the next value: at once when one is waiting, otherwise when the
    /// producer offers one. `reader` is called exactly once unless the channel
    /// is dropped while it is parked.
    pub fn next(&self, reader: impl FnOnce(Result<T, &'static str>) + Send + 'static) {
        let outcome = {
            let Ok(mut state) = self.state.lock() else {
                return reader(Err(CLOSED));
            };
            if state.closed {
                Err(CLOSED)
            } else if let Some(value) = state.queue.pop_front() {
                if state.finished && state.queue.is_empty() {
                    state.closed = true;
                }
                Ok(value)
            } else if state.waiter.is_some() {
                Err(PENDING_READ)
            } else {
                state.waiter = Some(Box::new(reader));
                return;
            }
        };
        reader(outcome);
    }

    /// Closes the channel: waiting values are discarded, the parked reader
    /// fails with [`STOPPED`] and later reads fail with [`CLOSED`].
    pub fn close(&self) {
        let waiter = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            if state.closed {
                return;
            }
            state.closed = true;
            state.queue.clear();
            state.waiter.take()
        };
        if let Some(waiter) = waiter {
            waiter(Err(STOPPED));
        }
    }

    /// Whether reads fail from now on.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.state.lock().map_or(true, |state| state.closed)
    }
}

impl WatchChannel<WireMap> {
    /// Answers `reply` with the next event, or with the channel's failure.
    pub fn next_reply(&self, reply: Reply) {
        self.next(move |outcome| match outcome {
            Ok(event) => reply.map(&event),
            Err(message) => reply.failure(message),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::mpsc;
    use std::time::Duration;

    fn read(channel: &WatchChannel<i32>) -> mpsc::Receiver<Result<i32, &'static str>> {
        let (sender, receiver) = mpsc::channel();
        channel.next(move |outcome| {
            let _ = sender.send(outcome);
        });
        receiver
    }

    fn now(channel: &WatchChannel<i32>) -> Option<Result<i32, &'static str>> {
        read(channel).try_recv().ok()
    }

    #[test]
    fn queued_values_are_read_in_order() {
        let channel = WatchChannel::new();
        channel.offer(1);
        channel.offer(2);
        assert_eq!(now(&channel), Some(Ok(1)));
        assert_eq!(now(&channel), Some(Ok(2)));
    }

    #[test]
    fn the_oldest_value_is_dropped_when_the_queue_is_full() {
        let channel = WatchChannel::new();
        for value in 1..=6 {
            channel.offer(value);
        }
        let values: Vec<_> = (0..CAPACITY).map(|_| now(&channel)).collect();
        assert_eq!(values, [Some(Ok(3)), Some(Ok(4)), Some(Ok(5)), Some(Ok(6))]);
        // Nothing is left: the next read parks.
        assert_eq!(now(&channel), None);
    }

    #[test]
    fn a_parked_reader_receives_the_next_value() {
        let channel = WatchChannel::new();
        let parked = read(&channel);
        assert!(parked.try_recv().is_err());
        channel.offer(7);
        assert_eq!(parked.try_recv(), Ok(Ok(7)));
        // The value went to the reader, not to the queue.
        assert_eq!(now(&channel), None);
    }

    #[test]
    fn a_second_concurrent_read_fails() {
        let channel = WatchChannel::new();
        let parked = read(&channel);
        assert_eq!(now(&channel), Some(Err(PENDING_READ)));
        channel.offer(1);
        assert_eq!(parked.try_recv(), Ok(Ok(1)));
    }

    #[test]
    fn closing_fails_the_parked_reader_and_later_reads() {
        let channel = WatchChannel::new();
        let parked = read(&channel);
        channel.close();
        assert_eq!(parked.try_recv(), Ok(Err(STOPPED)));
        assert_eq!(now(&channel), Some(Err(CLOSED)));
        assert!(channel.is_closed());
        // Closing twice and offering afterwards are harmless.
        channel.close();
        channel.offer(1);
        assert_eq!(now(&channel), Some(Err(CLOSED)));
    }

    #[test]
    fn closing_discards_queued_values() {
        let channel = WatchChannel::new();
        channel.offer(1);
        channel.close();
        assert_eq!(now(&channel), Some(Err(CLOSED)));
    }

    #[test]
    fn the_terminal_value_survives_a_full_queue_and_closes_the_channel() {
        let channel = WatchChannel::new();
        for value in 1..=4 {
            channel.offer(value);
        }
        channel.finish(99);
        // Offers after the terminal value cannot push it out.
        for value in 100..110 {
            channel.offer(value);
        }
        let values: Vec<_> = (0..CAPACITY).map(|_| now(&channel)).collect();
        assert_eq!(
            values,
            [Some(Ok(2)), Some(Ok(3)), Some(Ok(4)), Some(Ok(99))]
        );
        assert_eq!(now(&channel), Some(Err(CLOSED)));
    }

    #[test]
    fn a_terminal_value_delivered_to_a_parked_reader_closes_the_channel() {
        let channel = WatchChannel::new();
        let parked = read(&channel);
        channel.finish(5);
        assert_eq!(parked.try_recv(), Ok(Ok(5)));
        assert_eq!(now(&channel), Some(Err(CLOSED)));
    }

    #[test]
    fn producers_and_readers_on_different_threads_lose_nothing_in_order() {
        let channel = Arc::new(WatchChannel::new());
        let producer = Arc::clone(&channel);
        let (ready, proceed) = mpsc::channel::<()>();
        let handle = std::thread::spawn(move || {
            for value in 0..200 {
                // One value per read: wait until the reader consumed the last.
                proceed.recv_timeout(Duration::from_secs(5)).unwrap();
                producer.offer(value);
            }
            proceed.recv_timeout(Duration::from_secs(5)).unwrap();
            producer.finish(-1);
        });
        let mut seen = Vec::new();
        loop {
            let receiver = read(&channel);
            ready.send(()).unwrap();
            match receiver.recv_timeout(Duration::from_secs(5)).unwrap() {
                Ok(-1) => break,
                Ok(value) => seen.push(value),
                Err(message) => panic!("unexpected failure: {message}"),
            }
        }
        handle.join().unwrap();
        assert_eq!(seen, (0..200).collect::<Vec<_>>());
        assert_eq!(now(&channel), Some(Err(CLOSED)));
    }

    #[test]
    fn events_answer_module_replies() {
        let channel = WatchChannel::new();
        channel.offer(WireMap::new().with("state", 1));
        let (reply, answer) = Reply::channel();
        channel.next_reply(reply);
        let (status, payload) = answer.try_recv().unwrap();
        assert_eq!(status, crate::runtime::MODULE_SUCCESS);
        assert_eq!(WireMap::decode(&payload).unwrap().integer("state"), Some(1));

        let (parked, parked_answer) = Reply::channel();
        channel.next_reply(parked);
        let (second, second_answer) = Reply::channel();
        channel.next_reply(second);
        assert_eq!(
            second_answer.try_recv().unwrap(),
            (
                crate::runtime::MODULE_FAILURE,
                PENDING_READ.as_bytes().to_vec()
            )
        );
        channel.close();
        assert_eq!(
            parked_answer.try_recv().unwrap(),
            (crate::runtime::MODULE_FAILURE, STOPPED.as_bytes().to_vec())
        );
    }
}
