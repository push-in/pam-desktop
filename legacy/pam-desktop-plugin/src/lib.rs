//! Safe, process-isolated Rust plugin SDK for Pam Desktop.
//!
//! Plugins are ordinary executables supervised by the desktop host. They use
//! a versioned JSON-lines protocol over standard input/output, so a plugin
//! crash cannot corrupt the Servo process and no dynamic-library ABI is
//! exposed.

use std::fmt::{Display, Formatter};
use std::io::{self, BufRead, Read, Write};

use pam_desktop_protocol::{
    ClientEvent, ErrorCode, MAX_MESSAGE_BYTES, PLUGIN_BOOT_COMMAND, PluginMetadata,
    PluginRequestEnvelope, PluginResponseEnvelope,
};
use serde_json::Value;

pub use pam_desktop_protocol as protocol;

/// A plugin invocation delivered by the Pam Desktop host.
#[derive(Clone, Debug)]
pub struct PluginContext {
    pub request_id: u64,
    pub command: String,
    pub payload: Value,
}

/// Successful plugin data and optional application events.
#[derive(Clone, Debug, Default)]
pub struct PluginOutput {
    pub payload: Value,
    pub events: Vec<ClientEvent>,
}

impl PluginOutput {
    #[must_use]
    pub fn new(payload: Value) -> Self {
        Self {
            payload,
            events: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_event(mut self, event: ClientEvent) -> Self {
        self.events.push(event);
        self
    }
}

/// A typed failure returned by a plugin command.
#[derive(Clone, Debug)]
pub struct PluginFailure {
    pub code: ErrorCode,
    pub message: String,
}

impl PluginFailure {
    #[must_use]
    pub fn invalid_payload(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::InvalidPayload,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn handler_failed(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::HandlerFailed,
            message: message.into(),
        }
    }
}

/// A process-isolated native plugin.
pub trait Plugin {
    fn metadata(&self) -> PluginMetadata;

    /// Handles one exported command.
    ///
    /// # Errors
    ///
    /// Returns a typed failure that is forwarded to the authorized desktop
    /// caller without terminating the plugin process.
    fn invoke(&mut self, context: PluginContext) -> Result<PluginOutput, PluginFailure>;
}

/// Fatal transport or contract error in the plugin process.
#[derive(Debug)]
pub struct ServeError(String);

impl Display for ServeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ServeError {}

/// Runs a plugin until the host closes standard input.
///
/// # Errors
///
/// Returns a fatal error when plugin metadata is invalid or the supervised
/// JSON-lines transport cannot be read or written safely.
pub fn serve(plugin: impl Plugin) -> Result<(), ServeError> {
    serve_with_io(plugin, io::stdin().lock(), io::stdout().lock())
}

fn serve_with_io(
    mut plugin: impl Plugin,
    mut input: impl BufRead,
    mut output: impl Write,
) -> Result<(), ServeError> {
    let metadata = plugin.metadata();
    metadata
        .validate()
        .map_err(|error| ServeError(format!("invalid plugin metadata: {error}")))?;

    loop {
        let mut line = String::new();
        let bytes = Read::take(&mut input, (MAX_MESSAGE_BYTES + 1) as u64)
            .read_line(&mut line)
            .map_err(|error| ServeError(format!("cannot read plugin request: {error}")))?;
        if bytes == 0 {
            return Ok(());
        }
        if bytes > MAX_MESSAGE_BYTES || !line.ends_with('\n') {
            return Err(ServeError(
                "plugin request exceeds the one-megabyte limit or is incomplete".to_owned(),
            ));
        }

        let request = serde_json::from_str::<PluginRequestEnvelope>(&line)
            .map_err(|error| ServeError(format!("host sent invalid plugin JSON: {error}")))?;
        request
            .validate()
            .map_err(|error| ServeError(format!("host sent an invalid plugin request: {error}")))?;
        let response = if request.command == PLUGIN_BOOT_COMMAND {
            let payload = serde_json::to_value(&metadata).map_err(|error| {
                ServeError(format!("cannot serialize plugin metadata: {error}"))
            })?;
            PluginResponseEnvelope::success(request.id, payload, Vec::new())
        } else if !metadata.commands.contains(&request.command) {
            PluginResponseEnvelope::failure(
                request.id,
                ErrorCode::UnknownCommand,
                format!("Plugin command {:?} is not exported.", request.command),
            )
        } else {
            let context = PluginContext {
                request_id: request.id,
                command: request.command,
                payload: request.payload,
            };
            match plugin.invoke(context) {
                Ok(result) => {
                    for event in &result.events {
                        event.validate().map_err(|error| {
                            ServeError(format!("plugin emitted an invalid event: {error}"))
                        })?;
                    }
                    PluginResponseEnvelope::success(request.id, result.payload, result.events)
                }
                Err(error) => {
                    PluginResponseEnvelope::failure(request.id, error.code, error.message)
                }
            }
        };

        let encoded = serde_json::to_vec(&response)
            .map_err(|error| ServeError(format!("cannot serialize plugin response: {error}")))?;
        if encoded.len() > MAX_MESSAGE_BYTES {
            return Err(ServeError(
                "plugin response exceeds the one-megabyte limit".to_owned(),
            ));
        }
        output
            .write_all(&encoded)
            .and_then(|()| output.write_all(b"\n"))
            .and_then(|()| output.flush())
            .map_err(|error| ServeError(format!("cannot write plugin response: {error}")))?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ExamplePlugin;

    impl Plugin for ExamplePlugin {
        fn metadata(&self) -> PluginMetadata {
            PluginMetadata {
                identifier: "example".to_owned(),
                name: "Example".to_owned(),
                version: "1.0.0".to_owned(),
                commands: vec!["echo".to_owned()],
            }
        }

        fn invoke(&mut self, context: PluginContext) -> Result<PluginOutput, PluginFailure> {
            Ok(PluginOutput::new(context.payload))
        }
    }

    #[test]
    fn serves_boot_invoke_and_unknown_command_contracts() {
        let input = concat!(
            "{\"version\":1,\"id\":1,\"kind\":1,\"command\":\"@pam/plugin/boot\",\"payload\":null}\n",
            "{\"version\":1,\"id\":2,\"kind\":1,\"command\":\"echo\",\"payload\":{\"safe\":true}}\n",
            "{\"version\":1,\"id\":3,\"kind\":1,\"command\":\"missing\",\"payload\":null}\n",
        );
        let mut output = Vec::new();
        serve_with_io(ExamplePlugin, input.as_bytes(), &mut output)
            .expect("plugin fixture should serve");
        let lines = String::from_utf8(output).expect("plugin output should be UTF-8");
        let responses = lines
            .lines()
            .map(|line| {
                serde_json::from_str::<PluginResponseEnvelope>(line)
                    .expect("plugin response should be valid")
            })
            .collect::<Vec<_>>();

        assert_eq!(responses.len(), 3);
        assert_eq!(responses[0].id, 1);
        assert_eq!(responses[1].payload, serde_json::json!({"safe": true}));
        assert_eq!(
            responses[2].error.as_ref().map(|error| error.code),
            Some(ErrorCode::UnknownCommand)
        );
    }
}
