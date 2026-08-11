use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pam_desktop_protocol::{
    MAX_COMMAND_TIMEOUT_MS, MIN_COMMAND_TIMEOUT_MS, ProcessArgumentPolicy, ProcessCommandConfig,
};
use serde::{Deserialize, Serialize};

use crate::native::NativeError;

const MAX_IO: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessRequest {
    pub window_id: String,
    pub command: String,
    #[serde(default)]
    pub arguments: Vec<String>,
    #[serde(default)]
    pub stdin: String,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessResponse {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

struct AllowedCommand {
    executable: PathBuf,
    arguments: Vec<String>,
    policy: ProcessArgumentPolicy,
}

pub struct ProcessServices {
    commands: HashMap<String, AllowedCommand>,
    project_root: PathBuf,
}

impl ProcessServices {
    pub fn prepare(project_root: &Path, configs: &[ProcessCommandConfig]) -> Result<Self, String> {
        let project_root = project_root
            .canonicalize()
            .map_err(|error| format!("cannot resolve process project root: {error}"))?;
        let mut commands = HashMap::with_capacity(configs.len());
        for config in configs {
            let candidate = project_root.join(&config.executable);
            let metadata = std::fs::symlink_metadata(&candidate).map_err(|error| {
                format!("cannot inspect process command {:?}: {error}", config.name)
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(format!(
                    "process command {:?} must be a regular bundled file",
                    config.name
                ));
            }
            let executable = candidate.canonicalize().map_err(|error| {
                format!("cannot resolve process command {:?}: {error}", config.name)
            })?;
            if !executable.starts_with(&project_root) {
                return Err(format!(
                    "process command {:?} escapes the project",
                    config.name
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o111 == 0 {
                    return Err(format!(
                        "process command {:?} is not executable",
                        config.name
                    ));
                }
            }
            commands.insert(
                config.name.clone(),
                AllowedCommand {
                    executable,
                    arguments: config.arguments.clone(),
                    policy: config.argument_policy,
                },
            );
        }
        Ok(Self {
            commands,
            project_root,
        })
    }

    pub fn execute(&self, request: &ProcessRequest) -> Result<ProcessResponse, NativeError> {
        let allowed = self.commands.get(&request.command).ok_or_else(|| {
            NativeError::permission("The requested process command is not authorized.")
        })?;
        if !(MIN_COMMAND_TIMEOUT_MS..=MAX_COMMAND_TIMEOUT_MS).contains(&request.timeout_ms) {
            return Err(NativeError::invalid(
                "Process timeout is outside the supported range.",
            ));
        }
        if request.arguments.len() > 32
            || request
                .arguments
                .iter()
                .any(|value| value.len() > 1_024 || value.contains('\0'))
        {
            return Err(NativeError::invalid(
                "Dynamic process arguments are invalid.",
            ));
        }
        if allowed.policy == ProcessArgumentPolicy::Fixed && !request.arguments.is_empty() {
            return Err(NativeError::permission(
                "This process command does not allow dynamic arguments.",
            ));
        }
        if request.stdin.len() > MAX_IO {
            return Err(NativeError::too_large("Process stdin cannot exceed 1 MiB."));
        }
        let mut child = Command::new(&allowed.executable)
            .args(&allowed.arguments)
            .args(&request.arguments)
            .current_dir(&self.project_root)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| NativeError::native("Cannot start the authorized process", error))?;
        let stdout = drain(child.stdout.take(), "stdout")?;
        let stderr = drain(child.stderr.take(), "stderr")?;
        let stdin = request.stdin.clone();
        let mut input = child.stdin.take().ok_or_else(|| {
            NativeError::native(
                "Cannot open authorized process stdin",
                request.command.as_str(),
            )
        })?;
        let mut input = Some(
            std::thread::Builder::new()
                .name("pam-process-stdin".to_owned())
                .spawn(move || input.write_all(stdin.as_bytes()))
                .map_err(|error| NativeError::native("Cannot start process input writer", error))?,
        );
        let deadline = Instant::now() + Duration::from_millis(request.timeout_ms);
        let status = loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|error| NativeError::native("Cannot inspect process status", error))?
            {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                if let Some(input) = input.take() {
                    let _ = input.join();
                }
                let _ = stdout.join();
                let _ = stderr.join();
                return Err(NativeError::native(
                    "Authorized process timed out",
                    request.timeout_ms,
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        input
            .take()
            .expect("process input writer is available after successful wait")
            .join()
            .map_err(|_| {
                NativeError::native("Process input writer crashed", request.command.as_str())
            })?
            .map_err(|error| NativeError::native("Cannot write process stdin", error))?;
        Ok(ProcessResponse {
            success: status.success(),
            exit_code: status.code(),
            stdout: collect(stdout, "stdout")?,
            stderr: collect(stderr, "stderr")?,
        })
    }
}

type Drain = JoinHandle<Result<(Vec<u8>, bool), std::io::Error>>;
fn drain<R: Read + Send + 'static>(
    reader: Option<R>,
    label: &'static str,
) -> Result<Drain, NativeError> {
    let mut reader =
        reader.ok_or_else(|| NativeError::native("Cannot capture process output", label))?;
    std::thread::Builder::new()
        .name(format!("pam-process-{label}"))
        .spawn(move || {
            let mut kept = Vec::new();
            let mut overflow = false;
            let mut chunk = [0_u8; 16 * 1024];
            loop {
                let read = reader.read(&mut chunk)?;
                if read == 0 {
                    break;
                }
                let remaining = MAX_IO.saturating_sub(kept.len());
                kept.extend_from_slice(&chunk[..read.min(remaining)]);
                overflow |= read > remaining;
            }
            Ok((kept, overflow))
        })
        .map_err(|error| NativeError::native("Cannot start process output reader", error))
}
fn collect(thread: Drain, label: &str) -> Result<String, NativeError> {
    let (bytes, overflow) = thread
        .join()
        .map_err(|_| NativeError::native("Process output reader crashed", label))?
        .map_err(|error| NativeError::native("Cannot read process output", error))?;
    if overflow {
        return Err(NativeError::too_large(format!(
            "Process {label} exceeded 1 MiB."
        )));
    }
    String::from_utf8(bytes)
        .map_err(|_| NativeError::invalid(format!("Process {label} is not valid UTF-8.")))
}
const fn default_timeout() -> u64 {
    30_000
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn runs_only_declared_executables_and_argument_policy() {
        let root = std::env::temp_dir().join(format!("pam-process-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("bin")).expect("fixture directory should exist");
        let executable = root.join("bin/echo");
        fs::write(&executable, "#!/bin/sh\nprintf '%s' \"$1\"\n")
            .expect("fixture executable should be written");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
            .expect("fixture executable should be executable");
        let services = ProcessServices::prepare(
            &root,
            &[ProcessCommandConfig {
                name: "echo".to_owned(),
                executable: "bin/echo".to_owned(),
                arguments: Vec::new(),
                argument_policy: ProcessArgumentPolicy::Append,
            }],
        )
        .expect("process services should prepare");
        let result = services
            .execute(&ProcessRequest {
                window_id: "main".to_owned(),
                command: "echo".to_owned(),
                arguments: vec!["safe".to_owned()],
                stdin: String::new(),
                timeout_ms: 1_000,
            })
            .expect("declared process should run");
        assert_eq!(result.stdout, "safe");
        assert!(
            services
                .execute(&ProcessRequest {
                    command: "missing".to_owned(),
                    arguments: Vec::new(),
                    stdin: String::new(),
                    timeout_ms: 1_000,
                    window_id: "main".to_owned()
                })
                .is_err()
        );
        fs::remove_dir_all(root).expect("fixture should be removable");
    }
}
