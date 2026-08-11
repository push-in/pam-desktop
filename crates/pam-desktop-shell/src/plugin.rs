use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pam_desktop_protocol::{
    ClientEvent, ErrorCode, MAX_MESSAGE_BYTES, PLUGIN_BOOT_COMMAND, PLUGIN_PROTOCOL_VERSION,
    PluginMetadata, PluginRequestEnvelope, PluginResponseEnvelope, ResponseStatus,
    RustPluginConfig,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::project::Project;
use crate::worker::CancellationToken;

const PLUGIN_BOOT_TIMEOUT: Duration = Duration::from_secs(10);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Debug)]
pub struct PluginError {
    pub code: ErrorCode,
    pub message: String,
}

impl PluginError {
    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::PluginUnavailable,
            message: message.into(),
        }
    }

    fn failed(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::PluginFailed,
            message: message.into(),
        }
    }
}

#[derive(Debug)]
pub struct PluginInvocation {
    pub payload: Value,
    pub events: Vec<ClientEvent>,
}

pub struct PluginSupervisor {
    plugins: HashMap<String, Mutex<PluginSlot>>,
}

impl PluginSupervisor {
    pub fn prepare(project: &Project, configs: &[RustPluginConfig]) -> Result<Self, String> {
        let mut plugins = HashMap::with_capacity(configs.len());
        for config in configs {
            let executable = project.resolve_plugin_executable(&config.executable)?;
            verify_integrity(&executable, config)?;
            let slot = PluginSlot::start(project.root(), executable, config.clone())?;
            plugins.insert(config.id.clone(), Mutex::new(slot));
        }
        Ok(Self { plugins })
    }

    pub fn invoke(
        &self,
        plugin_id: &str,
        command: &str,
        payload: Value,
        timeout: Option<Duration>,
        cancellation: &CancellationToken,
    ) -> Result<PluginInvocation, PluginError> {
        let slot = self.plugins.get(plugin_id).ok_or_else(|| {
            PluginError::unavailable(format!("Rust plugin {plugin_id:?} is not registered."))
        })?;
        let mut slot = slot
            .lock()
            .map_err(|_| PluginError::unavailable("Rust plugin supervisor lock is poisoned."))?;
        slot.invoke(command, payload, timeout, cancellation)
    }
}

struct PluginSlot {
    project_root: PathBuf,
    executable: PathBuf,
    config: RustPluginConfig,
    metadata: PluginMetadata,
    client: Option<PluginClient>,
}

impl PluginSlot {
    fn start(
        project_root: &Path,
        executable: PathBuf,
        config: RustPluginConfig,
    ) -> Result<Self, String> {
        let mut client = PluginClient::spawn(project_root, &executable, &config)?;
        let metadata = client.boot()?;
        metadata.validate()?;
        if metadata.identifier != config.id {
            return Err(format!(
                "Rust plugin at {} identifies as {:?}, expected {:?}",
                executable.display(),
                metadata.identifier,
                config.id
            ));
        }
        Ok(Self {
            project_root: project_root.to_path_buf(),
            executable,
            config,
            metadata,
            client: Some(client),
        })
    }

    fn invoke(
        &mut self,
        command: &str,
        payload: Value,
        timeout: Option<Duration>,
        cancellation: &CancellationToken,
    ) -> Result<PluginInvocation, PluginError> {
        if !self
            .metadata
            .commands
            .iter()
            .any(|export| export == command)
        {
            return Err(PluginError {
                code: ErrorCode::UnknownCommand,
                message: format!(
                    "Rust plugin {:?} does not export command {command:?}.",
                    self.config.id
                ),
            });
        }
        if self.client.is_none() {
            self.restart().map_err(PluginError::unavailable)?;
        }
        let deadline = timeout.unwrap_or_else(|| Duration::from_millis(self.config.timeout_ms));
        let result = self
            .client
            .as_mut()
            .expect("plugin client was ensured above")
            .request(command, payload, deadline, cancellation);
        let response = match result {
            Ok(response) => response,
            Err(error) => {
                self.client.take();
                let _ = self.restart();
                return Err(error);
            }
        };
        if response.status == ResponseStatus::Failure {
            return Err(response.error.map_or_else(
                || PluginError::failed("Rust plugin returned an unspecified failure."),
                |error| PluginError {
                    code: error.code,
                    message: error.message,
                },
            ));
        }
        Ok(PluginInvocation {
            payload: response.payload,
            events: response.events,
        })
    }

