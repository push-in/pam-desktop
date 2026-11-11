//! Terminal byte routing that does not pass through PHP.
//!
//! A producer (an SSH shell channel of the host's `ssh` module) registers a
//! channel id with a [`TerminalInput`] that carries keystrokes and resizes to
//! the remote end, and pushes the output it receives with
//! [`TerminalChannel::output`]. A consumer (a native terminal element of the
//! render core) subscribes to the same id: it gets the recent output kept in a
//! bounded ring buffer, then every new chunk through its callback, and writes
//! keystrokes back with [`TerminalHub::write`] and [`TerminalHub::resize`].
//!
//! The hub knows nothing about SSH and never logs or inspects bytes. Callbacks
//! run on the producer's thread, never under a lock of the hub; a consumer
//! that lives on the UI thread posts what it receives to the inbox.
//!
//! When nobody subscribes, the producer is told so ([`TerminalChannel::output`]
//! returns `false`) and delivers the output another way (the `ssh` module
//! queues it for PHP's `next`).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock, Weak};

/// Output kept per channel for a consumer that attaches later.
pub const RING_BYTES: usize = 128 * 1024;
/// Channels that may be registered at once in one hub.
pub const MAX_CHANNELS: usize = 256;

/// The way back to the remote end of a terminal channel. Both calls must not
/// block: they queue the request and answer whether it was accepted.
pub trait TerminalInput: Send + Sync {
    /// Queues bytes typed by the user.
    fn write(&self, bytes: &[u8]) -> bool;
    /// Queues a new size of the terminal, in cells.
    fn resize(&self, cols: u16, rows: u16) -> bool;
}

/// What a subscriber hears.
#[derive(Debug, PartialEq, Eq)]
pub enum TerminalEvent<'a> {
    /// A chunk of output, in the order it arrived.
    Output(&'a [u8]),
    /// The channel ended; the exit status when the remote reported one.
    Exited(Option<i64>),
}

/// A consumer callback.
pub type TerminalSubscriber = Arc<dyn Fn(TerminalEvent<'_>) + Send + Sync>;

#[derive(Default)]
struct State {
    ring: VecDeque<u8>,
    subscriber: Option<(u64, TerminalSubscriber)>,
    next_token: u64,
    exited: Option<Option<i64>>,
}

/// One registered channel.
pub struct TerminalChannel {
    input: Arc<dyn TerminalInput>,
    state: Mutex<State>,
}

impl TerminalChannel {
    /// Records output and hands it to the subscriber, if there is one.
    /// Answers whether a subscriber received it. Must be called by a single
    /// producer thread at a time so chunks keep their order.
    pub fn output(&self, bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return self.has_subscriber();
        }
        let subscriber = {
            let Ok(mut state) = self.state.lock() else {
                return false;
            };
            if bytes.len() >= RING_BYTES {
                state.ring.clear();
                state
                    .ring
                    .extend(bytes[bytes.len() - RING_BYTES..].iter().copied());
            } else {
                let overflow = (state.ring.len() + bytes.len()).saturating_sub(RING_BYTES);
                state.ring.drain(..overflow);
                state.ring.extend(bytes.iter().copied());
            }
            state.subscriber.as_ref().map(|(_, s)| Arc::clone(s))
        };
        match subscriber {
            Some(subscriber) => {
                subscriber(TerminalEvent::Output(bytes));
                true
            }
            None => false,
        }
    }

    /// Whether a consumer is attached.
    #[must_use]
    pub fn has_subscriber(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| state.subscriber.is_some())
    }

    fn exit(&self, status: Option<i64>) {
        let subscriber = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            state.exited = Some(status);
            state.subscriber.take().map(|(_, s)| s)
        };
        if let Some(subscriber) = subscriber {
            subscriber(TerminalEvent::Exited(status));
        }
    }
}

/// A live subscription; dropping it detaches the consumer.
pub struct TerminalSubscription {
    channel: Weak<TerminalChannel>,
    token: u64,
    backlog: Vec<u8>,
}

impl TerminalSubscription {
    /// The output recorded before the subscription (at most [`RING_BYTES`]).
    /// Apply it before the first callback is handled.
    #[must_use]
    pub fn backlog(&self) -> &[u8] {
        &self.backlog
    }

    /// Releases the backlog buffer once it was applied.
    pub fn take_backlog(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.backlog)
    }
}

impl Drop for TerminalSubscription {
    fn drop(&mut self) {
        if let Some(channel) = self.channel.upgrade()
            && let Ok(mut state) = channel.state.lock()
            && state
                .subscriber
                .as_ref()
                .is_some_and(|(token, _)| *token == self.token)
        {
            state.subscriber = None;
        }
    }
}

/// Why a subscription was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum SubscribeError {
    /// No channel with that id is registered.
    Unknown,
    /// Another consumer is attached; one at a time.
    Busy,
}

/// The registry of terminal channels, by id.
#[derive(Default)]
pub struct TerminalHub {
    channels: Mutex<HashMap<String, Arc<TerminalChannel>>>,
}

