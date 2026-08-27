use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[cfg(test)]
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use pam_desktop_protocol::{
    MAX_COMMAND_TIMEOUT_MS, MIN_COMMAND_TIMEOUT_MS, ProcessArgumentPolicy, ProcessCommandConfig,
};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};

use crate::native::NativeError;

const MAX_IO: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessRequest {
    pub window_id: String,
    #[serde(default = "default_process_operation")]
    pub operation: u8,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub arguments: Vec<String>,
    #[serde(default)]
    pub stdin: String,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub session_id: u64,
    #[serde(default = "default_columns")]
    pub columns: u16,
    #[serde(default = "default_rows")]
    pub rows: u16,
    #[serde(default)]
    pub data: String,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum ProcessOperation {
    Run = 1,
    PtyOpen = 2,
    PtyWrite = 3,
    PtyRead = 4,
    PtyResize = 5,
    PtyClose = 6,
}

impl TryFrom<u8> for ProcessOperation {
    type Error = NativeError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Run),
            2 => Ok(Self::PtyOpen),
            3 => Ok(Self::PtyWrite),
            4 => Ok(Self::PtyRead),
            5 => Ok(Self::PtyResize),
            6 => Ok(Self::PtyClose),
            _ => Err(NativeError::invalid("Unknown process operation.")),
        }
    }
}

struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    output: mpsc::Receiver<Vec<u8>>,
}

pub struct ProcessServices {
    commands: HashMap<String, AllowedCommand>,
    project_root: PathBuf,
    sessions: Mutex<HashMap<u64, PtySession>>,
    next_session_id: AtomicU64,
}

impl Drop for ProcessServices {
    fn drop(&mut self) {
        let sessions = self
            .sessions
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for session in sessions.values_mut() {
            let _ = session.child.kill();
            let _ = session.child.wait();
        }
        sessions.clear();
    }
}

impl ProcessServices {
    pub fn terminal_session_count(&self) -> usize {
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

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
            sessions: Mutex::new(HashMap::new()),
            next_session_id: AtomicU64::new(1),
        })
    }

    pub fn dispatch(&self, request: &ProcessRequest) -> Result<serde_json::Value, NativeError> {
        match ProcessOperation::try_from(request.operation)? {
            ProcessOperation::Run => serde_json::to_value(self.execute(request)?)
                .map_err(|error| NativeError::native("Cannot encode process result", error)),
            ProcessOperation::PtyOpen => self.open_pty(request),
            ProcessOperation::PtyWrite => self.write_pty(request),
            ProcessOperation::PtyRead => self.read_pty(request),
            ProcessOperation::PtyResize => self.resize_pty(request),
            ProcessOperation::PtyClose => self.close_pty(request),
        }
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

    fn open_pty(&self, request: &ProcessRequest) -> Result<serde_json::Value, NativeError> {
        validate_terminal_size(request.columns, request.rows)?;
        let allowed = self.allowed_command(request)?;
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: request.rows,
                cols: request.columns,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| NativeError::native("Cannot open terminal PTY", error))?;
        let mut command = CommandBuilder::new(&allowed.executable);
        command.args(&allowed.arguments);
        command.args(&request.arguments);
        command.cwd(&self.project_root);
        command.env_clear();
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| NativeError::native("Cannot start terminal command", error))?;
        drop(pair.slave);
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| NativeError::native("Cannot open terminal output", error))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| NativeError::native("Cannot open terminal input", error))?;
        let (sender, output) = mpsc::sync_channel(64);
        std::thread::Builder::new()
            .name("pam-terminal-output".to_owned())
            .spawn(move || {
                let mut buffer = vec![0_u8; 16 * 1024];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(read) if sender.send(buffer[..read].to_vec()).is_err() => break,
                        Ok(_) => {}
                    }
                }
            })
            .map_err(|error| NativeError::native("Cannot start terminal output reader", error))?;
        let id = self.next_session_id.fetch_add(1, Ordering::Relaxed);
        self.sessions
            .lock()
            .map_err(|_| NativeError::native("Terminal session lock is poisoned", id))?
            .insert(
                id,
                PtySession {
                    master: pair.master,
                    writer,
                    child,
                    output,
                },
            );
        Ok(serde_json::json!({"sessionId": id, "columns": request.columns, "rows": request.rows}))
    }

    fn write_pty(&self, request: &ProcessRequest) -> Result<serde_json::Value, NativeError> {
        if request.data.len() > MAX_IO {
            return Err(NativeError::too_large(
                "Terminal write cannot exceed 1 MiB.",
            ));
        }
        let mut sessions = self.sessions.lock().map_err(|_| {
            NativeError::native("Terminal session lock is poisoned", request.session_id)
        })?;
        let session = sessions
            .get_mut(&request.session_id)
            .ok_or_else(|| NativeError::not_found("Terminal session does not exist."))?;
        session
            .writer
            .write_all(request.data.as_bytes())
            .and_then(|()| session.writer.flush())
            .map_err(|error| NativeError::native("Cannot write terminal input", error))?;
        Ok(serde_json::json!({"writtenBytes": request.data.len()}))
    }

    fn read_pty(&self, request: &ProcessRequest) -> Result<serde_json::Value, NativeError> {
        let mut sessions = self.sessions.lock().map_err(|_| {
            NativeError::native("Terminal session lock is poisoned", request.session_id)
        })?;
        let session = sessions
            .get_mut(&request.session_id)
            .ok_or_else(|| NativeError::not_found("Terminal session does not exist."))?;
        let mut bytes = Vec::new();
        let mut output_closed = false;
        while bytes.len() < 256 * 1024 {
            match session.output.try_recv() {
                Ok(chunk) => bytes.extend_from_slice(&chunk),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    output_closed = true;
                    break;
                }
            }
        }
        if bytes.is_empty() && !output_closed {
            match session.output.recv_timeout(Duration::from_millis(20)) {
                Ok(chunk) => {
                    bytes.extend_from_slice(&chunk);
                    while bytes.len() < 256 * 1024 {
                        match session.output.try_recv() {
                            Ok(chunk) => bytes.extend_from_slice(&chunk),
                            Err(mpsc::TryRecvError::Empty) => break,
                            Err(mpsc::TryRecvError::Disconnected) => {
                                output_closed = true;
                                break;
                            }
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => output_closed = true,
            }
        }
        let exit = session
            .child
            .try_wait()
            .map_err(|error| NativeError::native("Cannot inspect terminal command", error))?;
        Ok(serde_json::json!({
            "data": base64::engine::general_purpose::STANDARD.encode(bytes),
            "encoding": 1,
            "running": exit.is_none() || !output_closed,
            "exitCode": exit.as_ref().map(portable_pty::ExitStatus::exit_code),
            "signal": exit.as_ref().and_then(portable_pty::ExitStatus::signal),
        }))
    }

    fn resize_pty(&self, request: &ProcessRequest) -> Result<serde_json::Value, NativeError> {
        validate_terminal_size(request.columns, request.rows)?;
        let sessions = self.sessions.lock().map_err(|_| {
            NativeError::native("Terminal session lock is poisoned", request.session_id)
        })?;
        let session = sessions
            .get(&request.session_id)
            .ok_or_else(|| NativeError::not_found("Terminal session does not exist."))?;
        session
            .master
            .resize(PtySize {
                rows: request.rows,
                cols: request.columns,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| NativeError::native("Cannot resize terminal PTY", error))?;
        Ok(serde_json::json!({"columns": request.columns, "rows": request.rows}))
    }

    fn close_pty(&self, request: &ProcessRequest) -> Result<serde_json::Value, NativeError> {
        let mut session = self
            .sessions
            .lock()
            .map_err(|_| {
                NativeError::native("Terminal session lock is poisoned", request.session_id)
            })?
            .remove(&request.session_id)
            .ok_or_else(|| NativeError::not_found("Terminal session does not exist."))?;
        session
            .child
            .kill()
            .map_err(|error| NativeError::native("Cannot stop terminal command", error))?;
        Ok(serde_json::json!({"closed": true}))
    }

    fn allowed_command<'a>(
        &'a self,
        request: &ProcessRequest,
    ) -> Result<&'a AllowedCommand, NativeError> {
        let allowed = self.commands.get(&request.command).ok_or_else(|| {
            NativeError::permission("The requested process command is not authorized.")
        })?;
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
        Ok(allowed)
    }
}

