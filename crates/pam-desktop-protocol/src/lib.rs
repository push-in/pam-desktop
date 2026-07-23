use std::collections::HashSet;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_repr::{Deserialize_repr, Serialize_repr};

pub const PROTOCOL_VERSION: u16 = 6;
pub const PUBLIC_API_VERSION: u16 = 1;
pub const PLUGIN_PROTOCOL_VERSION: u16 = 1;
pub const UPDATE_FEED_VERSION: u16 = 1;
pub const BOOT_COMMAND: &str = "@pam/boot";
pub const EVENT_COMMAND: &str = "@pam/event";
pub const JOB_COMMAND: &str = "@pam/job";
pub const PLUGIN_BOOT_COMMAND: &str = "@pam/plugin/boot";
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
    UpdateDisabled = 19,
    UpdateUnavailable = 20,
    UpdateIntegrityFailed = 21,
    UpdateInstallFailed = 22,
    PluginUnavailable = 23,
    PluginFailed = 24,
    ShortcutUnavailable = 25,
    BackgroundJobFailed = 26,
}

#[derive(Clone, Copy, Debug, Deserialize_repr, Eq, PartialEq, Serialize_repr)]
#[repr(u16)]
pub enum EffectKind {
    SetWindowTitle = 1,
    SetWindowVisible = 2,
    CloseWindow = 3,
    FocusWindow = 4,
    SetMenuItemEnabled = 5,
    SetMenuItemChecked = 6,
    SetTrayVisible = 7,
}

#[derive(Clone, Copy, Debug, Default, Deserialize_repr, Eq, PartialEq, Serialize_repr)]
#[repr(u8)]
pub enum WindowTheme {
    #[default]
    System = 1,
    Light = 2,
    Dark = 3,
}

#[derive(Clone, Copy, Debug, Default, Deserialize_repr, Eq, PartialEq, Serialize_repr)]
#[repr(u8)]
pub enum ApplicationCategory {
    Development = 1,
    Productivity = 2,
    Graphics = 3,
    AudioVideo = 4,
    Network = 5,
    #[default]
    Utility = 6,
    Game = 7,
    Education = 8,
}

#[derive(Clone, Copy, Debug, Default, Deserialize_repr, Eq, Hash, PartialEq, Serialize_repr)]
#[repr(u8)]
pub enum UpdatePolicy {
    #[default]
    Manual = 1,
    Notify = 2,
    Automatic = 3,
}

#[derive(Clone, Copy, Debug, Deserialize_repr, Eq, Hash, PartialEq, Serialize_repr)]
#[repr(u8)]
pub enum UpdatePlatform {
    Linux = 1,
    Windows = 2,
    MacOs = 3,
}

#[derive(Clone, Copy, Debug, Deserialize_repr, Eq, Hash, PartialEq, Serialize_repr)]
#[repr(u8)]
pub enum UpdateArtifactKind {
    PortableArchive = 1,
    NativeInstaller = 2,
}

#[derive(Clone, Copy, Debug, Deserialize_repr, Eq, Hash, PartialEq, Serialize_repr)]
#[repr(u8)]
pub enum UpdateState {
    Disabled = 1,
    Idle = 2,
    Checking = 3,
    Available = 4,
    Downloading = 5,
    Ready = 6,
    Applying = 7,
    UpToDate = 8,
    Failed = 9,
}

#[derive(Clone, Copy, Debug, Deserialize_repr, Eq, PartialEq, Serialize_repr)]
#[repr(u8)]
pub enum MenuItemKind {
    Command = 1,
    Checkbox = 2,
    Separator = 3,
    Submenu = 4,
}

#[derive(Clone, Copy, Debug, Default, Deserialize_repr, Eq, PartialEq, Serialize_repr)]
#[repr(u8)]
pub enum TrayCloseBehavior {
    #[default]
    Exit = 1,
    Hide = 2,
}

#[derive(Clone, Copy, Debug, Deserialize_repr, Eq, PartialEq, Serialize_repr)]
#[repr(u8)]
pub enum ShortcutState {
    Pressed = 1,
    Released = 2,
}

#[derive(Clone, Copy, Debug, Default, Deserialize_repr, Eq, PartialEq, Serialize_repr)]
#[repr(u8)]
pub enum JobOverlapPolicy {
    #[default]
    Skip = 1,
    Wait = 2,
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginRequestEnvelope {
    pub version: u16,
    pub id: u64,
    pub kind: MessageKind,
    pub command: String,
    #[serde(default)]
    pub payload: Value,
}

impl PluginRequestEnvelope {
    #[must_use]
    pub fn new(id: u64, command: impl Into<String>, payload: Value) -> Self {
        Self {
            version: PLUGIN_PROTOCOL_VERSION,
            id,
            kind: MessageKind::Request,
            command: command.into(),
            payload,
        }
    }

