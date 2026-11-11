//! The tokio runtime of the `ssh` module.
//!
//! It starts with the first lease (a `connect`) and stops when the last one
//! is released (the last session ended), so an application without an open
//! session has no `pam-ssh` thread.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::oneshot;

/// Worker threads of the runtime. Key exchange and ciphers are cheap next to
/// the network; two threads keep one busy transfer from starving terminals.
const WORKERS: usize = 2;
/// Threads for local file work of transfers.
const BLOCKING: usize = 4;

struct Running {
    handle: Handle,
    leases: usize,
    stop: oneshot::Sender<()>,
}

#[derive(Default)]
pub struct Runtime {
    running: Mutex<Option<Running>>,
}

/// Keeps the runtime alive while held.
pub struct Lease {
    runtime: Arc<Runtime>,
    handle: Handle,
}

impl Lease {
    pub fn handle(&self) -> &Handle {
        &self.handle
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.runtime.release();
    }
}

impl Runtime {
    /// Takes a lease, starting the runtime when none is running.
    pub fn lease(self: &Arc<Self>) -> Result<Lease, &'static str> {
        let mut running = self
            .running
            .lock()
            .map_err(|_| "ssh: runtime unavailable")?;
        if let Some(running) = running.as_mut() {
            running.leases += 1;
            return Ok(Lease {
                runtime: Arc::clone(self),
                handle: running.handle.clone(),
            });
        }
        let (stop, stopped) = oneshot::channel::<()>();
        let (started, handle) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("pam-ssh".into())
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(WORKERS)
                    .max_blocking_threads(BLOCKING)
                    .thread_keep_alive(Duration::from_secs(10))
                    .thread_name("pam-ssh")
                    .enable_all()
                    .build()
                else {
                    return;
                };
                if started.send(runtime.handle().clone()).is_err() {
                    return;
                }
                let _ = runtime.block_on(stopped);
                // Tasks still running (a session that was told to stop and is
                // saying goodbye to its server) get a moment, then are dropped.
                runtime.shutdown_timeout(Duration::from_secs(2));
            })
            .map_err(|_| "ssh: runtime unavailable")?;
        let handle = handle.recv().map_err(|_| "ssh: runtime unavailable")?;
        *running = Some(Running {
            handle: handle.clone(),
            leases: 1,
            stop,
        });
        Ok(Lease {
            runtime: Arc::clone(self),
            handle,
        })
    }

    fn release(&self) {
        let Ok(mut running) = self.running.lock() else {
            return;
        };
        let last = running.as_mut().is_some_and(|running| {
            running.leases -= 1;
            running.leases == 0
        });
        if last && let Some(running) = running.take() {
            let _ = running.stop.send(());
        }
    }

    /// Whether the runtime threads exist.
    #[cfg(test)]
    pub fn is_running(&self) -> bool {
        self.running.lock().is_ok_and(|running| running.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_with_the_first_lease_and_stops_with_the_last() {
        let runtime = Arc::new(Runtime::default());
        assert!(!runtime.is_running());
        let first = runtime.lease().unwrap();
        let second = runtime.lease().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        first.handle().spawn(async move {
            let name = std::thread::current().name().map(str::to_owned);
            let _ = sender.send(name);
        });
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
            Some(String::from("pam-ssh"))
        );
        drop(first);
        assert!(runtime.is_running());
        drop(second);
        assert!(!runtime.is_running());
        let third = runtime.lease().unwrap();
        assert!(runtime.is_running());
        drop(third);
        assert!(!runtime.is_running());
    }
}
