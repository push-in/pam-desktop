use std::ffi::{OsStr, OsString};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pam_desktop_protocol::{
    BOOT_COMMAND, Bootstrap, MAIN_WINDOW_ID, MAX_MESSAGE_BYTES, RequestEnvelope, ResponseEnvelope,
    ResponseStatus,
};
use serde_json::Value;

use crate::project::Project;

const BOOT_TIMEOUT: Duration = Duration::from_secs(10);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Debug)]
pub enum WorkerRequestError {
    TimedOut,
    Cancelled,
    Crashed(String),
}

impl std::fmt::Display for WorkerRequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TimedOut => formatter.write_str("the PHP command exceeded its deadline"),
            Self::Cancelled => formatter.write_str("the PHP command was cancelled"),
            Self::Crashed(message) => write!(formatter, "the PHP worker stopped: {message}"),
        }
    }
}

pub struct WorkerSupervisor {
    project: Project,
    executable: OsString,
    worker: Option<WorkerClient>,
    bootstrap: Bootstrap,
    generation: u64,
}

impl WorkerSupervisor {
    pub fn start(project: Project) -> Result<Self, String> {
        let executable = std::env::var_os("PAM_BINARY").unwrap_or_else(|| "pam".into());
        Self::start_with_executable(project, executable)
    }

    fn start_with_executable(
        project: Project,
        executable: impl Into<OsString>,
    ) -> Result<Self, String> {
        let executable = executable.into();
        let (worker, bootstrap) = spawn_ready_worker(&project, &executable)?;
        Ok(Self {
            project,
            executable,
            worker: Some(worker),
            bootstrap,
            generation: 1,
        })
    }