    fn restart(&mut self) -> Result<(), String> {
        self.client.take();
        verify_integrity(&self.executable, &self.config)?;
        let mut client = PluginClient::spawn(&self.project_root, &self.executable, &self.config)?;
        let metadata = client.boot()?;
        metadata.validate()?;
        if metadata != self.metadata {
            return Err(format!(
                "Rust plugin {:?} changed metadata while recovering",
                self.config.id
            ));
        }
        self.client = Some(client);
        Ok(())
    }
}

struct PluginClient {
    child: Child,
    input: ChildStdin,
    responses: Receiver<Result<PluginResponseEnvelope, String>>,
    reader: Option<JoinHandle<()>>,
    next_id: u64,
}

impl PluginClient {
    fn spawn(
        project_root: &Path,
        executable: &Path,
        config: &RustPluginConfig,
    ) -> Result<Self, String> {
        let mut child = Command::new(executable)
            .args(&config.arguments)
            .current_dir(project_root)
            .env_clear()
            .env("PAM_DESKTOP_PLUGIN_ID", &config.id)
            .env(
                "PAM_DESKTOP_PLUGIN_PROTOCOL",
                PLUGIN_PROTOCOL_VERSION.to_string(),
            )
            .env("PAM_DESKTOP_PROJECT_ROOT", project_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| {
                format!(
                    "cannot start Rust plugin {:?} at {}: {error}",
                    config.id,
                    executable.display()
                )
            })?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| format!("cannot open stdin for Rust plugin {:?}", config.id))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| format!("cannot open stdout for Rust plugin {:?}", config.id))?;
        let (sender, responses) = mpsc::channel();
        let reader_name = format!("pam-plugin-{}-reader", config.id);
        let reader = std::thread::Builder::new()
            .name(reader_name)
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
                                sender.send(Err(format!("cannot read from Rust plugin: {error}")));
                            break;
                        }
                    };
                    if bytes == 0 {
                        let _ = sender.send(Err("Rust plugin exited before replying".to_owned()));
                        break;
                    }
                    if bytes > MAX_MESSAGE_BYTES || !line.ends_with('\n') {
                        let _ = sender.send(Err(
                            "Rust plugin response exceeds the limit or is incomplete".to_owned(),
                        ));
                        break;
                    }
                    let response = serde_json::from_str::<PluginResponseEnvelope>(&line)
                        .map_err(|error| format!("Rust plugin returned invalid JSON: {error}"));
                    if sender.send(response).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| format!("cannot start Rust plugin response reader: {error}"))?;
        Ok(Self {
            child,
            input,
            responses,
            reader: Some(reader),
            next_id: 1,
        })
    }

    fn boot(&mut self) -> Result<PluginMetadata, String> {
        let response = self
            .request(
                PLUGIN_BOOT_COMMAND,
                Value::Null,
                PLUGIN_BOOT_TIMEOUT,
                &CancellationToken::default(),
            )
            .map_err(|error| error.message)?;
        if response.status == ResponseStatus::Failure {
            return Err(response.error.map_or_else(
                || "Rust plugin rejected its boot request".to_owned(),
                |error| format!("Rust plugin boot failed: {}", error.message),
            ));
        }
        serde_json::from_value(response.payload)
            .map_err(|error| format!("Rust plugin returned invalid metadata: {error}"))
    }

    fn request(
        &mut self,
        command: impl Into<String>,
        payload: Value,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<PluginResponseEnvelope, PluginError> {
        let request_id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| PluginError::failed("Rust plugin request id overflow."))?;
        let request = PluginRequestEnvelope::new(request_id, command, payload);
        let encoded = serde_json::to_vec(&request).map_err(|error| {
            PluginError::failed(format!("Cannot encode plugin request: {error}"))
        })?;
        if encoded.len() > MAX_MESSAGE_BYTES {
            return Err(PluginError {
                code: ErrorCode::ResourceTooLarge,
                message: "Rust plugin request exceeds the one-megabyte limit.".to_owned(),
            });
        }
        self.input
            .write_all(&encoded)
            .and_then(|()| self.input.write_all(b"\n"))
            .and_then(|()| self.input.flush())
            .map_err(|error| {
                PluginError::unavailable(format!("Cannot write to Rust plugin: {error}"))
            })?;

        let deadline = Instant::now() + timeout;
        loop {
            if cancellation.is_cancelled() {
                return Err(PluginError {
                    code: ErrorCode::RequestCancelled,
                    message: "Rust plugin invocation was cancelled.".to_owned(),
                });
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(PluginError {
                    code: ErrorCode::RequestTimedOut,
                    message: "Rust plugin invocation exceeded its deadline.".to_owned(),
                });
            }
            let wait = CANCELLATION_POLL_INTERVAL.min(deadline.saturating_duration_since(now));
            match self.responses.recv_timeout(wait) {
                Ok(Ok(response)) => {
                    response
                        .validate_for(request_id)
                        .map_err(PluginError::failed)?;
                    return Ok(response);
                }
                Ok(Err(error)) => return Err(PluginError::unavailable(error)),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(PluginError::unavailable(
                        "Rust plugin response channel disconnected.",
                    ));
                }
            }
        }
    }
}