    /// Validates an inbound Rust plugin request.
    ///
    /// # Errors
    ///
    /// Returns an error when the plugin protocol, identity, message kind, or
    /// command is invalid.
    pub fn validate(&self) -> Result<(), String> {
        if self.version != PLUGIN_PROTOCOL_VERSION {
            return Err(format!(
                "plugin protocol {} is incompatible with SDK protocol {}",
                self.version, PLUGIN_PROTOCOL_VERSION
            ));
        }
        if self.id == 0 || self.kind != MessageKind::Request {
            return Err("plugin request identity is invalid".to_owned());
        }
        if self.command != PLUGIN_BOOT_COMMAND {
            validate_identifier(&self.command, "plugin command")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginResponseEnvelope {
    pub version: u16,
    pub id: u64,
    pub kind: MessageKind,
    pub status: ResponseStatus,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub events: Vec<ClientEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}

impl PluginResponseEnvelope {
    #[must_use]
    pub fn success(id: u64, payload: Value, events: Vec<ClientEvent>) -> Self {
        Self {
            version: PLUGIN_PROTOCOL_VERSION,
            id,
            kind: MessageKind::Response,
            status: ResponseStatus::Success,
            payload,
            events,
            error: None,
        }
    }

    #[must_use]
    pub fn failure(id: u64, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            version: PLUGIN_PROTOCOL_VERSION,
            id,
            kind: MessageKind::Response,
            status: ResponseStatus::Failure,
            payload: Value::Null,
            events: Vec::new(),
            error: Some(ProtocolError {
                code,
                message: message.into(),
            }),
        }
    }

    /// Validates that a plugin response matches its host request.
    ///
    /// # Errors
    ///
    /// Returns an error for incompatible protocol, mismatched identity,
    /// invalid events, or an incomplete failure.
    pub fn validate_for(&self, request_id: u64) -> Result<(), String> {
        if self.version != PLUGIN_PROTOCOL_VERSION {
            return Err(format!(
                "plugin protocol {} is incompatible with host plugin protocol {}",
                self.version, PLUGIN_PROTOCOL_VERSION
            ));
        }
        if self.kind != MessageKind::Response || self.id != request_id {
            return Err("plugin response identity does not match its request".to_owned());
        }
        if self.status == ResponseStatus::Failure && self.error.is_none() {
            return Err("plugin returned failure without an error".to_owned());
        }
        for event in &self.events {
            event.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginMetadata {
    pub identifier: String,
    pub name: String,
    pub version: String,
    pub commands: Vec<String>,
}

impl PluginMetadata {
    /// Validates plugin identity and its exported command surface.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed metadata or duplicate commands.
    pub fn validate(&self) -> Result<(), String> {
        validate_identifier(&self.identifier, "plugin")?;
        validate_text(&self.name, "plugin name", 80, false)?;
        validate_version(&self.version)?;
        if self.commands.is_empty() {
            return Err("Rust plugins must export at least one command".to_owned());
        }
        let mut commands = HashSet::with_capacity(self.commands.len());
        for command in &self.commands {
            validate_identifier(command, "plugin command")?;
            if !commands.insert(command.as_str()) {
                return Err(format!("plugin command {command:?} is duplicated"));
            }
        }
        Ok(())
    }
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
    pub manifest: ApplicationManifest,
    pub windows: Vec<WindowConfig>,
    pub command_timeout_ms: u64,
    #[serde(default)]
    pub capabilities: NativeCapabilities,
    #[serde(default)]
    pub shell: ShellConfig,
    #[serde(default)]
    pub background_jobs: Vec<BackgroundJobConfig>,
    #[serde(default)]
    pub rust_plugins: Vec<RustPluginConfig>,
    #[serde(default)]
    pub php_plugins: Vec<String>,
}

impl Bootstrap {
    /// Validates the complete application bootstrap contract.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid manifest, windows, deadlines, duplicate
    /// identifiers, missing main window, or malformed native capabilities.
    pub fn validate(&self) -> Result<(), String> {
        self.manifest.validate()?;
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
        self.shell.validate()?;

        let mut jobs = HashSet::with_capacity(self.background_jobs.len());
        for job in &self.background_jobs {
            job.validate()?;
            if !jobs.insert(job.id.as_str()) {
                return Err(format!(
                    "background job identifier {:?} is duplicated",
                    job.id
                ));
            }
        }

        let mut plugins = HashSet::with_capacity(self.rust_plugins.len());
        for plugin in &self.rust_plugins {
            plugin.validate()?;
            if !plugins.insert(plugin.id.as_str()) {
                return Err(format!(
                    "Rust plugin identifier {:?} is duplicated",
                    plugin.id
                ));
            }
        }
        let mut php_plugins = HashSet::with_capacity(self.php_plugins.len());
        for plugin in &self.php_plugins {
            validate_identifier(plugin, "PHP plugin")?;
            if !php_plugins.insert(plugin.as_str()) {
                return Err(format!("PHP plugin identifier {plugin:?} is duplicated"));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationManifest {
    pub identifier: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub publisher: String,
    pub category: ApplicationCategory,
    pub icon: String,
    #[serde(default)]
    pub bundle_excludes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updates: Option<UpdateConfig>,
}

impl ApplicationManifest {
    /// Validates distributable application identity, presentation and bundle
    /// exclusions.
    ///
    /// # Errors
    ///
    /// Returns an error when a field cannot be represented safely in Linux
    /// application metadata or refers outside the project.
    pub fn validate(&self) -> Result<(), String> {
        validate_application_identifier(&self.identifier)?;
        validate_text(&self.name, "application name", 80, false)?;
        validate_version(&self.version)?;
        validate_text(&self.description, "application description", 256, true)?;
        validate_text(&self.publisher, "application publisher", 128, false)?;
        validate_relative_project_path(&self.icon, "application icon")?;
        let extension = Path::new(&self.icon)
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase);
        if !matches!(extension.as_deref(), Some("png" | "svg")) {
            return Err("application icon must be a PNG or SVG project asset".to_owned());
        }

        let mut excludes = HashSet::with_capacity(self.bundle_excludes.len());
        for path in &self.bundle_excludes {
            validate_relative_project_path(path, "bundle exclusion")?;
            if !excludes.insert(path.as_str()) {
                return Err(format!("bundle exclusion {path:?} is duplicated"));
            }
            if matches!(
                path.as_str(),
                "app.php" | "composer.json" | "composer.lock" | "resources" | "vendor"
            ) {
                return Err(format!(
                    "required desktop project path {path:?} cannot be excluded"
                ));
            }
        }
        if let Some(updates) = &self.updates {
            updates.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateConfig {
    pub endpoint: String,
    pub channel: String,
    pub public_key: String,
    pub policy: UpdatePolicy,
}

impl UpdateConfig {
    /// Validates the trusted source and Ed25519 verification key used by the
    /// application updater.
    ///
    /// # Errors
    ///
    /// Returns an error for an insecure remote URL, malformed channel, or
    /// invalid 32-byte lowercase hexadecimal public key.
    pub fn validate(&self) -> Result<(), String> {
        validate_update_url(&self.endpoint, "update endpoint")?;
        validate_identifier(&self.channel, "update channel")?;
        validate_lower_hex(&self.public_key, 64, "update public key")
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateArtifact {
    pub platform: UpdatePlatform,
    pub architecture: String,
    pub kind: UpdateArtifactKind,
    pub url: String,
    pub bytes: u64,
    pub sha256: String,
}

impl UpdateArtifact {
    /// Validates one immutable release artifact.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed target metadata, URL, byte length, or
    /// SHA-256 digest.
    pub fn validate(&self) -> Result<(), String> {
        validate_identifier(&self.architecture, "update architecture")?;
        validate_update_url(&self.url, "update artifact URL")?;
        if self.bytes == 0 {
            return Err("update artifact size must be greater than zero".to_owned());
        }
        validate_lower_hex(&self.sha256, 64, "update artifact SHA-256")
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateRelease {
    pub schema_version: u16,
    pub application_id: String,
    pub channel: String,
    pub version: String,
    pub published_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_url: Option<String>,
    pub artifacts: Vec<UpdateArtifact>,
}

impl UpdateRelease {
    /// Validates a signed release payload independently from its signature.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported feed schema, mismatched identifiers,
    /// invalid release metadata, or duplicate target artifacts.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != UPDATE_FEED_VERSION {
            return Err(format!(
                "update feed schema {} is unsupported; expected {UPDATE_FEED_VERSION}",
                self.schema_version
            ));
        }
        validate_application_identifier(&self.application_id)?;
        validate_identifier(&self.channel, "update channel")?;
        validate_version(&self.version)?;
        validate_text(
            &self.published_at,
            "update publication timestamp",
            64,
            false,
        )?;
        if let Some(notes_url) = &self.notes_url {
            validate_update_url(notes_url, "update notes URL")?;
        }
        if self.artifacts.is_empty() {
            return Err("update release must contain at least one artifact".to_owned());
        }

        let mut targets = HashSet::with_capacity(self.artifacts.len());
        for artifact in &self.artifacts {
            artifact.validate()?;
            if !targets.insert((
                artifact.platform,
                artifact.architecture.as_str(),
                artifact.kind,
            )) {
                return Err(format!(
                    "update artifact target {:?}/{} ({:?}) is duplicated",
                    artifact.platform, artifact.architecture, artifact.kind
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignedUpdateRelease {
    pub release: UpdateRelease,
    pub signature: String,
}

impl SignedUpdateRelease {
    /// Validates the feed shape before cryptographic verification.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid release data or a malformed Ed25519
    /// signature.
    pub fn validate(&self) -> Result<(), String> {
        self.release.validate()?;
        validate_lower_hex(&self.signature, 128, "update signature")
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

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShellConfig {
    #[serde(default)]
    pub menus: Vec<MenuConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tray: Option<TrayConfig>,
    #[serde(default)]
    pub shortcuts: Vec<GlobalShortcutConfig>,
}

impl ShellConfig {
    /// Validates native menu, tray, and global shortcut configuration.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed or duplicated identifiers, an invalid
    /// tray menu reference, or duplicate shortcut accelerators.
    pub fn validate(&self) -> Result<(), String> {
        let mut menu_ids = HashSet::with_capacity(self.menus.len());
        let mut item_ids = HashSet::new();
        let mut item_count = 0_usize;
        for menu in &self.menus {
            menu.validate(&mut item_ids, &mut item_count)?;
            if !menu_ids.insert(menu.id.as_str()) {
                return Err(format!("menu identifier {:?} is duplicated", menu.id));
            }
        }
        if item_count > 256 {
            return Err("native menus cannot contain more than 256 items".to_owned());
        }
        if self.menus.len() > 1 {
            return Err("the stable Linux shell accepts one tray menu per application".to_owned());
        }
        if !self.menus.is_empty() && self.tray.is_none() {
            return Err("native menus require a tray configuration on Linux".to_owned());
        }
        if let Some(tray) = &self.tray {
            tray.validate()?;
            if !menu_ids.contains(tray.menu_id.as_str()) {
                return Err(format!(
                    "tray menu {:?} is not registered in the shell",
                    tray.menu_id
                ));
            }
        }

        let mut shortcut_ids = HashSet::with_capacity(self.shortcuts.len());
        let mut accelerators = HashSet::with_capacity(self.shortcuts.len());
        for shortcut in &self.shortcuts {
            shortcut.validate()?;
            if !shortcut_ids.insert(shortcut.id.as_str()) {
                return Err(format!(
                    "global shortcut identifier {:?} is duplicated",
                    shortcut.id
                ));
            }
            let normalized = shortcut.accelerator.to_ascii_lowercase();
            if !accelerators.insert(normalized) {
                return Err(format!(
                    "global shortcut accelerator {:?} is duplicated",
                    shortcut.accelerator
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MenuConfig {
    pub id: String,
    pub label: String,
    pub items: Vec<MenuItemConfig>,
}

impl MenuConfig {
    fn validate<'a>(
        &'a self,
        item_ids: &mut HashSet<&'a str>,
        item_count: &mut usize,
    ) -> Result<(), String> {
        validate_identifier(&self.id, "menu")?;
        validate_text(&self.label, "menu label", 80, false)?;
        if self.items.is_empty() {
            return Err(format!("menu {:?} must contain at least one item", self.id));
        }
        for item in &self.items {
            item.validate(1, item_ids, item_count)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MenuItemConfig {
    pub kind: MenuItemKind,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub checked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accelerator: Option<String>,
    #[serde(default)]
    pub items: Vec<MenuItemConfig>,
}

impl MenuItemConfig {
    fn validate<'a>(
        &'a self,
        depth: usize,
        item_ids: &mut HashSet<&'a str>,
        item_count: &mut usize,
    ) -> Result<(), String> {
        if depth > 8 {
            return Err("native menu nesting cannot exceed eight levels".to_owned());
        }
        *item_count = item_count.saturating_add(1);
        match self.kind {
            MenuItemKind::Separator => {
                if !self.id.is_empty()
                    || !self.label.is_empty()
                    || self.checked
                    || self.accelerator.is_some()
                    || !self.items.is_empty()
                {
                    return Err(
                        "menu separators cannot define identity, text, state, or children"
                            .to_owned(),
                    );
                }
            }
            MenuItemKind::Command | MenuItemKind::Checkbox => {
                validate_identifier(&self.id, "menu item")?;
                validate_text(&self.label, "menu item label", 80, false)?;
                if !item_ids.insert(self.id.as_str()) {
                    return Err(format!("menu item identifier {:?} is duplicated", self.id));
                }
                if !self.items.is_empty() {
                    return Err(format!("menu item {:?} cannot contain children", self.id));
                }
                if self.kind == MenuItemKind::Command && self.checked {
                    return Err(format!("command menu item {:?} cannot be checked", self.id));
                }
                if let Some(accelerator) = &self.accelerator {
                    validate_accelerator(accelerator)?;
                }
            }
            MenuItemKind::Submenu => {
                validate_identifier(&self.id, "submenu")?;
                validate_text(&self.label, "submenu label", 80, false)?;
                if !item_ids.insert(self.id.as_str()) {
                    return Err(format!("menu item identifier {:?} is duplicated", self.id));
                }
                if self.checked || self.accelerator.is_some() || self.items.is_empty() {
                    return Err(format!(
                        "submenu {:?} must contain children and cannot be checked or accelerated",
                        self.id
                    ));
                }
                for item in &self.items {
                    item.validate(depth + 1, item_ids, item_count)?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrayConfig {
    pub menu_id: String,
    pub tooltip: String,
    #[serde(default)]
    pub close_behavior: TrayCloseBehavior,
}

impl TrayConfig {
    fn validate(&self) -> Result<(), String> {
        validate_identifier(&self.menu_id, "tray menu")?;
        validate_text(&self.tooltip, "tray tooltip", 128, false)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlobalShortcutConfig {
    pub id: String,
    pub accelerator: String,
}

impl GlobalShortcutConfig {
    fn validate(&self) -> Result<(), String> {
        validate_identifier(&self.id, "global shortcut")?;
        validate_accelerator(&self.accelerator)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackgroundJobConfig {
    pub id: String,
    pub interval_ms: u64,
    #[serde(default)]
    pub initial_delay_ms: u64,
    pub timeout_ms: u64,
    #[serde(default)]
    pub overlap: JobOverlapPolicy,
}

impl BackgroundJobConfig {
    /// Validates a host-scheduled PHP background job.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid identity, cadence, delay, or timeout.
    pub fn validate(&self) -> Result<(), String> {
        const MIN_INTERVAL_MS: u64 = 1_000;
        const MAX_INTERVAL_MS: u64 = 86_400_000;
        validate_identifier(&self.id, "background job")?;
        if !(MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&self.interval_ms) {
            return Err(format!(
                "background job {:?} interval must be between {MIN_INTERVAL_MS} and {MAX_INTERVAL_MS} milliseconds",
                self.id
            ));
        }
        if self.initial_delay_ms > MAX_INTERVAL_MS {
            return Err(format!(
                "background job {:?} initial delay cannot exceed {MAX_INTERVAL_MS} milliseconds",
                self.id
            ));
        }
        if !(MIN_COMMAND_TIMEOUT_MS..=MAX_COMMAND_TIMEOUT_MS).contains(&self.timeout_ms) {
            return Err(format!(
                "background job {:?} timeout must be between {MIN_COMMAND_TIMEOUT_MS} and {MAX_COMMAND_TIMEOUT_MS} milliseconds",
                self.id
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RustPluginConfig {
    pub id: String,
    pub executable: String,
    #[serde(default)]
    pub arguments: Vec<String>,
    pub timeout_ms: u64,
}

impl RustPluginConfig {
    /// Validates a supervised Rust sidecar declaration.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid identity, unsafe project path, malformed
    /// arguments, or an unsupported timeout.
    pub fn validate(&self) -> Result<(), String> {
        validate_identifier(&self.id, "Rust plugin")?;
        validate_relative_project_path(&self.executable, "Rust plugin executable")?;
        if self.arguments.len() > 32 {
            return Err(format!(
                "Rust plugin {:?} cannot declare more than 32 arguments",
                self.id
            ));
        }
        for argument in &self.arguments {
            if argument.len() > 1_024 || argument.contains('\0') {
                return Err(format!(
                    "Rust plugin {:?} has an invalid process argument",
                    self.id
                ));
            }
        }
        if !(MIN_COMMAND_TIMEOUT_MS..=MAX_COMMAND_TIMEOUT_MS).contains(&self.timeout_ms) {
            return Err(format!(
                "Rust plugin {:?} timeout must be between {MIN_COMMAND_TIMEOUT_MS} and {MAX_COMMAND_TIMEOUT_MS} milliseconds",
                self.id
            ));
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

fn validate_accelerator(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 64 || value.contains(char::is_whitespace) {
        return Err("keyboard accelerators must contain 1-64 non-whitespace characters".to_owned());
    }
    let tokens = value.split('+').collect::<Vec<_>>();
    if tokens.is_empty() || tokens.iter().any(|token| token.is_empty()) {
        return Err(format!("keyboard accelerator {value:?} is malformed"));
    }
    let mut modifiers = HashSet::new();
    for modifier in &tokens[..tokens.len().saturating_sub(1)] {
        let normalized = modifier.to_ascii_lowercase();
        if !matches!(
            normalized.as_str(),
            "alt" | "ctrl" | "control" | "shift" | "super" | "cmd" | "command" | "cmdorctrl"
        ) || !modifiers.insert(normalized)
        {
            return Err(format!(
                "keyboard accelerator {value:?} has an invalid or duplicate modifier"
            ));
        }
    }
    let key = tokens
        .last()
        .expect("a non-empty accelerator contains a key token");
    if key.len() > 24
        || !key
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        return Err(format!("keyboard accelerator {value:?} has an invalid key"));
    }
    Ok(())
}

fn validate_application_identifier(value: &str) -> Result<(), String> {
    if value.len() > 155 || value.split('.').count() < 2 {
        return Err(
            "application identifier must be a reverse-DNS name of at most 155 bytes".to_owned(),
        );
    }
    for segment in value.split('.') {
        let mut characters = segment.chars();
        let Some(first) = characters.next() else {
            return Err("application identifier cannot contain empty segments".to_owned());
        };
        if !first.is_ascii_lowercase()
            || !characters.all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
            })
        {
            return Err(
                "application identifier segments must begin with a lowercase letter and contain lowercase letters, numbers, or dashes"
                    .to_owned(),
            );
        }
    }
    Ok(())
}

fn validate_text(
    value: &str,
    label: &str,
    maximum_bytes: usize,
    allow_empty: bool,
) -> Result<(), String> {
    if (!allow_empty && value.trim().is_empty())
        || value.len() > maximum_bytes
        || value
            .chars()
            .any(|character| character.is_control() || matches!(character, '\n' | '\r'))
    {
        return Err(format!(
            "{label} must contain {} through {maximum_bytes} printable UTF-8 bytes",
            usize::from(!allow_empty)
        ));
    }
    Ok(())
}

fn validate_version(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
        || !value.bytes().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, b'.' | b'+' | b'-' | b'~')
        })
    {
        return Err(
            "application version must begin with a number and contain at most 64 ASCII letters, numbers, dots, pluses, dashes, or tildes"
                .to_owned(),
        );
    }
    Ok(())
}

fn validate_relative_project_path(value: &str, label: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains('\0')
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err(format!(
            "{label} must be a relative path inside the project"
        ));
    }
    Ok(())
}

fn validate_lower_hex(value: &str, expected_bytes: usize, label: &str) -> Result<(), String> {
    if value.len() != expected_bytes
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!(
            "{label} must contain exactly {expected_bytes} lowercase hexadecimal characters"
        ));
    }
    Ok(())
}

fn validate_update_url(value: &str, label: &str) -> Result<(), String> {
    let parsed = url::Url::parse(value).ok();
    let trusted = parsed.as_ref().is_some_and(|url| {
        let has_authority = url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none();
        let secure = url.scheme() == "https";
        let loopback = url.scheme() == "http"
            && url.host().is_some_and(|host| match host {
                url::Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
                url::Host::Ipv4(address) => address.is_loopback(),
                url::Host::Ipv6(address) => address.is_loopback(),
            });
        has_authority && (secure || cfg!(debug_assertions) && loopback)
    });
    if value.len() > 2048 || value.chars().any(char::is_control) || !trusted {
        return Err(format!(
            "{label} must be an HTTPS URL without credentials or fragments (debug builds also permit HTTP loopback)"
        ));
    }
    Ok(())
}

fn main_window_id() -> String {
    MAIN_WINDOW_ID.to_owned()
}

const fn default_true() -> bool {
    true
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
        assert_eq!(EffectKind::SetMenuItemEnabled as u16, 5);
        assert_eq!(EffectKind::SetMenuItemChecked as u16, 6);
        assert_eq!(EffectKind::SetTrayVisible as u16, 7);
        assert_eq!(ApplicationCategory::Development as u8, 1);
        assert_eq!(ApplicationCategory::Productivity as u8, 2);
        assert_eq!(ApplicationCategory::Graphics as u8, 3);
        assert_eq!(ApplicationCategory::AudioVideo as u8, 4);
        assert_eq!(ApplicationCategory::Network as u8, 5);
        assert_eq!(ApplicationCategory::Utility as u8, 6);
        assert_eq!(ApplicationCategory::Game as u8, 7);
        assert_eq!(ApplicationCategory::Education as u8, 8);
        assert_eq!(UpdatePolicy::Manual as u8, 1);
        assert_eq!(UpdatePolicy::Notify as u8, 2);
        assert_eq!(UpdatePolicy::Automatic as u8, 3);
        assert_eq!(UpdatePlatform::Linux as u8, 1);
        assert_eq!(UpdatePlatform::Windows as u8, 2);
        assert_eq!(UpdatePlatform::MacOs as u8, 3);
        assert_eq!(UpdateArtifactKind::PortableArchive as u8, 1);
        assert_eq!(UpdateArtifactKind::NativeInstaller as u8, 2);
        assert_eq!(UpdateState::Disabled as u8, 1);
        assert_eq!(UpdateState::Idle as u8, 2);
        assert_eq!(UpdateState::Checking as u8, 3);
        assert_eq!(UpdateState::Available as u8, 4);
        assert_eq!(UpdateState::Downloading as u8, 5);
        assert_eq!(UpdateState::Ready as u8, 6);
        assert_eq!(UpdateState::Applying as u8, 7);
        assert_eq!(UpdateState::UpToDate as u8, 8);
        assert_eq!(UpdateState::Failed as u8, 9);
        assert_eq!(MenuItemKind::Command as u8, 1);
        assert_eq!(MenuItemKind::Checkbox as u8, 2);
        assert_eq!(MenuItemKind::Separator as u8, 3);
        assert_eq!(MenuItemKind::Submenu as u8, 4);
        assert_eq!(TrayCloseBehavior::Exit as u8, 1);
        assert_eq!(TrayCloseBehavior::Hide as u8, 2);
        assert_eq!(ShortcutState::Pressed as u8, 1);
        assert_eq!(ShortcutState::Released as u8, 2);
        assert_eq!(JobOverlapPolicy::Skip as u8, 1);
        assert_eq!(JobOverlapPolicy::Wait as u8, 2);
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
            manifest: fixture_manifest(),
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
            shell: ShellConfig::default(),
            background_jobs: Vec::new(),
            rust_plugins: Vec::new(),
            php_plugins: Vec::new(),
        };

        assert!(bootstrap.validate().is_ok());
    }

    #[test]
    fn validates_shell_jobs_plugins_and_plugin_transport() {
        let shell = ShellConfig {
            menus: vec![MenuConfig {
                id: "tray".to_owned(),
                label: "Application".to_owned(),
                items: vec![
                    MenuItemConfig {
                        kind: MenuItemKind::Command,
                        id: "show".to_owned(),
                        label: "Show".to_owned(),
                        enabled: true,
                        checked: false,
                        accelerator: Some("CmdOrCtrl+Shift+KeyP".to_owned()),
                        items: Vec::new(),
                    },
                    MenuItemConfig {
                        kind: MenuItemKind::Separator,
                        id: String::new(),
                        label: String::new(),
                        enabled: true,
                        checked: false,
                        accelerator: None,
                        items: Vec::new(),
                    },
                ],
            }],
            tray: Some(TrayConfig {
                menu_id: "tray".to_owned(),
                tooltip: "Pam Desktop".to_owned(),
                close_behavior: TrayCloseBehavior::Hide,
            }),
            shortcuts: vec![GlobalShortcutConfig {
                id: "show".to_owned(),
                accelerator: "CmdOrCtrl+Shift+KeyP".to_owned(),
            }],
        };
        assert!(shell.validate().is_ok());
        let mut menu_without_tray = shell.clone();
        menu_without_tray.tray = None;
        assert!(menu_without_tray.validate().is_err());
        let mut multiple_menus = shell.clone();
        multiple_menus.menus.push(MenuConfig {
            id: "other".to_owned(),
            label: "Other".to_owned(),
            items: vec![MenuItemConfig {
                kind: MenuItemKind::Command,
                id: "other.command".to_owned(),
                label: "Other".to_owned(),
                enabled: true,
                checked: false,
                accelerator: None,
                items: Vec::new(),
            }],
        });
        assert!(multiple_menus.validate().is_err());

        let job = BackgroundJobConfig {
            id: "heartbeat".to_owned(),
            interval_ms: 60_000,
            initial_delay_ms: 1_000,
            timeout_ms: 5_000,
            overlap: JobOverlapPolicy::Skip,
        };
        assert!(job.validate().is_ok());

        let plugin = RustPluginConfig {
            id: "system".to_owned(),
            executable: "plugins/bin/system".to_owned(),
            arguments: vec!["--quiet".to_owned()],
            timeout_ms: 5_000,
        };
        assert!(plugin.validate().is_ok());

        let request = PluginRequestEnvelope::new(9, "system.info", Value::Null);
        assert!(request.validate().is_ok());
        let response = PluginResponseEnvelope::success(
            9,
            serde_json::json!({"platform": "linux"}),
            Vec::new(),
        );
        assert!(response.validate_for(9).is_ok());
        assert!(response.validate_for(8).is_err());
    }

    #[test]
    fn preserves_the_stable_protocol_six_golden_contracts() {
        let bootstrap_json: Value =
            serde_json::from_str(include_str!("../../../compat/protocol-v6/bootstrap.json"))
                .expect("bootstrap golden JSON should parse");
        let bootstrap: Bootstrap = serde_json::from_value(bootstrap_json.clone())
            .expect("bootstrap golden contract should deserialize");
        bootstrap
            .validate()
            .expect("bootstrap golden contract should remain valid");
        assert_eq!(
            serde_json::to_value(bootstrap).expect("bootstrap should serialize"),
            bootstrap_json
        );

        let request_json: Value =
            serde_json::from_str(include_str!("../../../compat/protocol-v6/request.json"))
                .expect("request golden JSON should parse");
        let request: RequestEnvelope = serde_json::from_value(request_json.clone())
            .expect("request golden contract should deserialize");
        assert_eq!(request.version, PROTOCOL_VERSION);
        assert_eq!(
            serde_json::to_value(request).expect("request should serialize"),
            request_json
        );

        let response_json: Value =
            serde_json::from_str(include_str!("../../../compat/protocol-v6/response.json"))
                .expect("response golden JSON should parse");
        let response: ResponseEnvelope = serde_json::from_value(response_json.clone())
            .expect("response golden contract should deserialize");
        response
            .validate_for(41)
            .expect("response golden contract should remain valid");
        assert_eq!(
            serde_json::to_value(response).expect("response should serialize"),
            response_json
        );

        let plugin_request_json: Value = serde_json::from_str(include_str!(
            "../../../compat/protocol-v6/plugin-request.json"
        ))
        .expect("plugin request golden JSON should parse");
        let plugin_request: PluginRequestEnvelope =
            serde_json::from_value(plugin_request_json.clone())
                .expect("plugin request golden contract should deserialize");
        plugin_request
            .validate()
            .expect("plugin request golden contract should remain valid");
        assert_eq!(
            serde_json::to_value(plugin_request).expect("plugin request should serialize"),
            plugin_request_json
        );

        let plugin_response_json: Value = serde_json::from_str(include_str!(
            "../../../compat/protocol-v6/plugin-response.json"
        ))
        .expect("plugin response golden JSON should parse");
        let plugin_response: PluginResponseEnvelope =
            serde_json::from_value(plugin_response_json.clone())
                .expect("plugin response golden contract should deserialize");
        plugin_response
            .validate_for(9)
            .expect("plugin response golden contract should remain valid");
        assert_eq!(
            serde_json::to_value(plugin_response).expect("plugin response should serialize"),
            plugin_response_json
        );
    }

    #[test]
    fn rejects_an_unsafe_application_manifest() {
        let mut manifest = fixture_manifest();
        manifest.identifier = "com.Pushin.Pam".to_owned();
        assert!(manifest.validate().is_err());

        manifest = fixture_manifest();
        manifest.icon = "../icon.svg".to_owned();
        assert!(manifest.validate().is_err());

        manifest = fixture_manifest();
        manifest.bundle_excludes = vec!["vendor".to_owned()];
        assert!(manifest.validate().is_err());
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

    #[test]
    fn validates_signed_update_contracts() {
        let feed = SignedUpdateRelease {
            release: UpdateRelease {
                schema_version: UPDATE_FEED_VERSION,
                application_id: "com.pushin.pam".to_owned(),
                channel: "stable".to_owned(),
                version: "0.5.1".to_owned(),
                published_at: "2026-07-23T14:00:00Z".to_owned(),
                notes_url: Some("https://pam.pushin.dev/releases/0.5.1".to_owned()),
                artifacts: vec![UpdateArtifact {
                    platform: UpdatePlatform::Linux,
                    architecture: "x86_64".to_owned(),
                    kind: UpdateArtifactKind::PortableArchive,
                    url: "https://pam.pushin.dev/releases/pam.tar.gz".to_owned(),
                    bytes: 42,
                    sha256: "a".repeat(64),
                }],
            },
            signature: "b".repeat(128),
        };

        assert!(feed.validate().is_ok());
    }

    #[test]
    fn rejects_insecure_or_ambiguous_update_contracts() {
        let mut manifest = fixture_manifest();
        manifest.updates = Some(UpdateConfig {
            endpoint: "http://updates.example.com/latest.json".to_owned(),
            channel: "stable".to_owned(),
            public_key: "a".repeat(64),
            policy: UpdatePolicy::Automatic,
        });
        assert!(manifest.validate().is_err());

        let mut release = UpdateRelease {
            schema_version: UPDATE_FEED_VERSION,
            application_id: "com.pushin.pam".to_owned(),
            channel: "stable".to_owned(),
            version: "0.5.1".to_owned(),
            published_at: "2026-07-23T14:00:00Z".to_owned(),
            notes_url: None,
            artifacts: vec![UpdateArtifact {
                platform: UpdatePlatform::Linux,
                architecture: "x86_64".to_owned(),
                kind: UpdateArtifactKind::PortableArchive,
                url: "https://pam.pushin.dev/releases/pam.tar.gz".to_owned(),
                bytes: 42,
                sha256: "a".repeat(64),
            }],
        };
        release.artifacts.push(release.artifacts[0].clone());
        assert!(release.validate().is_err());
    }

    fn fixture_manifest() -> ApplicationManifest {
        ApplicationManifest {
            identifier: "com.pushin.pam".to_owned(),
            name: "Pam".to_owned(),
            version: "0.4.0".to_owned(),
            description: "A typed PHP desktop application.".to_owned(),
            publisher: "Pushin".to_owned(),
            category: ApplicationCategory::Development,
            icon: "resources/icon.svg".to_owned(),
            bundle_excludes: vec!["storage/cache".to_owned()],
            updates: None,
        }
    }
}