impl TerminalHub {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The hub of this process, shared by the host's modules and the render
    /// core.
    pub fn shared() -> &'static Self {
        static HUB: OnceLock<TerminalHub> = OnceLock::new();
        HUB.get_or_init(Self::new)
    }

    /// Registers a producer. Fails when the id is taken or the hub is full.
    pub fn register(
        &self,
        id: &str,
        input: Arc<dyn TerminalInput>,
    ) -> Option<Arc<TerminalChannel>> {
        let mut channels = self.channels.lock().ok()?;
        if channels.len() >= MAX_CHANNELS || channels.contains_key(id) {
            return None;
        }
        let channel = Arc::new(TerminalChannel {
            input,
            state: Mutex::new(State::default()),
        });
        channels.insert(id.to_owned(), Arc::clone(&channel));
        Some(channel)
    }

    /// Removes a channel and tells its subscriber it ended.
    pub fn unregister(&self, id: &str, status: Option<i64>) {
        let channel = self
            .channels
            .lock()
            .ok()
            .and_then(|mut channels| channels.remove(id));
        if let Some(channel) = channel {
            channel.exit(status);
        }
    }

    /// Whether a channel is registered.
    #[must_use]
    pub fn contains(&self, id: &str) -> bool {
        self.channels
            .lock()
            .is_ok_and(|channels| channels.contains_key(id))
    }

    fn channel(&self, id: &str) -> Option<Arc<TerminalChannel>> {
        self.channels.lock().ok()?.get(id).cloned()
    }

    /// Attaches a consumer. From now on output goes to `subscriber` instead
    /// of the producer's fallback.
    pub fn subscribe(
        &self,
        id: &str,
        subscriber: TerminalSubscriber,
    ) -> Result<TerminalSubscription, SubscribeError> {
        let channel = self.channel(id).ok_or(SubscribeError::Unknown)?;
        let mut state = channel.state.lock().map_err(|_| SubscribeError::Unknown)?;
        if state.exited.is_some() {
            return Err(SubscribeError::Unknown);
        }
        if state.subscriber.is_some() {
            return Err(SubscribeError::Busy);
        }
        state.next_token += 1;
        let token = state.next_token;
        state.subscriber = Some((token, subscriber));
        let backlog = state.ring.iter().copied().collect();
        drop(state);
        Ok(TerminalSubscription {
            channel: Arc::downgrade(&channel),
            token,
            backlog,
        })
    }

    /// Sends keystrokes to the remote end. False when the channel is unknown
    /// or its queue is full.
    pub fn write(&self, id: &str, bytes: &[u8]) -> bool {
        self.channel(id)
            .is_some_and(|channel| channel.input.write(bytes))
    }

    /// Sends a new terminal size to the remote end.
    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> bool {
        self.channel(id)
            .is_some_and(|channel| channel.input.resize(cols, rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<String>>);
    impl TerminalInput for Recorder {
        fn write(&self, bytes: &[u8]) -> bool {
            self.0
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(bytes).into_owned());
            true
        }
        fn resize(&self, cols: u16, rows: u16) -> bool {
            self.0.lock().unwrap().push(format!("{cols}x{rows}"));
            true
        }
    }

    #[test]
    fn output_without_subscriber_is_kept_and_reported_undelivered() {
        let hub = TerminalHub::new();
        let channel = hub.register("a", Arc::new(Recorder::default())).unwrap();
        assert!(!channel.output(b"hello "));
        assert!(!channel.output(b"world"));
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&received);
        let mut subscription = hub
            .subscribe(
                "a",
                Arc::new(move |event| {
                    if let TerminalEvent::Output(bytes) = event {
                        sink.lock().unwrap().extend_from_slice(bytes);
                    }
                }),
            )
            .unwrap();
        assert_eq!(subscription.take_backlog(), b"hello world");
        assert!(channel.output(b"!"));
        assert_eq!(*received.lock().unwrap(), b"!");
        assert_eq!(
            hub.subscribe("a", Arc::new(|_| {})).err(),
            Some(SubscribeError::Busy)
        );
        drop(subscription);
        assert!(!channel.has_subscriber());
        assert!(!channel.output(b"?"));
    }

    #[test]
    fn the_ring_is_bounded() {
        let hub = TerminalHub::new();
        let channel = hub.register("a", Arc::new(Recorder::default())).unwrap();
        channel.output(&vec![b'x'; RING_BYTES - 1]);
        channel.output(b"abc");
        let subscription = hub.subscribe("a", Arc::new(|_| {})).unwrap();
        assert_eq!(subscription.backlog().len(), RING_BYTES);
        assert!(subscription.backlog().ends_with(b"xabc"));
        drop(subscription);
        channel.output(&vec![b'y'; RING_BYTES * 2]);
        let subscription = hub.subscribe("a", Arc::new(|_| {})).unwrap();
        assert_eq!(subscription.backlog().len(), RING_BYTES);
    }

    #[test]
    fn writes_and_resizes_reach_the_producer_and_exit_reaches_the_consumer() {
        let hub = TerminalHub::new();
        let input = Arc::new(Recorder::default());
        let _channel = hub
            .register("a", Arc::clone(&input) as Arc<dyn TerminalInput>)
            .unwrap();
        assert!(hub.register("a", Arc::new(Recorder::default())).is_none());
        assert!(hub.write("a", b"ls\n"));
        assert!(hub.resize("a", 120, 40));
        assert!(!hub.write("missing", b"x"));
        assert_eq!(*input.0.lock().unwrap(), vec!["ls\n", "120x40"]);
        let exited = Arc::new(Mutex::new(None));
        let sink = Arc::clone(&exited);
        let _subscription = hub
            .subscribe(
                "a",
                Arc::new(move |event| {
                    if let TerminalEvent::Exited(status) = event {
                        *sink.lock().unwrap() = Some(status);
                    }
                }),
            )
            .unwrap();
        hub.unregister("a", Some(3));
        assert_eq!(*exited.lock().unwrap(), Some(Some(3)));
        assert!(!hub.contains("a"));
        assert_eq!(
            hub.subscribe("a", Arc::new(|_| {})).err(),
            Some(SubscribeError::Unknown)
        );
    }
}