impl Drop for PluginClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn verify_integrity(executable: &Path, config: &RustPluginConfig) -> Result<(), String> {
    let Some(expected) = &config.sha256 else {
        return Ok(());
    };
    let bytes = std::fs::read(executable).map_err(|error| {
        format!(
            "cannot read Rust plugin {:?} for integrity verification: {error}",
            config.id
        )
    })?;
    let actual = format!("{:x}", Sha256::digest(bytes));
    if &actual != expected {
        return Err(format!(
            "Rust plugin {:?} failed SHA-256 integrity verification",
            config.id
        ));
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn supervises_a_process_plugin_and_recovers_after_a_crash() {
        let fixture = Fixture::create();
        let project = Project::discover(&fixture.root).expect("fixture project should be valid");
        let config = RustPluginConfig {
            id: "fixture".to_owned(),
            executable: "plugins/fixture".to_owned(),
            arguments: Vec::new(),
            timeout_ms: 1_000,
            sha256: None,
        };
        let supervisor =
            PluginSupervisor::prepare(&project, &[config]).expect("plugin should start");
        let invocation = supervisor
            .invoke(
                "fixture",
                "echo",
                serde_json::json!({"safe": true}),
                None,
                &CancellationToken::default(),
            )
            .expect("plugin command should succeed");
        assert_eq!(invocation.payload, serde_json::json!({"safe": true}));

        let crashed = supervisor.invoke(
            "fixture",
            "crash",
            Value::Null,
            None,
            &CancellationToken::default(),
        );
        assert!(matches!(
            crashed,
            Err(PluginError {
                code: ErrorCode::PluginUnavailable,
                ..
            })
        ));
        assert!(
            supervisor
                .invoke(
                    "fixture",
                    "echo",
                    serde_json::json!({"recovered": true}),
                    None,
                    &CancellationToken::default(),
                )
                .is_ok()
        );
    }

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn create() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be valid")
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "pam-desktop-plugin-test-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(root.join("vendor")).expect("vendor should be created");
            fs::create_dir_all(root.join("resources")).expect("resources should be created");
            fs::create_dir_all(root.join("plugins")).expect("plugins should be created");
            fs::write(root.join("app.php"), "<?php\n").expect("app should be written");
            fs::write(root.join("composer.json"), "{}\n").expect("composer should be written");
            fs::write(root.join("vendor/autoload.php"), "<?php\n")
                .expect("autoload should be written");
            fs::write(root.join("resources/index.html"), "<!doctype html>\n")
                .expect("entry should be written");
            fs::write(
                root.join("resources/icon.svg"),
                "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 64 64\"></svg>\n",
            )
            .expect("icon should be written");
            let executable = root.join("plugins/fixture");
            fs::write(
                &executable,
                r#"#!/bin/sh
while IFS= read -r line; do
    id=$(printf '%s\n' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
    case "$line" in
        *'"command":"@pam/plugin/boot"'*)
            printf '{"version":1,"id":%s,"kind":2,"status":1,"payload":{"identifier":"fixture","name":"Fixture","version":"1.0.0","commands":["echo","crash"]},"events":[]}\n' "$id"
            ;;
        *'"command":"crash"'*)
            exit 9
            ;;
        *'"recovered":true'*)
            printf '{"version":1,"id":%s,"kind":2,"status":1,"payload":{"recovered":true},"events":[]}\n' "$id"
            ;;
        *)
            printf '{"version":1,"id":%s,"kind":2,"status":1,"payload":{"safe":true},"events":[]}\n' "$id"
            ;;
    esac
done
"#,
            )
            .expect("plugin should be written");
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
                .expect("plugin should be executable");
            Self { root }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
