use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_repr::{Deserialize_repr, Serialize_repr};

pub const PROTOCOL_VERSION: u16 = 1;
pub const BOOT_COMMAND: &str = "@pam/boot";
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize_repr, Eq, PartialEq, Serialize_repr)]
#[repr(u8)]
pub enum MessageKind {
    Request = 1,
    Response = 2,
}

#[derive(Clone, Copy, Debug, Deserialize_repr, Eq, PartialEq, Serialize_repr)]
#[repr(u8)]
pub enum ResponseStatus {
    Success = 1,
    Failure = 2,
}

#[derive(Clone, Copy, Debug, Deserialize_repr, Eq, PartialEq, Serialize_repr)]
#[repr(u16)]
pub enum ErrorCode {
    InvalidMessage = 1,
    UnsupportedProtocol = 2,
    UnknownCommand = 3,
    InvalidPayload = 4,
    HandlerFailed = 5,
    WorkerUnavailable = 6,
    Unauthorized = 7,
    Internal = 8,
}

#[derive(Clone, Copy, Debug, Deserialize_repr, Eq, PartialEq, Serialize_repr)]
#[repr(u16)]
pub enum EffectKind {
    SetWindowTitle = 1,
    SetWindowVisible = 2,
    CloseWindow = 3,
}

#[derive(Clone, Copy, Debug, Default, Deserialize_repr, Eq, PartialEq, Serialize_repr)]
#[repr(u8)]
pub enum WindowTheme {
    #[default]
    System = 1,
    Light = 2,
    Dark = 3,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestEnvelope {
    pub version: u16,
    pub id: u64,
    pub kind: MessageKind,
    pub command: String,
    #[serde(default)]
    pub payload: Value,
}

impl RequestEnvelope {
    #[must_use]
    pub fn new(id: u64, command: impl Into<String>, payload: Value) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            kind: MessageKind::Request,
            command: command.into(),
            payload,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseEnvelope {
    pub version: u16,
    pub id: u64,
    pub kind: MessageKind,
    pub status: ResponseStatus,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub effects: Vec<Effect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}

impl ResponseEnvelope {
    #[must_use]
    pub fn failure(id: u64, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            kind: MessageKind::Response,
            status: ResponseStatus::Failure,
            payload: Value::Null,
            effects: Vec::new(),
            error: Some(ProtocolError {
                code,
                message: message.into(),
            }),
        }
    }

    pub fn validate_for(&self, request_id: u64) -> Result<(), String> {
        if self.version != PROTOCOL_VERSION {
            return Err(format!(
                "worker protocol {} is incompatible with host protocol {}",
                self.version, PROTOCOL_VERSION
            ));
        }
        if self.kind != MessageKind::Response {
            return Err("worker returned a non-response message".to_owned());
        }
        if self.id != request_id {
            return Err(format!(
                "worker response id {} does not match request id {request_id}",
                self.id
            ));
        }
        if self.status == ResponseStatus::Failure && self.error.is_none() {
            return Err("worker returned failure without an error".to_owned());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Effect {
    pub kind: EffectKind,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub entry: String,
    pub window: WindowConfig,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub min_width: u32,
    pub min_height: u32,
    pub resizable: bool,
    pub visible: bool,
    pub theme: WindowTheme,
}

impl WindowConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.title.trim().is_empty() {
            return Err("window title cannot be empty".to_owned());
        }
        if self.width < self.min_width || self.height < self.min_height {
            return Err("window size cannot be smaller than its minimum size".to_owned());
        }
        if self.min_width < 320 || self.min_height < 240 {
            return Err("minimum window size must be at least 320x240".to_owned());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_discriminators_as_sequential_integers() {
        let request = RequestEnvelope::new(7, "hello", serde_json::json!({"name": "Pam"}));
        let json = serde_json::to_value(request).expect("request should serialize");

        assert_eq!(json["kind"], 1);
        assert_eq!(MessageKind::Request as u8, 1);
        assert_eq!(MessageKind::Response as u8, 2);
        assert_eq!(ResponseStatus::Success as u8, 1);
        assert_eq!(ResponseStatus::Failure as u8, 2);
        assert_eq!(EffectKind::SetWindowTitle as u16, 1);
        assert_eq!(EffectKind::SetWindowVisible as u16, 2);
        assert_eq!(EffectKind::CloseWindow as u16, 3);
    }

    #[test]
    fn rejects_an_invalid_window_contract() {
        let invalid = WindowConfig {
            title: "Pam".to_owned(),
            width: 400,
            height: 300,
            min_width: 600,
            min_height: 400,
            resizable: true,
            visible: true,
            theme: WindowTheme::System,
        };

        assert!(invalid.validate().is_err());
    }

    #[test]
    fn validates_response_identity_and_protocol() {
        let response = ResponseEnvelope {
            version: PROTOCOL_VERSION,
            id: 42,
            kind: MessageKind::Response,
            status: ResponseStatus::Success,
            payload: serde_json::json!({"ready": true}),
            effects: Vec::new(),
            error: None,
        };

        assert!(response.validate_for(42).is_ok());
        assert!(response.validate_for(41).is_err());
    }
}
