use std::collections::HashSet;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_repr::{Deserialize_repr, Serialize_repr};

pub const PROTOCOL_VERSION: u16 = 3;
pub const BOOT_COMMAND: &str = "@pam/boot";
pub const EVENT_COMMAND: &str = "@pam/event";
pub const MAIN_WINDOW_ID: &str = "main";
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
pub const MIN_COMMAND_TIMEOUT_MS: u64 = 100;
pub const MAX_COMMAND_TIMEOUT_MS: u64 = 120_000;

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
    UnknownEvent = 9,
    RequestTimedOut = 10,
    RequestCancelled = 11,
    WorkerCrashed = 12,
    CapabilityDisabled = 13,
    PermissionDenied = 14,
    ResourceNotFound = 15,
    ResourceTooLarge = 16,
    NativeOperationFailed = 17,
    InvalidGrant = 18,
}

#[derive(Clone, Copy, Debug, Deserialize_repr, Eq, PartialEq, Serialize_repr)]
#[repr(u16)]
pub enum EffectKind {
    SetWindowTitle = 1,
    SetWindowVisible = 2,
    CloseWindow = 3,
    FocusWindow = 4,
}

#[derive(Clone, Copy, Debug, Default, Deserialize_repr, Eq, PartialEq, Serialize_repr)]
#[repr(u8)]
pub enum WindowTheme {
    #[default]
    System = 1,
    Light = 2,
    Dark = 3,
}

#[derive(Clone, Copy, Debug, Deserialize_repr, Eq, PartialEq, Serialize_repr)]
#[repr(u8)]
pub enum FileAccess {
    Read = 1,
    Write = 2,
    ReadWrite = 3,
}

impl FileAccess {
    #[must_use]
    pub const fn can_read(self) -> bool {
        matches!(self, Self::Read | Self::ReadWrite)
    }

    #[must_use]
    pub const fn can_write(self) -> bool {
        matches!(self, Self::Write | Self::ReadWrite)
    }
}

#[derive(Clone, Copy, Debug, Deserialize_repr, Eq, PartialEq, Serialize_repr)]
#[repr(u8)]
pub enum FileEntryKind {
    File = 1,
    Directory = 2,
}

#[derive(Clone, Copy, Debug, Deserialize_repr, Eq, PartialEq, Serialize_repr)]
#[repr(u8)]
pub enum DialogKind {
    OpenFile = 1,
    OpenFiles = 2,
    SaveFile = 3,
    OpenDirectory = 4,
}

#[derive(Clone, Copy, Debug, Deserialize_repr, Eq, PartialEq, Serialize_repr)]
#[repr(u8)]
pub enum FileOperation {
    ReadText = 1,
    WriteText = 2,
    List = 3,
    Metadata = 4,
    CreateDirectory = 5,
}

#[derive(Clone, Copy, Debug, Deserialize_repr, Eq, PartialEq, Serialize_repr)]
#[repr(u8)]
pub enum ClipboardOperation {
    ReadText = 1,
    WriteText = 2,
    Clear = 3,
}

#[derive(Clone, Copy, Debug, Default, Deserialize_repr, Eq, PartialEq, Serialize_repr)]
#[repr(u8)]
pub enum NotificationUrgency {
    Low = 1,
    #[default]
    Normal = 2,
    Critical = 3,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestEnvelope {
    pub version: u16,
    pub id: u64,
    pub kind: MessageKind,
    pub command: String,
    pub window_id: String,
    #[serde(default)]
    pub payload: Value,
}

impl RequestEnvelope {
    #[must_use]
    pub fn new(id: u64, command: impl Into<String>, payload: Value) -> Self {
        Self::for_window(id, command, MAIN_WINDOW_ID, payload)
    }

    #[must_use]
    pub fn for_window(
        id: u64,
        command: impl Into<String>,
        window_id: impl Into<String>,
        payload: Value,
    ) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            kind: MessageKind::Request,
            command: command.into(),
            window_id: window_id.into(),
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
    #[serde(default)]
    pub events: Vec<ClientEvent>,
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
            events: Vec::new(),
            error: Some(ProtocolError {
                code,
                message: message.into(),
            }),
        }
    }

    /// Validates that this worker response matches its host request.
    ///
    /// # Errors
    ///
    /// Returns an error when protocol version, message kind, request identity,
    /// or failure payload is inconsistent.
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
    #[serde(default = "main_window_id")]
    pub window_id: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientEvent {
    pub name: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<String>,
}

