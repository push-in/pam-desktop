use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, TryLockError};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pam_desktop_protocol::{
    BackgroundJobConfig, ClientEvent, Effect, ErrorCode, JOB_COMMAND, JobOverlapPolicy,
    MAIN_WINDOW_ID, ResponseStatus,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use winit::event_loop::EventLoopProxy;

use crate::event_hub::EventHub;
use crate::host_event::HostEvent;
use crate::worker::{CancellationToken, WorkerRequestError, WorkerSupervisor};

static NEXT_RUN_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum JobRunState {
    Pending = 1,
    Running = 2,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct JobState {
    state: u8,
    next_run_at_ms: u64,
    attempts: u8,
}

struct JobJournal {
    path: PathBuf,
    states: Mutex<HashMap<String, JobState>>,
}

impl JobJournal {
    fn open(application_id: &str) -> Result<Self, String> {
        Self::open_at(job_data_path(application_id)?)
    }

    fn open_at(path: PathBuf) -> Result<Self, String> {
        let states = if path.is_file() {
            let bytes = std::fs::read(&path)
                .map_err(|error| format!("cannot read job journal: {error}"))?;
            serde_json::from_slice(&bytes)
                .map_err(|error| format!("cannot decode job journal: {error}"))?
        } else {
            HashMap::new()
        };
        Ok(Self {
            path,
            states: Mutex::new(states),
        })
    }

    fn initial_delay(&self, job: &BackgroundJobConfig) -> Duration {
        if !job.persistent {
            return Duration::from_millis(job.initial_delay_ms);
        }
        let states = self
            .states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(state) = states.get(&job.id) else {
            return Duration::from_millis(job.initial_delay_ms);
        };
        if state.state == JobRunState::Running as u8 {
            return Duration::ZERO;
        }
        Duration::from_millis(state.next_run_at_ms.saturating_sub(now_ms()))
    }

    fn started(&self, job: &BackgroundJobConfig, attempt: u8) -> Result<(), String> {
        if !job.persistent {
            return Ok(());
        }
        self.update(
            &job.id,
            JobState {
                state: JobRunState::Running as u8,
                next_run_at_ms: 0,
                attempts: attempt,
            },
        )
    }

    fn finished(&self, job: &BackgroundJobConfig) -> Result<(), String> {
        if !job.persistent {
            return Ok(());
        }
        self.update(
            &job.id,
            JobState {
                state: JobRunState::Pending as u8,
                next_run_at_ms: now_ms().saturating_add(job.interval_ms),
                attempts: 0,
            },
        )
    }

    fn update(&self, id: &str, state: JobState) -> Result<(), String> {
        let mut states = self
            .states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        states.insert(id.to_owned(), state);
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create job journal directory: {error}"))?;
        }
        let bytes = serde_json::to_vec(&*states)
            .map_err(|error| format!("cannot encode job journal: {error}"))?;
        let temporary = self.path.with_extension("json.tmp");
        let mut output = std::fs::File::create(&temporary)
            .map_err(|error| format!("cannot create job journal: {error}"))?;
        output
            .write_all(&bytes)
            .and_then(|()| output.sync_all())
            .map_err(|error| format!("cannot persist job journal: {error}"))?;
        std::fs::rename(&temporary, &self.path)
            .map_err(|error| format!("cannot publish job journal: {error}"))
    }
}

pub struct BackgroundScheduler {
    stop: Arc<(Mutex<bool>, Condvar)>,
    cancellations: Vec<CancellationToken>,
    threads: Vec<JoinHandle<()>>,
}

#[derive(Clone)]
enum EffectDispatcher {
    Ui(EventLoopProxy<HostEvent>),
    Headless,
}

impl EffectDispatcher {
    fn dispatch(&self, effects: Vec<Effect>) {
        if effects.is_empty() {
            return;
        }
        if let Self::Ui(proxy) = self {
            let _ = proxy.send_event(HostEvent::ApplyEffects(effects));
        }
    }
}

impl BackgroundScheduler {
    pub fn start(
        jobs: &[BackgroundJobConfig],
        application_id: &str,
        supervisor: &Arc<Mutex<WorkerSupervisor>>,
        events: &EventHub,
        event_proxy: &EventLoopProxy<HostEvent>,
    ) -> Result<Self, String> {
        Self::start_with_dispatcher(
            jobs,
            application_id,
            supervisor,
            events,
            &EffectDispatcher::Ui(event_proxy.clone()),
        )
    }

    pub fn start_headless(
        jobs: &[BackgroundJobConfig],
        application_id: &str,
        supervisor: &Arc<Mutex<WorkerSupervisor>>,
        events: &EventHub,
    ) -> Result<Self, String> {
        Self::start_with_dispatcher(
            jobs,
            application_id,
            supervisor,
            events,
            &EffectDispatcher::Headless,
        )
    }

    fn start_with_dispatcher(
        jobs: &[BackgroundJobConfig],
        application_id: &str,
        supervisor: &Arc<Mutex<WorkerSupervisor>>,
        events: &EventHub,
        effect_dispatcher: &EffectDispatcher,
    ) -> Result<Self, String> {
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let journal = Arc::new(JobJournal::open(application_id)?);
        let mut cancellations = Vec::with_capacity(jobs.len());
        let mut threads = Vec::with_capacity(jobs.len());
        for job in jobs {
            let job = job.clone();
            let job_supervisor = Arc::clone(supervisor);
            let job_events = events.clone();
            let job_effects = effect_dispatcher.clone();
            let job_stop = stop.clone();
            let cancellation = CancellationToken::default();
            let job_journal = journal.clone();
            cancellations.push(cancellation.clone());
            let job_id = job.id.clone();
            let thread = std::thread::Builder::new()
                .name(format!("pam-job-{}", job.id))
                .spawn(move || {
                    if wait_or_stop(&job_stop, job_journal.initial_delay(&job)) {
                        return;
                    }
                    loop {
                        for attempt in 1..=job.maximum_attempts {
                            if let Err(error) = job_journal.started(&job, attempt) {
                                publish(
                                    &job_events,
                                    "pam.job.journal-failed",
                                    serde_json::json!({"id": job.id, "message": error}),
                                );
                                return;
                            }
                            if run_job(
                                &job,
                                &job_supervisor,
                                &job_events,
                                &job_effects,
                                &cancellation,
                                &job_stop,
                            ) {
                                break;
                            }
                            if attempt < job.maximum_attempts {
                                let multiplier = 1_u64 << u32::from(attempt.saturating_sub(1));
                                if wait_or_stop(
                                    &job_stop,
                                    Duration::from_millis(
                                        job.retry_backoff_ms.saturating_mul(multiplier),
                                    ),
                                ) {
                                    return;
                                }
                            }
                        }
                        if let Err(error) = job_journal.finished(&job) {
                            publish(
                                &job_events,
                                "pam.job.journal-failed",
                                serde_json::json!({"id": job.id, "message": error}),
                            );
                            return;
                        }
                        if wait_or_stop(&job_stop, Duration::from_millis(job.interval_ms)) {
                            return;
                        }
                    }
                })
                .map_err(|error| {
                    format!("cannot start background job thread for {job_id:?}: {error}")
                })?;
            threads.push(thread);
        }
        Ok(Self {
            stop,
            cancellations,
            threads,
        })
    }
}

impl Drop for BackgroundScheduler {
    fn drop(&mut self) {
        {
            let (lock, changed) = &*self.stop;
            *lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
            changed.notify_all();
        }
        for cancellation in &self.cancellations {
            cancellation.cancel();
        }
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

fn run_job(
    job: &BackgroundJobConfig,
    supervisor: &Mutex<WorkerSupervisor>,
    events: &EventHub,
    effect_dispatcher: &EffectDispatcher,
    cancellation: &CancellationToken,
    stop: &(Mutex<bool>, Condvar),
) -> bool {
    if stopped(stop) {
        return false;
    }
    let run_id = NEXT_RUN_ID.fetch_add(1, Ordering::Relaxed);
    let started_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());

    let mut supervisor = match job.overlap {
        JobOverlapPolicy::Skip => match supervisor.try_lock() {
            Ok(supervisor) => supervisor,
            Err(TryLockError::WouldBlock) => {
                publish(
                    events,
                    "pam.job.skipped",
                    serde_json::json!({"id": job.id, "runId": run_id, "reason": 1}),
                );
                return true;
            }
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
        },
        JobOverlapPolicy::Wait => supervisor
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    };
    if stopped(stop) {
        return false;
    }
    publish(
        events,
        "pam.job.started",
        serde_json::json!({
            "id": job.id,
            "runId": run_id,
            "startedAtMs": started_at_ms,
        }),
    );
    let result = supervisor.request(
        JOB_COMMAND,
        MAIN_WINDOW_ID,
        serde_json::json!({
            "id": job.id,
            "runId": run_id,
            "startedAtMs": started_at_ms,
        }),
        Duration::from_millis(job.timeout_ms),
        cancellation,
    );
    drop(supervisor);

    match result {
        Ok(response) if response.status == ResponseStatus::Success => {
            effect_dispatcher.dispatch(response.effects);
            for event in response.events {
                events.publish(event);
            }
            publish(
                events,
                "pam.job.completed",
                serde_json::json!({
                    "id": job.id,
                    "runId": run_id,
                    "result": response.payload,
                }),
            );
            true
        }
        Ok(response) => {
            let message = response.error.map_or_else(
                || "PHP background job returned an unspecified failure.".to_owned(),
                |error| error.message,
            );
            publish_failure(
                events,
                job,
                run_id,
                ErrorCode::BackgroundJobFailed,
                &message,
            );
            false
        }
        Err(error) => {
            let code = match error {
                WorkerRequestError::TimedOut => ErrorCode::RequestTimedOut,
                WorkerRequestError::Cancelled => ErrorCode::RequestCancelled,
                WorkerRequestError::Crashed(_) => ErrorCode::WorkerCrashed,
            };
            publish_failure(events, job, run_id, code, &error.to_string());
            false
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

fn job_data_path(application_id: &str) -> Result<PathBuf, String> {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
    }
    .ok_or_else(|| "cannot locate operating-system data directory for jobs".to_owned())?;
    Ok(base
        .join("pam-desktop")
        .join(application_id)
        .join("jobs.json"))
}

fn publish_failure(
    events: &EventHub,
    job: &BackgroundJobConfig,
    run_id: u64,
    code: ErrorCode,
    message: &str,
) {
    publish(
        events,
        "pam.job.failed",
        serde_json::json!({
            "id": job.id,
            "runId": run_id,
            "code": code as u16,
            "message": message,
        }),
    );
}

fn publish(events: &EventHub, name: &str, payload: Value) {
    events.publish(ClientEvent {
        name: name.to_owned(),
        payload,
        window_id: None,
    });
}

fn wait_or_stop(stop: &(Mutex<bool>, Condvar), duration: Duration) -> bool {
    let (lock, changed) = stop;
    let guard = lock
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if *guard {
        return true;
    }
    let (guard, _) = changed
        .wait_timeout(guard, duration)
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *guard
}

fn stopped(stop: &(Mutex<bool>, Condvar)) -> bool {
    *stop
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_wait_can_be_interrupted_without_sleeping_the_interval() {
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let waiting = stop.clone();
        let thread = std::thread::spawn(move || wait_or_stop(&waiting, Duration::from_secs(60)));
        {
            let (lock, changed) = &*stop;
            *lock.lock().expect("stop lock should work") = true;
            changed.notify_all();
        }
        assert!(thread.join().expect("waiter should finish"));
    }

    #[test]
    fn persistent_journal_recovers_interrupted_jobs_immediately() {
        let root =
            std::env::temp_dir().join(format!("pam-journal-{}-{}", std::process::id(), now_ms()));
        let path = root.join("jobs.json");
        let job = BackgroundJobConfig {
            id: "sync".to_owned(),
            interval_ms: 60_000,
            initial_delay_ms: 30_000,
            timeout_ms: 5_000,
            overlap: JobOverlapPolicy::Skip,
            persistent: true,
            maximum_attempts: 3,
            retry_backoff_ms: 500,
        };
        let journal = JobJournal::open_at(path.clone()).expect("journal should open");
        journal
            .started(&job, 1)
            .expect("running state should be durable");
        assert!(path.is_file());
        drop(journal);
        let recovered = JobJournal::open_at(path).expect("journal should reopen");
        assert_eq!(recovered.initial_delay(&job), Duration::ZERO);
        recovered
            .finished(&job)
            .expect("completion state should be durable");
        assert!(recovered.initial_delay(&job) > Duration::from_secs(50));
        std::fs::remove_dir_all(root).expect("journal fixture should be removable");
    }
}