fn validate_terminal_size(columns: u16, rows: u16) -> Result<(), NativeError> {
    if !(1..=1_000).contains(&columns) || !(1..=1_000).contains(&rows) {
        return Err(NativeError::invalid(
            "Terminal size must be between 1 and 1000 cells.",
        ));
    }
    Ok(())
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
const fn default_process_operation() -> u8 {
    1
}
const fn default_columns() -> u16 {
    80
}
const fn default_rows() -> u16 {
    24
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn runs_only_declared_executables_and_argument_policy() {
        let root = std::env::temp_dir().join(format!(
            "pam-process-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be valid")
                .as_nanos(),
        ));
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
                operation: 1,
                command: "echo".to_owned(),
                arguments: vec!["safe".to_owned()],
                stdin: String::new(),
                timeout_ms: 1_000,
                session_id: 0,
                columns: 80,
                rows: 24,
                data: String::new(),
            })
            .expect("declared process should run");
        assert_eq!(result.stdout, "safe");
        let opened = services
            .dispatch(&ProcessRequest {
                window_id: "main".to_owned(),
                operation: 2,
                command: "echo".to_owned(),
                arguments: vec!["terminal".to_owned()],
                stdin: String::new(),
                timeout_ms: 1_000,
                session_id: 0,
                columns: 100,
                rows: 30,
                data: String::new(),
            })
            .expect("authorized PTY should open");
        let session_id = opened["sessionId"].as_u64().expect("PTY should have an id");
        let mut terminal_output = String::new();
        for _ in 0..50 {
            let chunk = services
                .dispatch(&ProcessRequest {
                    window_id: "main".to_owned(),
                    operation: 4,
                    command: String::new(),
                    arguments: Vec::new(),
                    stdin: String::new(),
                    timeout_ms: 1_000,
                    session_id,
                    columns: 80,
                    rows: 24,
                    data: String::new(),
                })
                .expect("PTY output should be readable");
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(chunk["data"].as_str().expect("PTY data should be base64"))
                .expect("PTY base64 should decode");
            terminal_output.push_str(&String::from_utf8_lossy(&bytes));
            if chunk["running"] == false {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(terminal_output.contains("terminal"));
        assert!(
            services
                .execute(&ProcessRequest {
                    operation: 1,
                    command: "missing".to_owned(),
                    arguments: Vec::new(),
                    stdin: String::new(),
                    timeout_ms: 1_000,
                    window_id: "main".to_owned(),
                    session_id: 0,
                    columns: 80,
                    rows: 24,
                    data: String::new(),
                })
                .is_err()
        );
        fs::remove_dir_all(root).expect("fixture should be removable");
    }
}
