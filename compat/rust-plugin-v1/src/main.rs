use pam_desktop_plugin::protocol::{
    ClientEvent, ErrorCode, PLUGIN_PROTOCOL_VERSION, PluginMetadata,
};
use pam_desktop_plugin::{Plugin, PluginContext, PluginFailure, PluginOutput, ServeError, serve};

struct CompatibilityPlugin;

impl Plugin for CompatibilityPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata {
            identifier: "compat.v1".to_owned(),
            name: "Compatibility v1".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            commands: vec!["echo".to_owned(), "fail".to_owned()],
        }
    }

    fn invoke(&mut self, context: PluginContext) -> Result<PluginOutput, PluginFailure> {
        match context.command.as_str() {
            "echo" => Ok(PluginOutput::new(serde_json::json!({
                "requestId": context.request_id,
                "payload": context.payload,
            }))
            .with_event(ClientEvent {
                name: "compat.echoed".to_owned(),
                payload: serde_json::Value::Null,
                window_id: None,
            })),
            "fail" => Err(PluginFailure::invalid_payload("compatibility failure")),
            _ => Err(PluginFailure::handler_failed(
                "unknown compatibility command",
            )),
        }
    }
}

fn stable_entrypoint() -> Result<(), ServeError> {
    serve(CompatibilityPlugin)
}

fn main() {
    assert_eq!(PLUGIN_PROTOCOL_VERSION, 1);
    let typed_error = ErrorCode::PluginFailed;
    assert_eq!(typed_error as u16, 24);
    let entrypoint: fn() -> Result<(), ServeError> = stable_entrypoint;
    let _ = entrypoint;
}
