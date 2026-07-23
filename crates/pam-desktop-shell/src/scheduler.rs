use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, TryLockError};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pam_desktop_protocol::{
    BackgroundJobConfig, ClientEvent, ErrorCode, JOB_COMMAND, JobOverlapPolicy, MAIN_WINDOW_ID,
    ResponseStatus,
};
use serde_json::Value;
use winit::event_loop::EventLoopProxy;

use crate::event_hub::EventHub;
use crate::host_event::HostEvent;
use crate::worker::{CancellationToken, WorkerRequestError, WorkerSupervisor};

static NEXT_RUN_ID: AtomicU64 = AtomicU64::new(1);

pub struct BackgroundScheduler {
    stop: Arc<(Mutex<bool>, Condvar)>,
    cancellations: Vec<CancellationToken>,
    threads: Vec<JoinHandle<()>>,
}

impl BackgroundScheduler {
    pub fn start(
        jobs: &[BackgroundJobConfig],
        supervisor: &Arc<Mutex<WorkerSupervisor>>,
        events: &EventHub,
        event_proxy: &EventLoopProxy<HostEvent>,
    ) -> Result<Self, String> {
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let mut cancellations = Vec::with_capacity(jobs.len());
        let mut threads = Vec::with_capacity(jobs.len());
        for job in jobs {
            let job = job.clone();
            let job_supervisor = Arc::clone(supervisor);
            let job_events = events.clone();
            let job_proxy = event_proxy.clone();
            let job_stop = stop.clone();
            let cancellation = CancellationToken::default();
            cancellations.push(cancellation.clone());
            let job_id = job.id.clone();
            let thread = std::thread::Builder::new()
                .name(format!("pam-job-{}", job.id))
                .spawn(move || {
                    if wait_or_stop(&job_stop, Duration::from_millis(job.initial_delay_ms)) {
                        return;
                    }
                    loop {
                        run_job(
                            &job,
                            &job_supervisor,
                            &job_events,
                            &job_proxy,
                            &cancellation,
                            &job_stop,
                        );
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
    event_proxy: &EventLoopProxy<HostEvent>,
    cancellation: &CancellationToken,
    stop: &(Mutex<bool>, Condvar),
) {
    if stopped(stop) {
        return;
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
                return;
            }
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
        },
        JobOverlapPolicy::Wait => supervisor
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    };
    if stopped(stop) {
        return;
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
            if !response.effects.is_empty() {
                let _ = event_proxy.send_event(HostEvent::ApplyEffects(response.effects));
            }
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
        }
        Err(error) => {
            let code = match error {
                WorkerRequestError::TimedOut => ErrorCode::RequestTimedOut,
                WorkerRequestError::Cancelled => ErrorCode::RequestCancelled,
                WorkerRequestError::Crashed(_) => ErrorCode::WorkerCrashed,
            };
            publish_failure(events, job, run_id, code, &error.to_string());
        }
    }
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
}