    #[must_use]
    pub fn bootstrap(&self) -> &Bootstrap {
        &self.bootstrap
    }

    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn request(
        &mut self,
        command: impl Into<String>,
        window_id: impl Into<String>,
        payload: Value,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<ResponseEnvelope, WorkerRequestError> {
        if self.worker.is_none() {
            self.restart().map_err(WorkerRequestError::Crashed)?;
        }

        let result = self
            .worker
            .as_mut()
            .expect("worker was ensured above")
            .request(command, window_id, payload, timeout, cancellation);
        if result.is_err() {
            self.worker.take();
            // Never retry a command automatically: it may have completed a side effect
            // immediately before the worker stopped. A fresh worker is only prepared for
            // the next request.
            let _ = self.restart();
        }
        result
    }

    pub fn restart(&mut self) -> Result<Bootstrap, String> {
        self.worker.take();
        let (worker, bootstrap) = spawn_ready_worker(&self.project, &self.executable)?;
        self.worker = Some(worker);
        self.bootstrap = bootstrap.clone();
        self.generation = self.generation.saturating_add(1);
        Ok(bootstrap)
    }
}

fn spawn_ready_worker(
    project: &Project,
    executable: &OsStr,
) -> Result<(WorkerClient, Bootstrap), String> {
    let mut worker = WorkerClient::spawn(project, executable)?;
    let bootstrap = worker.boot()?;
    bootstrap.validate()?;
    Ok((worker, bootstrap))
}

struct WorkerClient {
    child: Child,
    input: ChildStdin,
    responses: Receiver<Result<ResponseEnvelope, String>>,
    reader: Option<JoinHandle<()>>,
    next_id: u64,
}

impl WorkerClient {
    fn spawn(project: &Project, executable: &OsStr) -> Result<Self, String> {
        let mut child = Command::new(executable)
            .arg("exec")
            .arg(project.application())
            .current_dir(project.root())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| {
                format!(
                    "cannot start PHP worker with {}: {error}",
                    executable.to_string_lossy(),
                )
            })?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| "cannot open PHP worker stdin".to_owned())?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| "cannot open PHP worker stdout".to_owned())?;
        let (sender, responses) = mpsc::channel();
        let reader = std::thread::Builder::new()
            .name("pam-desktop-php-reader".to_owned())
            .spawn(move || {
                let mut output = BufReader::new(output);
                loop {
                    let mut line = String::new();
                    let bytes = match output
                        .by_ref()
                        .take((MAX_MESSAGE_BYTES + 1) as u64)
                        .read_line(&mut line)
                    {
                        Ok(bytes) => bytes,
                        Err(error) => {
                            let _ =
                                sender.send(Err(format!("cannot read from PHP worker: {error}")));
                            break;
                        }
                    };
                    if bytes == 0 {
                        let _ = sender.send(Err("PHP worker exited before replying".to_owned()));
                        break;
                    }
                    if bytes > MAX_MESSAGE_BYTES || !line.ends_with('\n') {
                        let _ = sender.send(Err(
                            "PHP worker response exceeds the limit or is incomplete".to_owned(),
                        ));
                        break;
                    }
                    let response = serde_json::from_str::<ResponseEnvelope>(&line)
                        .map_err(|error| format!("PHP worker returned invalid JSON: {error}"));
                    if sender.send(response).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| format!("cannot start PHP response reader: {error}"))?;

        Ok(Self {
            child,
            input,
            responses,
            reader: Some(reader),
            next_id: 1,
        })
    }

    fn boot(&mut self) -> Result<Bootstrap, String> {
        let response = self
            .request(
                BOOT_COMMAND,
                MAIN_WINDOW_ID,
                Value::Null,
                BOOT_TIMEOUT,
                &CancellationToken::default(),
            )
            .map_err(|error| error.to_string())?;
        if response.status == ResponseStatus::Failure {
            return Err(worker_failure(&response));
        }
        serde_json::from_value(response.payload)
            .map_err(|error| format!("worker returned an invalid bootstrap contract: {error}"))
    }

    fn request(
        &mut self,
        command: impl Into<String>,
        window_id: impl Into<String>,
        payload: Value,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<ResponseEnvelope, WorkerRequestError> {
        let request_id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| WorkerRequestError::Crashed("worker request id overflow".to_owned()))?;
        let request = RequestEnvelope::for_window(request_id, command, window_id, payload);
        let encoded = serde_json::to_vec(&request).map_err(|error| {
            WorkerRequestError::Crashed(format!("cannot serialize worker request: {error}"))
        })?;
        if encoded.len() > MAX_MESSAGE_BYTES {
            return Err(WorkerRequestError::Crashed(
                "worker request exceeds the one-megabyte limit".to_owned(),
            ));
        }

        self.input
            .write_all(&encoded)
            .and_then(|()| self.input.write_all(b"\n"))
            .and_then(|()| self.input.flush())
            .map_err(|error| {
                WorkerRequestError::Crashed(format!("cannot write to PHP worker: {error}"))
            })?;

        let deadline = Instant::now() + timeout;
        loop {
            if cancellation.is_cancelled() {
                return Err(WorkerRequestError::Cancelled);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(WorkerRequestError::TimedOut);
            }
            let wait = CANCELLATION_POLL_INTERVAL.min(deadline.saturating_duration_since(now));
            match self.responses.recv_timeout(wait) {
                Ok(Ok(response)) => {
                    response
                        .validate_for(request_id)
                        .map_err(WorkerRequestError::Crashed)?;
                    return Ok(response);
                }
                Ok(Err(error)) => return Err(WorkerRequestError::Crashed(error)),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(WorkerRequestError::Crashed(
                        "PHP response channel disconnected".to_owned(),
                    ));
                }
            }
        }
    }
}

