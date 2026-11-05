//! `updates`: signed updates of the application, as PHP sees them.
//!
//! `state` answers with the current snapshot; `check`, `download` and
//! `install` run one at a time on a worker thread and answer with the
//! snapshot they reached, or fail with the reason. `next` is a long poll
//! that answers whenever the snapshot changes, whoever changed it: a call
//! from PHP or the update policy of the manifest working on its own.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use pam_desktop_ui::modules::channel::WatchChannel;
use pam_desktop_ui::modules::fetch::HttpClient;
use pam_desktop_ui::modules::{Module, Reply, SerialWorker};
use pam_desktop_ui::wiremap::WireMap;

use crate::bundle::Installed;
use crate::manifest::{UpdateConfig, UpdatePolicy};
use crate::updater::{Snapshot, Updater};

/// Time the reply of `install` has to reach PHP before the host exits.
const EXIT_DELAY: Duration = Duration::from_millis(150);

/// The channel of the current PHP worker. A worker that restarts gets a
/// fresh one, so a read parked by the previous worker cannot swallow an
/// event meant for the new one.
struct Events {
    channel: Mutex<Arc<WatchChannel<WireMap>>>,
    policy: Option<UpdatePolicy>,
}

impl Events {
    fn current(&self) -> Arc<WatchChannel<WireMap>> {
        Arc::clone(&self.channel.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn publish(&self, snapshot: &Snapshot) {
        self.current().offer(wire(snapshot, self.policy));
    }

    fn reset(&self) {
        let fresh = Arc::new(WatchChannel::new());
        let stale = std::mem::replace(
            &mut *self.channel.lock().unwrap_or_else(PoisonError::into_inner),
            fresh,
        );
        stale.close();
    }
}

/// The snapshot as a WireMap; absent values are left out.
fn wire(snapshot: &Snapshot, policy: Option<UpdatePolicy>) -> WireMap {
    let bytes = |count: u64| i64::try_from(count).unwrap_or(i64::MAX);
    let mut map = WireMap::new()
        .with("state", snapshot.status.keyword())
        .with("currentVersion", snapshot.current_version.as_str())
        .with("downloadedBytes", bytes(snapshot.downloaded_bytes))
        .with("totalBytes", bytes(snapshot.total_bytes));
    if let Some(version) = &snapshot.available_version {
        map.insert("availableVersion", version.as_str());
    }
    if let Some(url) = &snapshot.notes_url {
        map.insert("notesUrl", url.as_str());
    }
    if let Some(error) = &snapshot.error {
        map.insert("error", error.as_str());
    }
    if let Some(policy) = policy {
        map.insert("policy", policy.keyword());
    }
    map
}

pub struct Updates {
    updater: Updater,
    events: Arc<Events>,
    worker: SerialWorker,
    exit: Arc<dyn Fn() + Send + Sync>,
}

impl Updates {
    /// `config` is the `updates` block of the manifest and `installed` the
    /// bundle the host runs from. Outside development the update policy
    /// starts right away; `exit` ends the host once an installation began.
    #[must_use]
    pub fn new(
        application_id: &str,
        version: &str,
        config: Option<UpdateConfig>,
        installed: Option<Installed>,
        client: Arc<dyn HttpClient>,
        development: bool,
        exit: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let events = Arc::new(Events {
            channel: Mutex::new(Arc::new(WatchChannel::new())),
            policy: config.as_ref().map(|config| config.policy),
        });
        let listener = Arc::clone(&events);
        let updater = Updater::new(
            application_id,
            version,
            config,
            installed,
            client,
            move |snapshot| listener.publish(snapshot),
        );
        let module = Self {
            updater,
            events,
            worker: SerialWorker::new("pam-updates"),
            exit: Arc::new(exit),
        };
        // A development run is not an installed bundle: there is nothing an
        // unattended check could lead to.
        if !development
            && module
                .updater
                .policy()
                .is_some_and(|policy| policy != UpdatePolicy::Manual)
        {
            let (updater, exit) = (module.updater.clone(), Arc::clone(&module.exit));
            module.worker.run(move || {
                if updater.run_policy() {
                    std::thread::sleep(EXIT_DELAY);
                    exit();
                }
            });
        }
        module
    }

    fn operation(
        &self,
        reply: Reply,
        operation: fn(&Updater) -> Result<Snapshot, String>,
        exits: bool,
    ) {
        let (updater, exit, policy) = (
            self.updater.clone(),
            Arc::clone(&self.exit),
            self.events.policy,
        );
        self.worker.run(move || match operation(&updater) {
            Ok(snapshot) => {
                reply.map(&wire(&snapshot, policy));
                if exits {
                    std::thread::sleep(EXIT_DELAY);
                    exit();
                }
            }
            Err(message) => reply.failure(message),
        });
    }
}

impl Module for Updates {
    fn call(&mut self, method: &str, _payload: &[u8], reply: Reply) {
        match method {
            "state" => reply.map(&wire(&self.updater.snapshot(), self.events.policy)),
            "check" => self.operation(reply, Updater::check, false),
            "download" => self.operation(reply, Updater::download, false),
            "install" => self.operation(reply, Updater::install, true),
            "next" => self.events.current().next_reply(reply),
            _ => reply.failure("Unknown updates method"),
        }
    }

    fn reset(&mut self) {
        self.events.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http_client::UreqClient;
    use crate::updater::Status;
    use pam_desktop_ui::modules::channel::{PENDING_READ, STOPPED};
    use pam_desktop_ui::modules::test_support::{Served, TestServer};
    use pam_desktop_ui::runtime::{MODULE_FAILURE, MODULE_SUCCESS};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::Receiver;

    const WAIT: Duration = Duration::from_secs(10);

    fn call(module: &mut Updates, method: &str) -> Receiver<(u8, Vec<u8>)> {
        let (reply, answer) = Reply::channel();
        module.call(method, &[], reply);
        answer
    }

    fn success(answer: &Receiver<(u8, Vec<u8>)>) -> WireMap {
        let (status, payload) = answer.recv_timeout(WAIT).expect("an answer");
        assert_eq!(
            status,
            MODULE_SUCCESS,
            "{}",
            String::from_utf8_lossy(&payload)
        );
        WireMap::decode(&payload).expect("a WireMap")
    }

    fn failure(answer: &Receiver<(u8, Vec<u8>)>) -> String {
        let (status, payload) = answer.recv_timeout(WAIT).expect("an answer");
        assert_eq!(status, MODULE_FAILURE);
        String::from_utf8(payload).expect("a message")
    }

    fn module(
        config: Option<UpdateConfig>,
        development: bool,
        exits: &Arc<AtomicUsize>,
    ) -> Updates {
        let exits = Arc::clone(exits);
        Updates::new(
            "com.pushin.test",
            "1.0.0",
            config,
            None,
            Arc::new(UreqClient::new()),
            development,
            move || {
                exits.fetch_add(1, Ordering::SeqCst);
            },
        )
    }

    #[test]
    fn a_manifest_without_updates_answers_disabled() {
        let exits = Arc::new(AtomicUsize::new(0));
        let mut module = module(None, false, &exits);
        let state = success(&call(&mut module, "state"));
        assert_eq!(state.text("state"), Some(Status::Disabled.keyword()));
        assert_eq!(state.text("currentVersion"), Some("1.0.0"));
        assert_eq!(state.integer("totalBytes"), Some(0));
        assert_eq!(state.text("policy"), None);
        assert_eq!(state.text("availableVersion"), None);
        assert!(failure(&call(&mut module, "check")).contains("not configured"));
        assert!(failure(&call(&mut module, "install")).contains("No verified update"));
        assert!(failure(&call(&mut module, "restart")).contains("Unknown"));
        assert_eq!(exits.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn the_long_poll_reports_changes_and_restarts_with_the_worker() {
        let server = TestServer::start(|_| Served::ok(b"not a feed".to_vec()));
        let config = UpdateConfig {
            endpoint: server.url("/feed"),
            public_key: "ab".repeat(32),
            channel: "stable".into(),
            policy: UpdatePolicy::Manual,
        };
        let exits = Arc::new(AtomicUsize::new(0));
        let mut module = module(Some(config), false, &exits);
        assert_eq!(
            success(&call(&mut module, "state")).text("policy"),
            Some("manual")
        );
        // The manual policy asks for nothing by itself.
        assert!(server.requests().is_empty());

        let parked = call(&mut module, "next");
        assert_eq!(failure(&call(&mut module, "next")), PENDING_READ);
        assert!(failure(&call(&mut module, "check")).contains("not valid"));
        assert_eq!(
            success(&parked).text("state"),
            Some(Status::Checking.keyword())
        );
        let failed = success(&call(&mut module, "next"));
        assert_eq!(failed.text("state"), Some(Status::Failed.keyword()));
        assert!(
            failed
                .text("error")
                .is_some_and(|error| error.contains("not valid"))
        );

        // A restarted worker starts a new poll; the old one ends.
        let stale = call(&mut module, "next");
        module.reset();
        assert_eq!(failure(&stale), STOPPED);
        let fresh = call(&mut module, "next");
        assert!(failure(&call(&mut module, "check")).contains("not valid"));
        assert_eq!(
            success(&fresh).text("state"),
            Some(Status::Checking.keyword())
        );
        assert_eq!(exits.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn the_policy_checks_at_startup_outside_development() {
        let server = TestServer::start(|_| Served::ok(b"not a feed".to_vec()));
        let config = |policy| UpdateConfig {
            endpoint: server.url("/feed"),
            public_key: "ab".repeat(32),
            channel: "stable".into(),
            policy,
        };
        let exits = Arc::new(AtomicUsize::new(0));

        let mut development = module(Some(config(UpdatePolicy::Automatic)), true, &exits);
        // The worker is serial: once this answers, a startup check would have run.
        assert!(failure(&call(&mut development, "download")).contains("Check for updates first"));
        assert!(server.requests().is_empty());

        let mut installed = module(Some(config(UpdatePolicy::Notify)), false, &exits);
        assert!(failure(&call(&mut installed, "download")).contains("Check for updates first"));
        assert_eq!(server.requests().len(), 1);
        assert_eq!(
            success(&call(&mut installed, "state")).text("state"),
            Some(Status::Failed.keyword())
        );
        assert_eq!(exits.load(Ordering::SeqCst), 0);
    }
}
