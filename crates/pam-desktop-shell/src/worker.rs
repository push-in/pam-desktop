use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use pam_desktop_protocol::{
    BOOT_COMMAND, Bootstrap, MAX_MESSAGE_BYTES, RequestEnvelope, ResponseEnvelope, ResponseStatus,
};
use serde_json::Value;

use crate::project::Project;

pub struct WorkerClient {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next_id: u64,
}

impl WorkerClient {
    pub fn spawn(project: &Project) -> Result<Self, String> {
        let executable = std::env::var_os("PAM_BINARY").unwrap_or_else(|| "pam".into());
        let mut child = Command::new(&executable)
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
                    executable.to_string_lossy()
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

        Ok(Self {
            child,
            input,
            output: BufReader::new(output),
            next_id: 1,
        })
    }

    pub fn boot(&mut self) -> Result<Bootstrap, String> {
        let response = self.request(BOOT_COMMAND, Value::Null)?;
        if response.status == ResponseStatus::Failure {
            return Err(worker_failure(&response));
        }
        serde_json::from_value(response.payload)
            .map_err(|error| format!("worker returned an invalid bootstrap contract: {error}"))
    }

    pub fn request(
        &mut self,
        command: impl Into<String>,
        payload: Value,
    ) -> Result<ResponseEnvelope, String> {
        let request_id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| "worker request id overflow".to_owned())?;
        let request = RequestEnvelope::new(request_id, command, payload);
        let encoded = serde_json::to_vec(&request)
            .map_err(|error| format!("cannot serialize worker request: {error}"))?;
        if encoded.len() > MAX_MESSAGE_BYTES {
            return Err("worker request exceeds the one-megabyte limit".to_owned());
        }

        self.input
            .write_all(&encoded)
            .and_then(|()| self.input.write_all(b"\n"))
            .and_then(|()| self.input.flush())
            .map_err(|error| format!("cannot write to PHP worker: {error}"))?;

        let mut line = String::new();
        let bytes = self
            .output
            .by_ref()
            .take((MAX_MESSAGE_BYTES + 1) as u64)
            .read_line(&mut line)
            .map_err(|error| format!("cannot read from PHP worker: {error}"))?;
        if bytes == 0 {
            return Err("PHP worker exited before replying".to_owned());
        }
        if bytes > MAX_MESSAGE_BYTES || !line.ends_with('\n') {
            return Err("PHP worker response exceeds the limit or is incomplete".to_owned());
        }

        let response: ResponseEnvelope = serde_json::from_str(&line)
            .map_err(|error| format!("PHP worker returned invalid JSON: {error}"))?;
        response.validate_for(request_id)?;
        Ok(response)
    }
}

impl Drop for WorkerClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[must_use]
pub fn worker_failure(response: &ResponseEnvelope) -> String {
    response.error.as_ref().map_or_else(
        || "PHP worker returned an unspecified failure".to_owned(),
        |error| format!("PHP worker error {}: {}", error.code as u16, error.message),
    )
}
