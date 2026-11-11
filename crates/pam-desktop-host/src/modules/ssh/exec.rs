//! `exec`: runs one command and answers with its exit code and bounded output.

use std::sync::Arc;
use std::time::Duration;

use pam_desktop_ui::modules::Reply;
use pam_desktop_ui::wiremap::{WireMap, WireValue};
use russh::ChannelMsg;

use super::session::Connection;
use super::{MAX_BYTES_VALUE, MAX_EXECS, Permit, input};

const MAX_COMMAND: usize = 64 * 1024;

/// Output of a command, capped at `max` bytes for stdout and stderr together.
#[derive(Debug, Default)]
pub struct Output {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub truncated: bool,
    max: usize,
}

impl Output {
    pub fn new(max: usize) -> Self {
        Self {
            max,
            ..Self::default()
        }
    }

    /// Keeps what fits; the rest is read and dropped so the command can end.
    pub fn push(&mut self, stderr: bool, bytes: &[u8]) {
        let room = self
            .max
            .saturating_sub(self.stdout.len() + self.stderr.len());
        if bytes.len() > room {
            self.truncated = true;
        }
        let kept = &bytes[..bytes.len().min(room)];
        if stderr {
            self.stderr.extend_from_slice(kept);
        } else {
            self.stdout.extend_from_slice(kept);
        }
    }
}

pub fn run(connection: &Arc<Connection>, data: &WireMap, reply: Reply) {
    let parsed = (|| {
        let command = input::required(data, "command", MAX_COMMAND)?.to_owned();
        let timeout = input::integer(data, "timeoutMs", 30_000, 100, 600_000)?;
        let max = input::integer(
            data,
            "maxBytes",
            MAX_BYTES_VALUE as i64,
            1,
            MAX_BYTES_VALUE as i64,
        )?;
        Ok::<_, String>((command, timeout, max))
    })();
    let (command, timeout, max) = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            reply.failure(error);
            return;
        }
    };
    let Some(permit) = Permit::acquire(&connection.execs, MAX_EXECS) else {
        reply.failure("ssh: too many commands running");
        return;
    };
    let connection = Arc::clone(connection);
    let runtime = connection.runtime.clone();
    runtime.spawn(async move {
        let _permit = permit;
        let timeout = Duration::from_millis(u64::try_from(timeout).unwrap_or(30_000));
        let max = usize::try_from(max).unwrap_or(MAX_BYTES_VALUE);
        let result = tokio::select! {
            result = tokio::time::timeout(timeout, execute(&connection, command, max)) => {
                result.unwrap_or_else(|_| Err("exec-timeout".into()))
            }
            () = connection.stopping() => Err("ssh: session closed".into()),
        };
        match result {
            Ok((code, output)) => {
                let mut map = WireMap::new();
                map.insert("exitCode", code);
                map.insert("stdout", WireValue::Bytes(output.stdout));
                map.insert("stderr", WireValue::Bytes(output.stderr));
                map.insert("truncated", output.truncated);
                reply.map(&map);
            }
            Err(error) => reply.failure(error),
        }
    });
}

async fn execute(
    connection: &Connection,
    command: String,
    max: usize,
) -> Result<(i64, Output), String> {
    let mut channel = connection.open_channel().await?;
    channel
        .exec(true, command)
        .await
        .map_err(|_| "ssh: exec failed")?;
    let mut output = Output::new(max);
    let mut code = None;
    while let Some(message) = channel.wait().await {
        match message {
            ChannelMsg::Data { data } => output.push(false, &data),
            ChannelMsg::ExtendedData { data, ext } => output.push(ext == 1, &data),
            ChannelMsg::ExitStatus { exit_status } => code = Some(i64::from(exit_status)),
            ChannelMsg::ExitSignal { .. } => code = code.or(Some(-1)),
            ChannelMsg::Failure => return Err("ssh: exec refused".into()),
            ChannelMsg::Close => break,
            _ => {}
        }
    }
    let _ = channel.close().await;
    Ok((code.unwrap_or(-1), output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_capped_across_both_streams() {
        let mut output = Output::new(5);
        output.push(false, b"abc");
        output.push(true, b"de");
        assert!(!output.truncated);
        output.push(true, b"fgh");
        assert!(output.truncated);
        assert_eq!(output.stdout, b"abc");
        assert_eq!(output.stderr, b"de");
    }
}