impl Drop for WorkerClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[must_use]
pub fn worker_failure(response: &ResponseEnvelope) -> String {
    response.error.as_ref().map_or_else(
        || "PHP worker returned an unspecified failure".to_owned(),
        |error| format!("PHP worker error {}: {}", error.code as u16, error.message),
    )
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn recovers_after_timeout_cancellation_and_crash_without_replaying_commands() {
        let fixture = WorkerFixture::create();
        let project = Project::discover(&fixture.root).expect("fixture project should be valid");
        let mut supervisor =
            WorkerSupervisor::start_with_executable(project, fixture.worker.clone())
                .expect("fake worker should boot");

        let timeout = supervisor.request(
            "slow",
            MAIN_WINDOW_ID,
            Value::Null,
            Duration::from_millis(40),
            &CancellationToken::default(),
        );
        assert!(matches!(timeout, Err(WorkerRequestError::TimedOut)));
        assert_eq!(supervisor.generation(), 2);
        assert_success(supervisor.request(
            "ok",
            MAIN_WINDOW_ID,
            Value::Null,
            Duration::from_secs(1),
            &CancellationToken::default(),
        ));

        let cancellation = CancellationToken::default();
        let cancelling_token = cancellation.clone();
        let canceller = thread::spawn(move || {
            thread::sleep(Duration::from_millis(40));
            cancelling_token.cancel();
        });
        let cancelled = supervisor.request(
            "slow",
            MAIN_WINDOW_ID,
            Value::Null,
            Duration::from_secs(1),
            &cancellation,
        );
        canceller.join().expect("canceller should finish");
        assert!(matches!(cancelled, Err(WorkerRequestError::Cancelled)));
        assert_eq!(supervisor.generation(), 3);
        assert_success(supervisor.request(
            "ok",
            MAIN_WINDOW_ID,
            Value::Null,
            Duration::from_secs(1),
            &CancellationToken::default(),
        ));

        let crashed = supervisor.request(
            "crash",
            MAIN_WINDOW_ID,
            Value::Null,
            Duration::from_secs(1),
            &CancellationToken::default(),
        );
        assert!(matches!(crashed, Err(WorkerRequestError::Crashed(_))));
        assert_eq!(supervisor.generation(), 4);
        assert_success(supervisor.request(
            "ok",
            MAIN_WINDOW_ID,
            Value::Null,
            Duration::from_secs(1),
            &CancellationToken::default(),
        ));
    }

    fn assert_success(response: Result<ResponseEnvelope, WorkerRequestError>) {
        let response = response.expect("recovered worker should answer");
        assert_eq!(response.status, ResponseStatus::Success);
        assert_eq!(response.payload, serde_json::json!({"recovered": true}));
    }

    struct WorkerFixture {
        root: PathBuf,
        worker: PathBuf,
    }

    impl WorkerFixture {
        fn create() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be valid")
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "pam-desktop-worker-test-{}-{unique}",
                std::process::id(),
            ));
            fs::create_dir_all(root.join("vendor")).expect("vendor directory should be created");
            fs::create_dir_all(root.join("resources"))
                .expect("resources directory should be created");
            fs::write(root.join("app.php"), "<?php\n")
                .expect("application fixture should be written");
            fs::write(root.join("composer.json"), "{}\n")
                .expect("manifest fixture should be written");
            fs::write(root.join("vendor/autoload.php"), "<?php\n")
                .expect("autoload fixture should be written");
            fs::write(root.join("resources/index.html"), "<!doctype html>\n")
                .expect("entry fixture should be written");
            fs::write(
                root.join("resources/icon.svg"),
                "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 64 64\"></svg>\n",
            )
            .expect("icon fixture should be written");

            let worker = root.join("fake-pam");
            fs::write(
                &worker,
                r#"#!/bin/sh
while IFS= read -r line; do
    id=$(printf '%s\n' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
    case "$line" in
        *'"command":"@pam/boot"'*)
            printf '{"version":4,"id":%s,"kind":2,"status":1,"payload":{"manifest":{"identifier":"com.pushin.test","name":"Test","version":"0.4.0","description":"","publisher":"Pushin","category":1,"icon":"resources/icon.svg","bundleExcludes":[]},"windows":[{"id":"main","entry":"resources/index.html","title":"Test","width":800,"height":600,"minWidth":320,"minHeight":240,"resizable":true,"visible":true,"theme":3}],"commandTimeoutMs":30000,"capabilities":{}},"effects":[],"events":[]}\n' "$id"
            ;;
        *'"command":"slow"'*)
            sleep 1
            printf '{"version":4,"id":%s,"kind":2,"status":1,"payload":null,"effects":[],"events":[]}\n' "$id"
            ;;
        *'"command":"crash"'*)
            exit 9
            ;;
        *)
            printf '{"version":4,"id":%s,"kind":2,"status":1,"payload":{"recovered":true},"effects":[],"events":[]}\n' "$id"
            ;;
    esac
done
"#,
            )
            .expect("worker fixture should be written");
            fs::set_permissions(&worker, fs::Permissions::from_mode(0o755))
                .expect("worker fixture should be executable");

            Self { root, worker }
        }
    }

    impl Drop for WorkerFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