impl ClientEvent {
    /// Validates event and optional target window identifiers.
    ///
    /// # Errors
    ///
    /// Returns an error when either identifier violates the protocol grammar.
    pub fn validate(&self) -> Result<(), String> {
        validate_identifier(&self.name, "event")?;
        if let Some(window_id) = &self.window_id {
            validate_identifier(window_id, "window")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub windows: Vec<WindowConfig>,
    pub command_timeout_ms: u64,
    #[serde(default)]
    pub capabilities: NativeCapabilities,
}

impl Bootstrap {
    /// Validates the complete application bootstrap contract.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid windows, deadlines, duplicate identifiers,
    /// missing main window, or malformed native capabilities.
    pub fn validate(&self) -> Result<(), String> {
        if self.windows.is_empty() {
            return Err("desktop applications must register at least one window".to_owned());
        }
        if !(MIN_COMMAND_TIMEOUT_MS..=MAX_COMMAND_TIMEOUT_MS).contains(&self.command_timeout_ms) {
            return Err(format!(
                "command timeout must be between {MIN_COMMAND_TIMEOUT_MS} and {MAX_COMMAND_TIMEOUT_MS} milliseconds"
            ));
        }

        let mut identifiers = HashSet::with_capacity(self.windows.len());
        for window in &self.windows {
            window.validate()?;
            if !identifiers.insert(window.id.as_str()) {
                return Err(format!("window identifier {:?} is duplicated", window.id));
            }
        }
        if !identifiers.contains(MAIN_WINDOW_ID) {
            return Err(format!(
                "desktop applications must register the {MAIN_WINDOW_ID:?} window"
            ));
        }
        self.capabilities.validate()?;
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent capability switches intentionally serialize as booleans"
)]
pub struct NativeCapabilities {
    #[serde(default)]
    pub filesystem_roots: Vec<FileSystemRootConfig>,
    #[serde(default)]
    pub dialogs: bool,
    #[serde(default)]
    pub clipboard_read: bool,
    #[serde(default)]
    pub clipboard_write: bool,
    #[serde(default)]
    pub notifications: bool,
    #[serde(default)]
    pub drag_and_drop: bool,
}

impl NativeCapabilities {
    /// Validates named filesystem roots and uniqueness.
    ///
    /// # Errors
    ///
    /// Returns an error when a root contract is malformed or duplicated.
    pub fn validate(&self) -> Result<(), String> {
        let mut names = HashSet::with_capacity(self.filesystem_roots.len());
        for root in &self.filesystem_roots {
            root.validate()?;
            if !names.insert(root.name.as_str()) {
                return Err(format!(
                    "filesystem root identifier {:?} is duplicated",
                    root.name
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSystemRootConfig {
    pub name: String,
    pub path: String,
    pub access: FileAccess,
}

impl FileSystemRootConfig {
    /// Validates the filesystem root identifier and configured path.
    ///
    /// # Errors
    ///
    /// Returns an error when the identifier or path is invalid.
    pub fn validate(&self) -> Result<(), String> {
        validate_identifier(&self.name, "filesystem root")?;
        if self.path.trim().is_empty() || self.path.contains('\0') {
            return Err(format!(
                "filesystem root {:?} must declare a non-empty path",
                self.name
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowConfig {
    pub id: String,
    pub entry: String,
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
    /// Validates a native window contract.
    ///
    /// # Errors
    ///
    /// Returns an error for unsafe entries, invalid identifiers, empty titles,
    /// or dimensions below the supported minimum.
    pub fn validate(&self) -> Result<(), String> {
        validate_identifier(&self.id, "window")?;
        let entry = Path::new(&self.entry);
        if self.entry.is_empty()
            || entry.is_absolute()
            || entry
                .components()
                .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
        {
            return Err(format!(
                "entry for window {:?} must be a relative project path",
                self.id
            ));
        }
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

/// Validates a protocol identifier.
///
/// # Errors
///
/// Returns an error unless the value begins with an ASCII letter and contains
/// at most 64 permitted ASCII identifier characters.
pub fn validate_identifier(value: &str, label: &str) -> Result<(), String> {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return Err(format!("{label} identifier cannot be empty"));
    };
    if !first.is_ascii_alphabetic()
        || value.len() > 64
        || !characters
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
    {
        return Err(format!(
            "{label} identifier must begin with a letter and contain at most 64 ASCII letters, numbers, dots, dashes, or underscores"
        ));
    }
    Ok(())
}

fn main_window_id() -> String {
    MAIN_WINDOW_ID.to_owned()
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
        assert_eq!(EffectKind::FocusWindow as u16, 4);
        assert_eq!(FileAccess::Read as u8, 1);
        assert_eq!(FileAccess::Write as u8, 2);
        assert_eq!(FileAccess::ReadWrite as u8, 3);
        assert_eq!(FileEntryKind::File as u8, 1);
        assert_eq!(FileEntryKind::Directory as u8, 2);
        assert_eq!(DialogKind::OpenFile as u8, 1);
        assert_eq!(DialogKind::OpenFiles as u8, 2);
        assert_eq!(DialogKind::SaveFile as u8, 3);
        assert_eq!(DialogKind::OpenDirectory as u8, 4);
        assert_eq!(FileOperation::ReadText as u8, 1);
        assert_eq!(FileOperation::WriteText as u8, 2);
        assert_eq!(FileOperation::List as u8, 3);
        assert_eq!(FileOperation::Metadata as u8, 4);
        assert_eq!(FileOperation::CreateDirectory as u8, 5);
        assert_eq!(ClipboardOperation::ReadText as u8, 1);
        assert_eq!(ClipboardOperation::WriteText as u8, 2);
        assert_eq!(ClipboardOperation::Clear as u8, 3);
        assert_eq!(NotificationUrgency::Low as u8, 1);
        assert_eq!(NotificationUrgency::Normal as u8, 2);
        assert_eq!(NotificationUrgency::Critical as u8, 3);
    }

    #[test]
    fn rejects_an_invalid_window_contract() {
        let invalid = WindowConfig {
            id: MAIN_WINDOW_ID.to_owned(),
            entry: "resources/index.html".to_owned(),
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
            events: Vec::new(),
            error: None,
        };

        assert!(response.validate_for(42).is_ok());
        assert!(response.validate_for(41).is_err());
    }

    #[test]
    fn validates_multi_window_bootstrap_contracts() {
        let bootstrap = Bootstrap {
            windows: vec![
                WindowConfig {
                    id: MAIN_WINDOW_ID.to_owned(),
                    entry: "resources/index.html".to_owned(),
                    title: "Main".to_owned(),
                    width: 1024,
                    height: 720,
                    min_width: 640,
                    min_height: 480,
                    resizable: true,
                    visible: true,
                    theme: WindowTheme::System,
                },
                WindowConfig {
                    id: "settings".to_owned(),
                    entry: "resources/settings.html".to_owned(),
                    title: "Settings".to_owned(),
                    width: 720,
                    height: 520,
                    min_width: 640,
                    min_height: 480,
                    resizable: true,
                    visible: false,
                    theme: WindowTheme::Dark,
                },
            ],
            command_timeout_ms: 30_000,
            capabilities: NativeCapabilities {
                filesystem_roots: vec![FileSystemRootConfig {
                    name: "data".to_owned(),
                    path: "storage".to_owned(),
                    access: FileAccess::ReadWrite,
                }],
                dialogs: true,
                clipboard_read: true,
                clipboard_write: true,
                notifications: true,
                drag_and_drop: true,
            },
        };

        assert!(bootstrap.validate().is_ok());
    }

    #[test]
    fn rejects_duplicate_filesystem_capabilities() {
        let capabilities = NativeCapabilities {
            filesystem_roots: vec![
                FileSystemRootConfig {
                    name: "data".to_owned(),
                    path: "storage".to_owned(),
                    access: FileAccess::Read,
                },
                FileSystemRootConfig {
                    name: "data".to_owned(),
                    path: "cache".to_owned(),
                    access: FileAccess::Write,
                },
            ],
            ..NativeCapabilities::default()
        };

        assert!(capabilities.validate().is_err());
    }
}
