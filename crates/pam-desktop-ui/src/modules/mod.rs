//! Native modules: what PHP reaches through `pam_desktop_call`.
//!
//! A module answers each request exactly once through its [`Reply`], from any
//! thread and at any later time (a timer firing, a download finishing). The
//! reply is tagged with the worker generation that asked, so results of a
//! worker that was hot-reloaded away are dropped.

pub mod channel;
pub mod fetch;
pub mod files;
pub mod http;
pub mod image;
pub mod storage;
pub mod support;
pub mod system;
pub mod test_support;
pub mod timers;

use std::collections::HashMap;
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;

use crate::runtime::{Inbound, Inbox, MODULE_FAILURE, MODULE_SUCCESS};
use crate::wiremap::WireMap;

enum Sink {
    /// The runtime's queue: the result reaches the PHP worker.
    Inbox { inbox: Arc<Inbox>, generation: u64, request: u64 },
    /// A plain channel (tests of modules without a runtime).
    Channel(Sender<ModuleResult>),
}

/// What a module answered: the status (`MODULE_SUCCESS` or `MODULE_FAILURE`)
/// and the payload bytes.
pub type ModuleResult = (u8, Vec<u8>);

/// The pending answer of one module request.
pub struct Reply {
    sink: Sink,
}

impl Reply {
    #[must_use]
    pub fn new(inbox: Arc<Inbox>, generation: u64, request: u64) -> Self {
        Self { sink: Sink::Inbox { inbox, generation, request } }
    }

    /// A reply whose answer can be read from the returned receiver.
    #[must_use]
    pub fn channel() -> (Self, mpsc::Receiver<ModuleResult>) {
        let (sender, receiver) = mpsc::channel();
        (Self { sink: Sink::Channel(sender) }, receiver)
    }

    fn send(self, status: u8, payload: Vec<u8>) {
        match self.sink {
            Sink::Inbox { inbox, generation, request } => {
                inbox.post(Inbound::ModuleResult { generation, request, status, payload });
            }
            Sink::Channel(sender) => {
                let _ = sender.send((status, payload));
            }
        }
    }

    /// Success with a raw payload.
    pub fn success(self, payload: impl Into<Vec<u8>>) {
        self.send(MODULE_SUCCESS, payload.into());
    }

    /// Success with an empty payload.
    pub fn done(self) {
        self.send(MODULE_SUCCESS, Vec::new());
    }

    /// Success with a WireMap.
    pub fn map(self, map: &WireMap) {
        self.send(MODULE_SUCCESS, map.encode());
    }

    /// Failure with a human readable message (PHP surfaces it as an exception).
    pub fn failure(self, message: impl AsRef<str>) {
        self.send(MODULE_FAILURE, message.as_ref().as_bytes().to_vec());
    }

    /// Answers from a `Result`.
    pub fn result(self, result: Result<Vec<u8>, String>) {
        match result {
            Ok(payload) => self.success(payload),
            Err(message) => self.failure(message),
        }
    }
}

/// Decodes a request payload; an empty payload is an empty map.
pub fn request_map(payload: &[u8]) -> Result<WireMap, String> {
    if payload.is_empty() {
        return Ok(WireMap::new());
    }
    WireMap::decode(payload).map_err(|error| error.to_string())
}

pub trait Module {
    /// Handles `method`; must eventually consume `reply` (or drop it to never answer).
    fn call(&mut self, method: &str, payload: &[u8], reply: Reply);

    /// The PHP worker restarted: forget subscriptions and parked waiters.
    fn reset(&mut self) {}
}

#[derive(Default)]
pub struct Registry {
    modules: HashMap<String, Box<dyn Module>>,
}

impl Registry {
    pub fn register(&mut self, name: &str, module: impl Module + 'static) {
        self.modules.insert(name.to_owned(), Box::new(module));
    }

    pub fn register_boxed(&mut self, name: &str, module: Box<dyn Module>) {
        self.modules.insert(name.to_owned(), module);
    }

    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.modules.contains_key(name)
    }

    /// Routes a call; returns the reply back when no module has that name.
    pub fn call(&mut self, module: &str, method: &str, payload: &[u8], reply: Reply) -> Option<Reply> {
        match self.modules.get_mut(module) {
            Some(handler) => {
                handler.call(method, payload, reply);
                None
            }
            None => Some(reply),
        }
    }

    pub fn reset(&mut self) {
        for module in self.modules.values_mut() {
            module.reset();
        }
    }
}

type Job = Box<dyn FnOnce() + Send>;

/// How long a serial worker keeps its thread without work.
const SERIAL_IDLE: std::time::Duration = std::time::Duration::from_secs(30);

/// Runs jobs one at a time, in order, off the UI thread: operations of a
/// storage-like module complete in call order without blocking the UI.
///
/// The thread starts with the first job and exits after a while without
/// work, so a module nobody uses costs nothing.
pub struct SerialWorker {
    name: String,
    /// The queue of the running thread; `None` while no thread exists.
    jobs: std::sync::Arc<std::sync::Mutex<Option<Sender<Job>>>>,
}

impl SerialWorker {
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self { name: name.to_owned(), jobs: std::sync::Arc::new(std::sync::Mutex::new(None)) }
    }

    pub fn run(&self, job: impl FnOnce() + Send + 'static) {
        let mut job: Job = Box::new(job);
        // Senders and the exiting thread meet on this lock, so a job is
        // never left in the queue of a thread that already decided to stop.
        let Ok(mut slot) = self.jobs.lock() else { return };
        if let Some(sender) = slot.as_ref() {
            match sender.send(job) {
                Ok(()) => return,
                Err(mpsc::SendError(returned)) => job = returned,
            }
        }
        let (sender, receiver) = mpsc::channel::<Job>();
        if sender.send(job).is_err() {
            return;
        }
        let jobs = std::sync::Arc::clone(&self.jobs);
        let spawned = thread::Builder::new().name(self.name.clone()).spawn(move || loop {
            match receiver.recv_timeout(SERIAL_IDLE) {
                Ok(job) => job(),
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let Ok(mut slot) = jobs.lock() else { return };
                    match receiver.try_recv() {
                        Ok(job) => {
                            drop(slot);
                            job();
                        }
                        Err(_) => {
                            *slot = None;
                            return;
                        }
                    }
                }
            }
        });
        *slot = spawned.is_ok().then_some(sender);
    }
}
