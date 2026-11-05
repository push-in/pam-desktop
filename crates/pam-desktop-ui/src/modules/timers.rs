//! `timers`: one-shot delays. Intervals are re-armed by PHP after every tick.

use std::collections::{BinaryHeap, HashMap};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::{request_map, Module, Reply};

const MAX_DELAY_MS: i64 = 86_400_000;

struct Pending {
    reply: Reply,
    /// The PHP timer id, when the delay belongs to a cancellable timer.
    timer: Option<i64>,
}

#[derive(Default)]
struct State {
    /// Deadlines, earliest first (`Reverse` ordering through negated keys).
    queue: BinaryHeap<std::cmp::Reverse<(Instant, u64)>>,
    pending: HashMap<u64, Pending>,
    /// Sequence of the delay currently armed for each PHP timer id.
    by_timer: HashMap<i64, u64>,
    next: u64,
    closed: bool,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

pub struct Timers {
    shared: Arc<Shared>,
}

impl Default for Timers {
    fn default() -> Self {
        Self::new()
    }
}

impl Timers {
    #[must_use]
    pub fn new() -> Self {
        let shared = Arc::new(Shared { state: Mutex::new(State::default()), wake: Condvar::new() });
        let worker = Arc::clone(&shared);
        let _ = thread::Builder::new().name("pam-timers".into()).spawn(move || run(&worker));
        Self { shared }
    }
}

fn run(shared: &Shared) {
    let Ok(mut state) = shared.state.lock() else { return };
    loop {
        if state.closed {
            return;
        }
        let now = Instant::now();
        // Deliver everything that is due, in deadline order.
        let mut due = Vec::new();
        while let Some(std::cmp::Reverse((deadline, sequence))) = state.queue.peek().copied() {
            if deadline > now {
                break;
            }
            state.queue.pop();
            if let Some(pending) = state.pending.remove(&sequence) {
                if let Some(timer) = pending.timer {
                    if state.by_timer.get(&timer) == Some(&sequence) {
                        state.by_timer.remove(&timer);
                    }
                }
                due.push(pending.reply);
            }
        }
        if !due.is_empty() {
            drop(state);
            for reply in due {
                reply.done();
            }
            state = match shared.state.lock() {
                Ok(state) => state,
                Err(_) => return,
            };
            continue;
        }
        state = match state.queue.peek().copied() {
            Some(std::cmp::Reverse((deadline, _))) => {
                let wait = deadline.saturating_duration_since(Instant::now());
                match shared.wake.wait_timeout(state, wait) {
                    Ok((state, _)) => state,
                    Err(_) => return,
                }
            }
            None => match shared.wake.wait(state) {
                Ok(state) => state,
                Err(_) => return,
            },
        };
    }
}

impl Module for Timers {
    fn call(&mut self, method: &str, payload: &[u8], reply: Reply) {
        let request = match request_map(payload) {
            Ok(request) => request,
            Err(error) => return reply.failure(error),
        };
        let Ok(mut state) = self.shared.state.lock() else { return reply.failure("Timers module is closed") };
        match method {
            "after" => {
                let delay = request.integer("milliseconds").unwrap_or(0).clamp(0, MAX_DELAY_MS) as u64;
                let timer = request.integer("timer");
                state.next += 1;
                let sequence = state.next;
                if let Some(timer) = timer {
                    // Re-arming a timer replaces its pending delay without answering it.
                    if let Some(previous) = state.by_timer.insert(timer, sequence) {
                        state.pending.remove(&previous);
                    }
                }
                state.pending.insert(sequence, Pending { reply, timer });
                state.queue.push(std::cmp::Reverse((Instant::now() + Duration::from_millis(delay), sequence)));
                drop(state);
                self.shared.wake.notify_one();
            }
            "cancel" => {
                let Some(timer) = request.integer("timer") else { return reply.failure("Timer id is required") };
                let cancelled = state.by_timer.remove(&timer).and_then(|sequence| state.pending.remove(&sequence));
                drop(state);
                if let Some(pending) = cancelled {
                    pending.reply.failure("Timer cancelled");
                }
                reply.done();
            }
            other => reply.failure(format!("Unknown timers method {other}")),
        }
    }

    fn reset(&mut self) {
        // Delays of the previous worker are dropped without an answer.
        if let Ok(mut state) = self.shared.state.lock() {
            state.queue.clear();
            state.pending.clear();
            state.by_timer.clear();
        }
    }
}

impl Drop for Timers {
    fn drop(&mut self) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.closed = true;
        }
        self.shared.wake.notify_one();
    }
}
