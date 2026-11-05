use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use arboard::{Clipboard, ImageData};
use base64::Engine as _;
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use clipboard_rs::{Clipboard as ClipboardCustom, ClipboardContext};
use getrandom::fill;
use notify_rust::Notification;
#[cfg(target_os = "linux")]
use notify_rust::Urgency;
use pam_desktop_protocol::{
    ClipboardOperation, DialogKind, ErrorCode, FileAccess, FileEntryKind, FileOperation,
    NativeCapabilities, NotificationUrgency,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::oneshot;

use crate::database::{DatabaseRequest, DatabaseServices};
use crate::event_hub::EventHub;
use crate::http_client::{HttpRequest, HttpServices};
use crate::process_runner::{ProcessRequest, ProcessServices};
use crate::search::{SearchRequest, SearchServices};

const MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_CLIPBOARD_BYTES: usize = 1024 * 1024;
const MAX_CLIPBOARD_FILES: usize = 256;
const MAX_CLIPBOARD_FORMAT_BYTES: usize = 128;
const MAX_NOTIFICATION_TITLE_BYTES: usize = 256;
const MAX_NOTIFICATION_BODY_BYTES: usize = 4 * 1024;
const MAX_DIALOG_FILTERS: usize = 16;
const MAX_DIALOG_EXTENSIONS: usize = 32;
pub const MAX_STREAM_BYTES: u64 = 4 * 1024 * 1024 * 1024;

fn ensure_clipboard_size(bytes: usize) -> Result<(), NativeError> {
    if bytes > MAX_CLIPBOARD_BYTES {
        return Err(NativeError {
            code: ErrorCode::ResourceTooLarge,
            message: format!("Clipboard content is limited to {MAX_CLIPBOARD_BYTES} bytes."),
        });
    }
    Ok(())
}

fn decode_clipboard_image(image: &ClipboardImage) -> Result<Vec<u8>, NativeError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&image.rgba_base64)
        .map_err(|_| NativeError::invalid("Clipboard image must be valid base64."))?;
    ensure_clipboard_size(bytes.len())?;
    let expected = image
        .width
        .checked_mul(image.height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| NativeError::invalid("Clipboard image dimensions overflow."))?;
    if expected != bytes.len() {
        return Err(NativeError::invalid(
            "Clipboard image must contain width × height RGBA pixels.",
        ));
    }
    Ok(bytes)
}

fn validate_custom_clipboard_format(format: Option<&str>) -> Result<&str, NativeError> {
    let format = format.ok_or_else(|| NativeError::invalid("Clipboard format is required."))?;
    if format.is_empty()
        || format.len() > MAX_CLIPBOARD_FORMAT_BYTES
        || !format.is_ascii()
        || format.bytes().any(|byte| byte.is_ascii_control())
        || !(format.starts_with("application/x-") || format.starts_with("application/vnd."))
    {
        return Err(NativeError::invalid(
            "Custom clipboard formats must be printable application/x-* or application/vnd.* identifiers of at most 128 bytes.",
        ));
    }
    Ok(format)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileTarget {
    pub root: Option<String>,
    pub grant_id: Option<String>,
    #[serde(default)]
    pub path: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRequest {
    pub operation: FileOperation,
    pub window_id: String,
    pub target: FileTarget,
    pub content: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardRequest {
    pub operation: ClipboardOperation,
    pub window_id: String,
    pub text: Option<String>,
    pub html: Option<String>,
    pub image: Option<ClipboardImage>,
    #[serde(default)]
    pub files: Vec<FileTarget>,
    pub format: Option<String>,
    pub data_base64: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClipboardImage {
    pub width: usize,
    pub height: usize,
    pub rgba_base64: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationRequest {
    pub window_id: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub urgency: NotificationUrgency,
    #[serde(default)]
    pub actions: Vec<NotificationAction>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationAction {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogBridgeRequest {
    pub kind: DialogKind,
    pub window_id: String,
    pub title: Option<String>,
    pub file_name: Option<String>,
    #[serde(default)]
    pub filters: Vec<DialogFilter>,
    pub access: Option<FileAccess>,
    #[serde(default)]
    pub persistent: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogFilter {
    pub name: String,
    pub extensions: Vec<String>,
}

#[derive(Debug)]
pub struct DialogRequest {
    pub kind: DialogKind,
    pub title: Option<String>,
    pub file_name: Option<String>,
    pub filters: Vec<DialogFilter>,
    pub reply: oneshot::Sender<Result<Vec<PathBuf>, String>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileReference {
    pub grant_id: String,
    pub name: String,
    pub kind: FileEntryKind,
    pub access: FileAccess,
}

#[derive(Clone, Debug)]
pub struct NativeError {
    pub code: ErrorCode,
    pub message: String,
}

impl NativeError {
    pub(crate) fn disabled(capability: impl std::fmt::Display) -> Self {
        Self {
            code: ErrorCode::CapabilityDisabled,
            message: format!("The {capability} capability is not enabled by the PHP application."),
        }
    }

    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::InvalidPayload,
            message: message.into(),
        }
    }

    pub(crate) fn permission(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::PermissionDenied,
            message: message.into(),
        }
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::ResourceNotFound,
            message: message.into(),
        }
    }

    fn invalid_grant() -> Self {
        Self {
            code: ErrorCode::InvalidGrant,
            message: "The file grant is unknown or has expired.".to_owned(),
        }
    }

    fn io(context: &str, error: &std::io::Error) -> Self {
        let code = match error.kind() {
            ErrorKind::NotFound => ErrorCode::ResourceNotFound,
            ErrorKind::PermissionDenied => ErrorCode::PermissionDenied,
            _ => ErrorCode::NativeOperationFailed,
        };
        Self {
            code,
            message: format!("{context}: {error}"),
        }
    }

    pub(crate) fn native(context: &str, error: impl std::fmt::Display) -> Self {
        Self {
            code: ErrorCode::NativeOperationFailed,
            message: format!("{context}: {error}"),
        }
    }

    pub(crate) fn too_large(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::ResourceTooLarge,
            message: message.into(),
        }
    }
}

pub struct NativeServices {
    capabilities: NativeCapabilities,
    roots: HashMap<String, AuthorizedRoot>,
    grants: Mutex<HashMap<String, FileGrant>>,
    persistent_grants: Mutex<HashMap<String, PersistentGrant>>,
    persistent_grants_path: PathBuf,
    clipboard: Mutex<Option<Clipboard>>,
    custom_clipboard: Mutex<Option<ClipboardContext>>,
    clipboard_gate: Mutex<()>,
    databases: DatabaseServices,
    http: HttpServices,
    processes: ProcessServices,
    search: SearchServices,
}

impl NativeServices {
    pub fn prepare(
        project_root: &Path,
        application_id: &str,
        capabilities: &NativeCapabilities,
    ) -> Result<Self, String> {
        capabilities.validate()?;
        let mut roots = HashMap::with_capacity(capabilities.filesystem_roots.len());
        for config in &capabilities.filesystem_roots {
            let configured = PathBuf::from(&config.path);
            let candidate = if configured.is_absolute() {
                configured
            } else {
                project_root.join(configured)
            };
            let canonical = std::fs::canonicalize(&candidate).map_err(|error| {
                format!(
                    "cannot authorize filesystem root {:?} at {}: {error}",
                    config.name,
                    candidate.display()
                )
            })?;
            if !canonical.is_dir() {
                return Err(format!(
                    "filesystem root {:?} is not a directory: {}",
                    config.name,
                    canonical.display()
                ));
            }
            let directory =
                Dir::open_ambient_dir(&canonical, ambient_authority()).map_err(|error| {
                    format!(
                        "cannot open filesystem root {:?} at {}: {error}",
                        config.name,
                        canonical.display()
                    )
                })?;
            roots.insert(
                config.name.clone(),
                AuthorizedRoot {
                    directory: Arc::new(directory),
                    access: config.access,
                    path: canonical,
                },
            );
        }

        let persistent_grants_path = persistent_grants_path(application_id)?;
        let persistent_grants = load_persistent_grants(&persistent_grants_path)?;
        let grants = persistent_grants
            .iter()
            .filter_map(|(id, grant)| {
                create_file_grant(&grant.path, grant.access)
                    .ok()
                    .map(|file_grant| (id.clone(), file_grant))
            })
            .collect();
        Ok(Self {
            capabilities: capabilities.clone(),
            roots,
            grants: Mutex::new(grants),
            persistent_grants: Mutex::new(persistent_grants),
            persistent_grants_path,
            clipboard: Mutex::new(None),
            custom_clipboard: Mutex::new(None),
            clipboard_gate: Mutex::new(()),
            databases: DatabaseServices::prepare(project_root, &capabilities.databases)?,
            http: HttpServices::prepare(&capabilities.http_origins)?,
            processes: ProcessServices::prepare(project_root, &capabilities.processes)?,
            search: SearchServices::prepare(project_root)?,
        })
    }

    #[must_use]
    pub fn secrets_enabled(&self) -> bool {
        self.capabilities.secrets
    }

    #[must_use]
    pub fn desktop_portal_enabled(&self) -> bool {
        self.capabilities.desktop_portal
    }

    pub fn terminal_session_count(&self) -> usize {
        self.processes.terminal_session_count()
    }

    #[must_use]
    pub fn drag_and_drop_enabled(&self) -> bool {
        self.capabilities.drag_and_drop
    }

    pub fn database(&self, request: &DatabaseRequest) -> Result<Value, NativeError> {
        self.databases.dispatch(request)
    }

    pub fn system_information(&self) -> Result<Value, NativeError> {
        if !self.capabilities.system_information {
            return Err(NativeError::disabled("system information"));
        }
        serde_json::to_value(crate::system_info::snapshot())
            .map_err(|error| NativeError::native("Cannot encode system information", error))
    }

    pub fn http(&self, request: &HttpRequest) -> Result<Value, NativeError> {
        self.http.dispatch(request)
    }

    pub fn process(&self, request: &ProcessRequest) -> Result<Value, NativeError> {
        self.processes.dispatch(request)
    }

    pub fn search(&self, request: &SearchRequest) -> Result<Value, NativeError> {
        let path = request
            .target
            .as_ref()
            .map(|target| self.watch_path(target))
            .transpose()?;
        self.search.dispatch(request, path.as_deref())
    }

    pub fn watch_path(&self, target: &FileTarget) -> Result<PathBuf, NativeError> {
        let Some(root_name) = &target.root else {
            return Err(NativeError::invalid(
                "File watches require a named filesystem root.",
            ));
        };
        if target.grant_id.is_some() {
            return Err(NativeError::invalid(
                "File watches cannot combine root and grantId.",
            ));
        }
        validate_relative_path(&target.path)?;
        let root = self
            .roots
            .get(root_name)
            .ok_or_else(|| NativeError::permission("The filesystem root is not authorized."))?;
        if !root.access.can_read() {
            return Err(NativeError::permission("File watches require read access."));
        }
        let candidate = root.path.join(&target.path);
        let canonical = candidate
            .canonicalize()
            .map_err(|error| NativeError::io("Cannot resolve watched path", &error))?;
        if !canonical.starts_with(&root.path) {
            return Err(NativeError::permission(
                "The watched path escapes its authorized root.",
            ));
        }
        let metadata = std::fs::symlink_metadata(&candidate)
            .map_err(|error| NativeError::io("Cannot inspect watched path", &error))?;
        if metadata.file_type().is_symlink() {
            return Err(NativeError::permission("Symbolic links cannot be watched."));
        }
        Ok(canonical)
    }

    pub fn filesystem(&self, request: &FileRequest) -> Result<Value, NativeError> {
        let target = self.resolve(&request.target)?;
        match request.operation {
            FileOperation::ReadText => Self::read_text(&target),
            FileOperation::WriteText => Self::write_text(&target, request.content.as_deref()),
            FileOperation::List => Self::list(&target, &request.target.path),
            FileOperation::Metadata => Self::metadata(&target),
            FileOperation::CreateDirectory => Self::create_directory(&target),
        }
    }

    pub fn open_read_stream(
        &self,
        target: &FileTarget,
    ) -> Result<(std::fs::File, u64), NativeError> {
        let target = self.resolve(target)?;
        target.require_read()?;
        let metadata = target.metadata()?;
        if !metadata.is_file() {
            return Err(NativeError::invalid(
                "Streaming reads require a regular file.",
            ));
        }
        if metadata.len() > MAX_STREAM_BYTES {
            return Err(NativeError::too_large(format!(
                "Streaming files are limited to {MAX_STREAM_BYTES} bytes."
            )));
        }
        let file = target
            .directory
            .open(&target.relative)
            .map_err(|error| NativeError::io("Cannot open the streaming file", &error))?;
        Ok((file.into_std(), metadata.len()))
    }

    pub fn open_write_stream(&self, target: &FileTarget) -> Result<std::fs::File, NativeError> {
        let target = self.resolve(target)?;
        target.require_write()?;
        if target.relative.as_os_str().is_empty() {
            return Err(NativeError::invalid(
                "Streaming writes require a non-empty file path.",
            ));
        }
        target.reject_existing_symlink()?;
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        target
            .directory
            .open_with(&target.relative, &options)
            .map(cap_std::fs::File::into_std)
            .map_err(|error| NativeError::io("Cannot open the streaming destination", &error))
    }

    fn read_text(target: &ResolvedTarget) -> Result<Value, NativeError> {
        target.require_read()?;
        let metadata = target.metadata()?;
        if !metadata.is_file() {
            return Err(NativeError::invalid("The selected resource is not a file."));
        }
        if metadata.len() > MAX_TEXT_BYTES as u64 {
            return Err(text_too_large("files"));
        }
        let mut file = target
            .directory
            .open(&target.relative)
            .map_err(|error| NativeError::io("Cannot open the text file", &error))?;
        let capacity = usize::try_from(metadata.len()).unwrap_or(MAX_TEXT_BYTES);
        let mut bytes = Vec::with_capacity(capacity);
        Read::by_ref(&mut file)
            .take((MAX_TEXT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| NativeError::io("Cannot read the text file", &error))?;
        if bytes.len() > MAX_TEXT_BYTES {
            return Err(text_too_large("files"));
        }
        let text = String::from_utf8(bytes)
            .map_err(|_| NativeError::invalid("The selected file is not valid UTF-8."))?;
        Ok(json!({"text": text}))
    }

    fn write_text(target: &ResolvedTarget, content: Option<&str>) -> Result<Value, NativeError> {
        target.require_write()?;
        let content = content.ok_or_else(|| NativeError::invalid("Text content is required."))?;
        if content.len() > MAX_TEXT_BYTES {
            return Err(text_too_large("writes"));
        }
        target.reject_existing_symlink()?;
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        let mut file = target
            .directory
            .open_with(&target.relative, &options)
            .map_err(|error| NativeError::io("Cannot open the text file for writing", &error))?;
        file.write_all(content.as_bytes())
            .map_err(|error| NativeError::io("Cannot write the text file", &error))?;
        Ok(json!({"bytesWritten": content.len()}))
    }

    fn list(target: &ResolvedTarget, bridge_path: &str) -> Result<Value, NativeError> {
        target.require_read()?;
        let metadata = target.metadata()?;
        if !metadata.is_dir() {
            return Err(NativeError::invalid(
                "Directory listing requires a directory target.",
            ));
        }
        let mut entries = Vec::new();
        let iterator = target
            .directory
            .read_dir(&target.relative)
            .map_err(|error| NativeError::io("Cannot list the directory", &error))?;
        for entry in iterator {
            let entry =
                entry.map_err(|error| NativeError::io("Cannot read a directory entry", &error))?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| NativeError::invalid("Non-UTF-8 filenames are not supported."))?;
            let file_type = entry
                .file_type()
                .map_err(|error| NativeError::io("Cannot inspect a directory entry", &error))?;
            let kind = if file_type.is_file() {
                FileEntryKind::File
            } else if file_type.is_dir() {
                FileEntryKind::Directory
            } else {
                continue;
            };
            let metadata = entry
                .metadata()
                .map_err(|error| NativeError::io("Cannot inspect a directory entry", &error))?;
            entries.push(FileEntry {
                path: join_bridge_path(bridge_path, &name),
                name,
                kind,
                size: metadata.len(),
            });
        }
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        serde_json::to_value(entries)
            .map_err(|error| NativeError::native("Cannot encode the directory listing", error))
    }

    fn metadata(target: &ResolvedTarget) -> Result<Value, NativeError> {
        target.require_read()?;
        let metadata = target.metadata()?;
        let kind = if metadata.is_file() {
            FileEntryKind::File
        } else if metadata.is_dir() {
            FileEntryKind::Directory
        } else {
            return Err(NativeError::invalid(
                "Only files and directories are exposed by the bridge.",
            ));
        };
        Ok(json!({
            "name": target.name(),
            "kind": kind,
            "size": metadata.len(),
        }))
    }

    fn create_directory(target: &ResolvedTarget) -> Result<Value, NativeError> {
        target.require_write()?;
        if target.relative.as_os_str().is_empty() {
            return Err(NativeError::invalid(
                "Creating a directory requires a non-empty relative path.",
            ));
        }
        target
            .directory
            .create_dir_all(&target.relative)
            .map_err(|error| NativeError::io("Cannot create the directory", &error))?;
        Ok(Value::Null)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "all bounded clipboard formats share one lazily owned native clipboard"
    )]
    pub fn clipboard(&self, request: &ClipboardRequest) -> Result<Value, NativeError> {
        match request.operation {
            ClipboardOperation::ReadText
            | ClipboardOperation::ReadImage
            | ClipboardOperation::ReadFiles
            | ClipboardOperation::ReadCustom
            | ClipboardOperation::AvailableFormats
                if !self.capabilities.clipboard_read =>
            {
                return Err(NativeError::disabled("clipboard read"));
            }
            ClipboardOperation::WriteText
            | ClipboardOperation::WriteHtml
            | ClipboardOperation::WriteImage
            | ClipboardOperation::WriteFiles
            | ClipboardOperation::WriteCustom
            | ClipboardOperation::Clear
                if !self.capabilities.clipboard_write =>
            {
                return Err(NativeError::disabled("clipboard write"));
            }
            _ => {}
        }
        let _gate = self.clipboard_gate.lock().map_err(|_| {
            NativeError::native("Cannot serialize clipboard access", "lock is poisoned")
        })?;
        let mut clipboard = self
            .clipboard
            .lock()
            .map_err(|_| NativeError::native("Cannot lock the clipboard", "lock is poisoned"))?;
        if clipboard.is_none() {
            *clipboard =
                Some(Clipboard::new().map_err(|error| {
                    NativeError::native("Cannot initialize the clipboard", error)
                })?);
        }
        let clipboard = clipboard
            .as_mut()
            .expect("the clipboard is initialized immediately above");
        match request.operation {
            ClipboardOperation::ReadText => {
                let text = clipboard
                    .get_text()
                    .map_err(|error| NativeError::native("Cannot read clipboard text", error))?;
                if text.len() > MAX_CLIPBOARD_BYTES {
                    return Err(NativeError {
                        code: ErrorCode::ResourceTooLarge,
                        message: format!(
                            "Clipboard text is limited to {MAX_CLIPBOARD_BYTES} bytes."
                        ),
                    });
                }
                Ok(json!({"text": text}))
            }
            ClipboardOperation::WriteText => {
                let text = request
                    .text
                    .as_deref()
                    .ok_or_else(|| NativeError::invalid("Clipboard text is required."))?;
                if text.len() > MAX_CLIPBOARD_BYTES {
                    return Err(NativeError {
                        code: ErrorCode::ResourceTooLarge,
                        message: format!(
                            "Clipboard text is limited to {MAX_CLIPBOARD_BYTES} bytes."
                        ),
                    });
                }
                clipboard
                    .set_text(text)
                    .map_err(|error| NativeError::native("Cannot write clipboard text", error))?;
                Ok(Value::Null)
            }
            ClipboardOperation::WriteHtml => {
                let html = request
                    .html
                    .as_deref()
                    .ok_or_else(|| NativeError::invalid("Clipboard HTML is required."))?;
                ensure_clipboard_size(html.len())?;
                clipboard
                    .set_html(html, request.text.as_deref())
                    .map_err(|error| NativeError::native("Cannot write clipboard HTML", error))?;
                Ok(Value::Null)
            }
            ClipboardOperation::ReadImage => {
                let image = clipboard
                    .get_image()
                    .map_err(|error| NativeError::native("Cannot read clipboard image", error))?;
                ensure_clipboard_size(image.bytes.len())?;
                Ok(json!({
                    "width": image.width,
                    "height": image.height,
                    "rgbaBase64": base64::engine::general_purpose::STANDARD.encode(image.bytes),
                }))
            }
            ClipboardOperation::WriteImage => {
                let image = request
                    .image
                    .as_ref()
                    .ok_or_else(|| NativeError::invalid("Clipboard image is required."))?;
                let bytes = decode_clipboard_image(image)?;
                clipboard
                    .set_image(ImageData {
                        width: image.width,
                        height: image.height,
                        bytes: Cow::Owned(bytes),
                    })
                    .map_err(|error| NativeError::native("Cannot write clipboard image", error))?;
                Ok(Value::Null)
            }
            ClipboardOperation::ReadFiles => {
                let paths = clipboard
                    .get()
                    .file_list()
                    .map_err(|error| NativeError::native("Cannot read clipboard files", error))?;
                if paths.len() > MAX_CLIPBOARD_FILES {
                    return Err(NativeError::too_large(format!(
                        "Clipboard file lists are limited to {MAX_CLIPBOARD_FILES} entries."
                    )));
                }
                let grants = self.grant_paths(paths, FileAccess::Read, false)?;
                serde_json::to_value(grants).map_err(|error| {
                    NativeError::native("Cannot encode clipboard file grants", error)
                })
            }
            ClipboardOperation::WriteFiles => {
                if request.files.is_empty() || request.files.len() > MAX_CLIPBOARD_FILES {
                    return Err(NativeError::invalid(format!(
                        "Clipboard file lists require 1 to {MAX_CLIPBOARD_FILES} entries."
                    )));
                }
                let paths = request
                    .files
                    .iter()
                    .map(|target| self.clipboard_file_path(target))
                    .collect::<Result<Vec<_>, _>>()?;
                clipboard
                    .set()
                    .file_list(&paths)
                    .map_err(|error| NativeError::native("Cannot write clipboard files", error))?;
                Ok(Value::Null)
            }
            ClipboardOperation::ReadCustom => {
                let format = validate_custom_clipboard_format(request.format.as_deref())?;
                let context = self.custom_clipboard()?;
                let bytes = context
                    .as_ref()
                    .expect("custom clipboard is initialized immediately above")
                    .get_buffer(format)
                    .map_err(|error| {
                        NativeError::native("Cannot read custom clipboard data", error)
                    })?;
                ensure_clipboard_size(bytes.len())?;
                Ok(json!({
                    "format": format,
                    "dataBase64": base64::engine::general_purpose::STANDARD.encode(bytes),
                }))
            }
            ClipboardOperation::WriteCustom => {
                let format = validate_custom_clipboard_format(request.format.as_deref())?;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(request.data_base64.as_deref().ok_or_else(|| {
                        NativeError::invalid("Custom clipboard data is required.")
                    })?)
                    .map_err(|_| {
                        NativeError::invalid("Custom clipboard data must be valid base64.")
                    })?;
                ensure_clipboard_size(bytes.len())?;
                self.custom_clipboard()?
                    .as_ref()
                    .expect("custom clipboard is initialized immediately above")
                    .set_buffer(format, bytes)
                    .map_err(|error| {
                        NativeError::native("Cannot write custom clipboard data", error)
                    })?;
                Ok(Value::Null)
            }
            ClipboardOperation::AvailableFormats => {
                let formats = self
                    .custom_clipboard()?
                    .as_ref()
                    .expect("custom clipboard is initialized immediately above")
                    .available_formats()
                    .map_err(|error| NativeError::native("Cannot list clipboard formats", error))?;
                if formats.len() > 256 {
                    return Err(NativeError::too_large(
                        "The system clipboard exposed more than 256 formats.",
                    ));
                }
                Ok(json!({"formats": formats}))
            }
            ClipboardOperation::Clear => {
                clipboard
                    .clear()
                    .map_err(|error| NativeError::native("Cannot clear the clipboard", error))?;
                Ok(Value::Null)
            }
        }
    }

    fn custom_clipboard(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, Option<ClipboardContext>>, NativeError> {
        let mut context = self
            .custom_clipboard
            .lock()
            .map_err(|_| NativeError::native("Cannot lock custom clipboard", "lock is poisoned"))?;
        if context.is_none() {
            *context = Some(ClipboardContext::new().map_err(|error| {
                NativeError::native("Cannot initialize custom clipboard", error)
            })?);
        }
        Ok(context)
    }

    fn clipboard_file_path(&self, target: &FileTarget) -> Result<PathBuf, NativeError> {
        let resolved = self.resolve(target)?;
        resolved.require_read()?;
        let metadata = resolved.metadata()?;
        if !metadata.is_file() && !metadata.is_dir() {
            return Err(NativeError::invalid(
                "Clipboard file entries must be regular files or directories.",
            ));
        }
        let path = resolved.absolute_path.ok_or_else(|| {
            NativeError::permission("Clipboard files require a canonical local path.")
        })?;
        let canonical = path
            .canonicalize()
            .map_err(|error| NativeError::io("Cannot resolve clipboard file", &error))?;
        if canonical != path {
            return Err(NativeError::permission(
                "Clipboard file paths cannot traverse symbolic links.",
            ));
        }
        Ok(path)
    }

    pub fn notify(
        &self,
        request: &NotificationRequest,
        events: &EventHub,
    ) -> Result<Value, NativeError> {
        if !self.capabilities.notifications {
            return Err(NativeError::disabled("notifications"));
        }
        if request.title.trim().is_empty() || request.title.len() > MAX_NOTIFICATION_TITLE_BYTES {
            return Err(NativeError::invalid(format!(
                "Notification titles must contain 1 to {MAX_NOTIFICATION_TITLE_BYTES} bytes."
            )));
        }
        if request.body.len() > MAX_NOTIFICATION_BODY_BYTES {
            return Err(NativeError::invalid(format!(
                "Notification bodies are limited to {MAX_NOTIFICATION_BODY_BYTES} bytes."
            )));
        }
        if request.actions.len() > 4 {
            return Err(NativeError::invalid(
                "Notifications accept at most four actions.",
            ));
        }
        let mut action_ids = std::collections::HashSet::new();
        for action in &request.actions {
            if action.id.is_empty()
                || action.id.len() > 64
                || !action
                    .id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
            {
                return Err(NativeError::invalid(
                    "Notification action IDs must contain 1 to 64 ASCII identifier bytes.",
                ));
            }
            if action.label.trim().is_empty() || action.label.len() > 80 {
                return Err(NativeError::invalid(
                    "Notification action labels must contain 1 to 80 bytes.",
                ));
            }
            if !action_ids.insert(action.id.as_str()) {
                return Err(NativeError::invalid(
                    "Notification action IDs must be unique.",
                ));
            }
        }
        let mut notification = Notification::new();
        notification
            .appname("Pam Desktop")
            .summary(&request.title)
            .body(&request.body);
        #[cfg(target_os = "linux")]
        {
            notification.urgency(match request.urgency {
                NotificationUrgency::Low => Urgency::Low,
                NotificationUrgency::Normal => Urgency::Normal,
                NotificationUrgency::Critical => Urgency::Critical,
            });
            for action in &request.actions {
                notification.action(&action.id, &action.label);
            }
        }
        #[cfg(target_os = "linux")]
        let handle = notification
            .show()
            .map_err(|error| NativeError::native("Cannot show the notification", error))?;
        #[cfg(not(target_os = "linux"))]
        notification
            .show()
            .map_err(|error| NativeError::native("Cannot show the notification", error))?;
        #[cfg(target_os = "linux")]
        {
            let id = handle.id();
            if !request.actions.is_empty() {
                let window_id = request.window_id.clone();
                let events = events.clone();
                std::thread::Builder::new()
                    .name(format!("pam-notification-{id}"))
                    .spawn(move || {
                        handle.wait_for_action(|action| {
                            events.publish(pam_desktop_protocol::ClientEvent {
                                name: "pam.notification.action".to_owned(),
                                payload: json!({"notificationId": id, "action": action}),
                                window_id: Some(window_id.clone()),
                            });
                        });
                    })
                    .map_err(|error| {
                        NativeError::native("Cannot start notification action listener", error)
                    })?;
            }
            Ok(json!({"id": id}))
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = events;
            Ok(Value::Null)
        }
    }

    pub fn validate_dialog(
        &self,
        request: &DialogBridgeRequest,
    ) -> Result<FileAccess, NativeError> {
        if !self.capabilities.dialogs {
            return Err(NativeError::disabled("dialogs"));
        }
        if request
            .title
            .as_ref()
            .is_some_and(|title| title.len() > 256)
        {
            return Err(NativeError::invalid(
                "Dialog titles are limited to 256 bytes.",
            ));
        }
        if let Some(file_name) = &request.file_name
            && (file_name.is_empty()
                || file_name.len() > 255
                || Path::new(file_name).components().count() != 1
                || !matches!(
                    Path::new(file_name).components().next(),
                    Some(Component::Normal(_))
                ))
        {
            return Err(NativeError::invalid(
                "The suggested filename must be one plain filename.",
            ));
        }
        if request.filters.len() > MAX_DIALOG_FILTERS {
            return Err(NativeError::invalid(format!(
                "Dialogs support at most {MAX_DIALOG_FILTERS} filters."
            )));
        }
        for filter in &request.filters {
            if filter.name.trim().is_empty()
                || filter.name.len() > 64
                || filter.extensions.is_empty()
                || filter.extensions.len() > MAX_DIALOG_EXTENSIONS
                || filter.extensions.iter().any(|extension| {
                    extension.is_empty()
                        || extension.len() > 16
                        || !extension
                            .chars()
                            .all(|character| character.is_ascii_alphanumeric())
                })
            {
                return Err(NativeError::invalid(
                    "Dialog filters require a short name and alphanumeric extensions.",
                ));
            }
        }

        let access = match request.kind {
            DialogKind::OpenFile | DialogKind::OpenFiles => FileAccess::Read,
            DialogKind::SaveFile => FileAccess::ReadWrite,
            DialogKind::OpenDirectory => request.access.unwrap_or(FileAccess::Read),
        };
        Ok(access)
    }

    pub fn grant_paths(
        &self,
        paths: Vec<PathBuf>,
        access: FileAccess,
        persistent: bool,
    ) -> Result<Vec<FileReference>, NativeError> {
        paths
            .into_iter()
            .map(|path| self.grant_path_with_persistence(&path, access, persistent))
            .collect()
    }

    pub fn grant_path(
        &self,
        selected: &Path,
        access: FileAccess,
    ) -> Result<FileReference, NativeError> {
        self.grant_path_with_persistence(selected, access, false)
    }

    fn grant_path_with_persistence(
        &self,
        selected: &Path,
        access: FileAccess,
        persistent: bool,
    ) -> Result<FileReference, NativeError> {
        let file_grant = create_file_grant(selected, access)?;
        let grant_id = secure_grant_id()?;
        let name = file_grant.name.clone();
        let kind = file_grant.kind;
        self.grants
            .lock()
            .map_err(|_| NativeError::native("Cannot store the file grant", "lock is poisoned"))?
            .insert(grant_id.clone(), file_grant);
        if persistent {
            let canonical = canonical_grant_target(selected, access)?;
            let mut grants = self.persistent_grants.lock().map_err(|_| {
                NativeError::native("Cannot store the persistent file grant", "lock is poisoned")
            })?;
            grants.insert(
                grant_id.clone(),
                PersistentGrant {
                    path: canonical,
                    access,
                },
            );
            if let Err(error) = persist_grants(&self.persistent_grants_path, &grants) {
                grants.remove(&grant_id);
                self.grants
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&grant_id);
                return Err(error);
            }
        }
        Ok(FileReference {
            grant_id,
            name,
            kind,
            access,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistentGrant {
    path: PathBuf,
    access: FileAccess,
}

fn create_file_grant(selected: &Path, access: FileAccess) -> Result<FileGrant, NativeError> {
    let (directory_path, relative, kind, name) = match std::fs::symlink_metadata(selected) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(NativeError::permission(
                    "Symbolic links cannot become external file grants.",
                ));
            }
            let canonical = std::fs::canonicalize(selected)
                .map_err(|error| NativeError::io("Cannot resolve the selected path", &error))?;
            if metadata.is_dir() {
                let name = canonical.file_name().map_or_else(
                    || canonical.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                );
                (canonical, PathBuf::new(), FileEntryKind::Directory, name)
            } else if metadata.is_file() {
                let parent = canonical.parent().ok_or_else(|| {
                    NativeError::invalid("The selected file has no parent directory.")
                })?;
                let file_name = canonical
                    .file_name()
                    .ok_or_else(|| NativeError::invalid("The selected file has no filename."))?;
                (
                    parent.to_path_buf(),
                    PathBuf::from(file_name),
                    FileEntryKind::File,
                    file_name.to_string_lossy().into_owned(),
                )
            } else {
                return Err(NativeError::invalid(
                    "Only regular files and directories can become grants.",
                ));
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound && access.can_write() => {
            let parent = selected.parent().ok_or_else(|| {
                NativeError::invalid("The selected save path has no parent directory.")
            })?;
            let parent = std::fs::canonicalize(parent)
                .map_err(|error| NativeError::io("Cannot resolve the save directory", &error))?;
            let file_name = selected
                .file_name()
                .ok_or_else(|| NativeError::invalid("The selected save path has no filename."))?;
            (
                parent,
                PathBuf::from(file_name),
                FileEntryKind::File,
                file_name.to_string_lossy().into_owned(),
            )
        }
        Err(error) => return Err(NativeError::io("Cannot inspect the selected path", &error)),
    };

    let path = directory_path.join(&relative);
    let directory = Dir::open_ambient_dir(&directory_path, ambient_authority())
        .map_err(|error| NativeError::io("Cannot open the granted directory", &error))?;
    Ok(FileGrant {
        directory: Arc::new(directory),
        relative,
        kind,
        access,
        name,
        path,
    })
}

impl NativeServices {
    fn resolve(&self, target: &FileTarget) -> Result<ResolvedTarget, NativeError> {
        validate_relative_path(&target.path)?;
        match (&target.root, &target.grant_id) {
            (Some(root), None) => {
                let authorized = self.roots.get(root).ok_or_else(|| {
                    NativeError::permission(format!(
                        "Filesystem root {root:?} is not authorized by the PHP application."
                    ))
                })?;
                Ok(ResolvedTarget {
                    directory: authorized.directory.clone(),
                    relative: PathBuf::from(&target.path),
                    access: authorized.access,
                    display_name: Path::new(&target.path)
                        .file_name()
                        .map_or_else(|| root.clone(), |name| name.to_string_lossy().into_owned()),
                    absolute_path: Some(authorized.path.join(&target.path)),
                })
            }
            (None, Some(grant_id)) => {
                let grant = self
                    .grants
                    .lock()
                    .map_err(|_| {
                        NativeError::native("Cannot read the file grant", "lock is poisoned")
                    })?
                    .get(grant_id)
                    .cloned()
                    .ok_or_else(NativeError::invalid_grant)?;
                let relative = if grant.kind == FileEntryKind::File {
                    if !target.path.is_empty() {
                        return Err(NativeError::invalid(
                            "A file grant cannot address a child path.",
                        ));
                    }
                    grant.relative
                } else {
                    grant.relative.join(&target.path)
                };
                let absolute_path = if grant.kind == FileEntryKind::File {
                    grant.path.clone()
                } else {
                    grant.path.join(&target.path)
                };
                Ok(ResolvedTarget {
                    directory: grant.directory,
                    relative,
                    access: grant.access,
                    display_name: if target.path.is_empty() {
                        grant.name
                    } else {
                        Path::new(&target.path)
                            .file_name()
                            .map_or_else(|| grant.name, |name| name.to_string_lossy().into_owned())
                    },
                    absolute_path: Some(absolute_path),
                })
            }
            _ => Err(NativeError::invalid(
                "A file target must contain exactly one root or grantId.",
            )),
        }
    }
}

#[derive(Clone)]
struct AuthorizedRoot {
    directory: Arc<Dir>,
    access: FileAccess,
    path: PathBuf,
}

#[derive(Clone)]
struct FileGrant {
    directory: Arc<Dir>,
    relative: PathBuf,
    kind: FileEntryKind,
    access: FileAccess,
    name: String,
    path: PathBuf,
}

struct ResolvedTarget {
    directory: Arc<Dir>,
    relative: PathBuf,
    access: FileAccess,
    display_name: String,
    absolute_path: Option<PathBuf>,
}

impl ResolvedTarget {
    fn require_read(&self) -> Result<(), NativeError> {
        if self.access.can_read() {
            Ok(())
        } else {
            Err(NativeError::permission(
                "This filesystem capability does not allow reads.",
            ))
        }
    }

    fn require_write(&self) -> Result<(), NativeError> {
        if self.access.can_write() {
            Ok(())
        } else {
            Err(NativeError::permission(
                "This filesystem capability does not allow writes.",
            ))
        }
    }

    fn metadata(&self) -> Result<cap_std::fs::Metadata, NativeError> {
        let metadata = if self.relative.as_os_str().is_empty() {
            self.directory.dir_metadata()
        } else {
            self.directory.symlink_metadata(&self.relative)
        }
        .map_err(|error| NativeError::io("Cannot inspect the filesystem resource", &error))?;
        if metadata.file_type().is_symlink() {
            return Err(NativeError::permission(
                "Symbolic links are not exposed through filesystem capabilities.",
            ));
        }
        Ok(metadata)
    }

    fn reject_existing_symlink(&self) -> Result<(), NativeError> {
        match self.directory.symlink_metadata(&self.relative) {
            Ok(metadata) if metadata.file_type().is_symlink() => Err(NativeError::permission(
                "Symbolic links cannot be written through filesystem capabilities.",
            )),
            Ok(metadata) if metadata.is_dir() => Err(NativeError::invalid(
                "A directory cannot be overwritten with text.",
            )),
            Ok(_) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(NativeError::io(
                "Cannot inspect the filesystem resource",
                &error,
            )),
        }
    }

    fn name(&self) -> &str {
        &self.display_name
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileEntry {
    name: String,
    path: String,
    kind: FileEntryKind,
    size: u64,
}

pub fn show_dialog(request: DialogRequest) {
    let mut dialog = rfd::FileDialog::new();
    if let Some(title) = &request.title {
        dialog = dialog.set_title(title);
    }
    if let Some(file_name) = &request.file_name {
        dialog = dialog.set_file_name(file_name);
    }
    for filter in &request.filters {
        dialog = dialog.add_filter(&filter.name, &filter.extensions);
    }
    let paths = match request.kind {
        DialogKind::OpenFile => dialog.pick_file().into_iter().collect(),
        DialogKind::OpenFiles => dialog.pick_files().unwrap_or_default(),
        DialogKind::SaveFile => dialog.save_file().into_iter().collect(),
        DialogKind::OpenDirectory => dialog.pick_folder().into_iter().collect(),
    };
    let _ = request.reply.send(Ok(paths));
}

fn validate_relative_path(value: &str) -> Result<(), NativeError> {
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err(NativeError::invalid(
            "Filesystem paths must be relative and cannot contain parent components.",
        ));
    }
    Ok(())
}

fn join_bridge_path(parent: &str, child: &str) -> String {
    if parent.is_empty() {
        child.to_owned()
    } else {
        format!("{parent}/{child}")
    }
}

fn text_too_large(operation: &str) -> NativeError {
    NativeError {
        code: ErrorCode::ResourceTooLarge,
        message: format!(
            "Text {operation} are limited to {MAX_TEXT_BYTES} bytes through the bridge."
        ),
    }
}

fn canonical_grant_target(selected: &Path, access: FileAccess) -> Result<PathBuf, NativeError> {
    match std::fs::canonicalize(selected) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == ErrorKind::NotFound && access.can_write() => {
            let parent = selected.parent().ok_or_else(|| {
                NativeError::invalid("The selected save path has no parent directory.")
            })?;
            let parent = std::fs::canonicalize(parent)
                .map_err(|error| NativeError::io("Cannot resolve the save directory", &error))?;
            let name = selected
                .file_name()
                .ok_or_else(|| NativeError::invalid("The selected save path has no filename."))?;
            Ok(parent.join(name))
        }
        Err(error) => Err(NativeError::io("Cannot resolve the selected path", &error)),
    }
}

fn load_persistent_grants(path: &Path) -> Result<HashMap<String, PersistentGrant>, String> {
    if !path.is_file() {
        return Ok(HashMap::new());
    }
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect persistent file grants: {error}"))?;
    if metadata.file_type().is_symlink() || metadata.len() > 1024 * 1024 {
        return Err("persistent file grants must be a regular file below 1 MiB".to_owned());
    }
    let grants: HashMap<String, PersistentGrant> = serde_json::from_slice(
        &std::fs::read(path)
            .map_err(|error| format!("cannot read persistent file grants: {error}"))?,
    )
    .map_err(|error| format!("cannot decode persistent file grants: {error}"))?;
    if grants.len() > 128
        || grants.iter().any(|(id, grant)| {
            id.len() != 64
                || !id.bytes().all(|byte| byte.is_ascii_hexdigit())
                || !grant.path.is_absolute()
        })
    {
        return Err("persistent file grant store has an invalid entry".to_owned());
    }
    Ok(grants)
}

fn persist_grants(
    path: &Path,
    grants: &HashMap<String, PersistentGrant>,
) -> Result<(), NativeError> {
    if grants.len() > 128 {
        return Err(NativeError::too_large(
            "Applications may retain at most 128 persistent file grants.",
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| NativeError::native("Cannot persist file grants", "missing parent"))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| NativeError::io("Cannot create the file grant store", &error))?;
    let bytes = serde_json::to_vec(grants)
        .map_err(|error| NativeError::native("Cannot encode persistent file grants", error))?;
    let temporary = path.with_extension("json.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options
        .open(&temporary)
        .map_err(|error| NativeError::io("Cannot create persistent file grants", &error))?;
    output
        .write_all(&bytes)
        .and_then(|()| output.sync_all())
        .map_err(|error| NativeError::io("Cannot write persistent file grants", &error))?;
    std::fs::rename(&temporary, path)
        .map_err(|error| NativeError::io("Cannot publish persistent file grants", &error))
}

fn persistent_grants_path(application_id: &str) -> Result<PathBuf, String> {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
    }
    .ok_or_else(|| "cannot locate OS data directory for persistent file grants".to_owned())?;
    Ok(base
        .join("pam-desktop")
        .join(application_id)
        .join("file-grants.json"))
}

fn secure_grant_id() -> Result<String, NativeError> {
    let mut bytes = [0_u8; 32];
    fill(&mut bytes)
        .map_err(|error| NativeError::native("Cannot create a secure file grant", error))?;
    let mut token = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut token, "{byte:02x}")
            .map_err(|error| NativeError::native("Cannot encode a secure file grant", error))?;
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pam_desktop_protocol::FileSystemRootConfig;

    fn temporary_directory() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "pam-desktop-native-{}",
            secure_grant_id().expect("a temporary name should be generated")
        ));
        std::fs::create_dir_all(&root).expect("the temporary directory should be created");
        root
    }

    fn services(root: &Path, access: FileAccess) -> NativeServices {
        NativeServices::prepare(
            root,
            "com.pushin.pam-desktop-test",
            &NativeCapabilities {
                filesystem_roots: vec![FileSystemRootConfig {
                    name: "data".to_owned(),
                    path: ".".to_owned(),
                    access,
                }],
                ..NativeCapabilities::default()
            },
        )
        .expect("native services should prepare")
    }

    fn target(path: &str) -> FileTarget {
        FileTarget {
            root: Some("data".to_owned()),
            grant_id: None,
            path: path.to_owned(),
        }
    }

    #[test]
    fn reads_writes_lists_and_inspects_an_authorized_root() {
        let root = temporary_directory();
        let services = services(&root, FileAccess::ReadWrite);
        let write = services
            .filesystem(&FileRequest {
                operation: FileOperation::WriteText,
                window_id: "main".to_owned(),
                target: target("notes/hello.txt"),
                content: Some("Olá, Pam.".to_owned()),
            })
            .expect_err("missing parent directories should not be ambiently created");
        assert_eq!(write.code, ErrorCode::ResourceNotFound);

        services
            .filesystem(&FileRequest {
                operation: FileOperation::CreateDirectory,
                window_id: "main".to_owned(),
                target: target("notes"),
                content: None,
            })
            .expect("the directory should be created");
        services
            .filesystem(&FileRequest {
                operation: FileOperation::WriteText,
                window_id: "main".to_owned(),
                target: target("notes/hello.txt"),
                content: Some("Olá, Pam.".to_owned()),
            })
            .expect("the file should be written");
        let read = services
            .filesystem(&FileRequest {
                operation: FileOperation::ReadText,
                window_id: "main".to_owned(),
                target: target("notes/hello.txt"),
                content: None,
            })
            .expect("the file should be read");
        assert_eq!(read["text"], "Olá, Pam.");
        let list = services
            .filesystem(&FileRequest {
                operation: FileOperation::List,
                window_id: "main".to_owned(),
                target: target("notes"),
                content: None,
            })
            .expect("the directory should be listed");
        assert_eq!(list[0]["kind"], FileEntryKind::File as u8);
        assert_eq!(list[0]["path"], "notes/hello.txt");

        drop(services);
        std::fs::remove_dir_all(root).expect("the temporary directory should be removed");
    }

    #[test]
    fn rejects_parent_paths_and_write_without_permission() {
        let root = temporary_directory();
        let services = services(&root, FileAccess::Read);
        let escape = services
            .filesystem(&FileRequest {
                operation: FileOperation::ReadText,
                window_id: "main".to_owned(),
                target: target("../outside"),
                content: None,
            })
            .expect_err("parent traversal should fail");
        assert_eq!(escape.code, ErrorCode::InvalidPayload);
        let write = services
            .filesystem(&FileRequest {
                operation: FileOperation::WriteText,
                window_id: "main".to_owned(),
                target: target("blocked.txt"),
                content: Some("blocked".to_owned()),
            })
            .expect_err("read-only roots should reject writes");
        assert_eq!(write.code, ErrorCode::PermissionDenied);

        drop(services);
        std::fs::remove_dir_all(root).expect("the temporary directory should be removed");
    }

    #[test]
    fn opens_capability_scoped_binary_streams() {
        let root = temporary_directory();
        let services = services(&root, FileAccess::ReadWrite);
        let mut writer = services
            .open_write_stream(&target("payload.bin"))
            .expect("the streaming destination should open");
        writer
            .write_all(&[0, 1, 2, 3, 255])
            .expect("binary bytes should be written");
        writer.sync_all().expect("binary bytes should be durable");
        drop(writer);

        let (mut reader, bytes) = services
            .open_read_stream(&target("payload.bin"))
            .expect("the streaming source should open");
        let mut payload = Vec::new();
        reader
            .read_to_end(&mut payload)
            .expect("binary bytes should be read");
        assert_eq!(bytes, 5);
        assert_eq!(payload, [0, 1, 2, 3, 255]);

        drop(reader);
        drop(services);
        std::fs::remove_dir_all(root).expect("the temporary directory should be removed");
    }

    #[test]
    fn resolves_only_readable_canonical_clipboard_files() {
        let root = temporary_directory();
        std::fs::write(root.join("document.txt"), "PAM").expect("clipboard fixture");
        let services = services(&root, FileAccess::Read);
        assert_eq!(
            services
                .clipboard_file_path(&target("document.txt"))
                .expect("regular authorized file"),
            root.join("document.txt")
                .canonicalize()
                .expect("clipboard fixture should canonicalize")
        );
        assert!(
            services
                .clipboard_file_path(&target("missing.txt"))
                .is_err()
        );

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("document.txt"), root.join("alias.txt"))
                .expect("symlink fixture");
            assert!(services.clipboard_file_path(&target("alias.txt")).is_err());
        }

        drop(services);
        std::fs::remove_dir_all(root).expect("the temporary directory should be removed");
    }

    #[test]
    fn validates_clipboard_rgba_shape_and_bounds_without_opening_the_system_clipboard() {
        assert_eq!(
            validate_custom_clipboard_format(Some("application/vnd.pushin.document+json"))
                .expect("vendor clipboard format"),
            "application/vnd.pushin.document+json"
        );
        assert!(validate_custom_clipboard_format(Some("text/plain")).is_err());
        assert!(validate_custom_clipboard_format(Some("application/x-pam\nforged")).is_err());

        let valid = ClipboardImage {
            width: 1,
            height: 1,
            rgba_base64: base64::engine::general_purpose::STANDARD.encode([1, 2, 3, 4]),
        };
        assert_eq!(
            decode_clipboard_image(&valid).expect("one RGBA pixel should decode"),
            [1, 2, 3, 4],
        );

        let malformed = ClipboardImage {
            width: 2,
            height: 1,
            rgba_base64: valid.rgba_base64,
        };
        assert_eq!(
            decode_clipboard_image(&malformed)
                .expect_err("mismatched RGBA length must fail")
                .code,
            ErrorCode::InvalidPayload,
        );

        let overflow = ClipboardImage {
            width: usize::MAX,
            height: 2,
            rgba_base64: String::new(),
        };
        assert_eq!(
            decode_clipboard_image(&overflow)
                .expect_err("overflowing dimensions must fail")
                .code,
            ErrorCode::InvalidPayload,
        );
    }

    #[test]
    fn persists_bounded_file_grants_atomically() {
        let root = temporary_directory();
        let selected = root.join("document.txt");
        std::fs::write(&selected, "persistent").expect("fixture should exist");
        let path = root.join("state/file-grants.json");
        let id = "a".repeat(64);
        let grants = HashMap::from([(
            id.clone(),
            PersistentGrant {
                path: selected
                    .canonicalize()
                    .expect("fixture should canonicalize"),
                access: FileAccess::Read,
            },
        )]);
        persist_grants(&path, &grants).expect("grant store should persist");
        let loaded = load_persistent_grants(&path).expect("grant store should reload");
        assert_eq!(loaded[&id].access, FileAccess::Read);
        assert!(create_file_grant(&loaded[&id].path, loaded[&id].access).is_ok());
        std::fs::remove_dir_all(root).expect("fixture should be removable");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symbolic_links() {
        use std::os::unix::fs::symlink;

        let root = temporary_directory();
        let outside = temporary_directory();
        std::fs::write(outside.join("secret.txt"), "secret")
            .expect("the outside file should be written");
        symlink(outside.join("secret.txt"), root.join("escape.txt"))
            .expect("the test symlink should be created");
        let services = services(&root, FileAccess::Read);
        let error = services
            .filesystem(&FileRequest {
                operation: FileOperation::ReadText,
                window_id: "main".to_owned(),
                target: target("escape.txt"),
                content: None,
            })
            .expect_err("symlinks should not be exposed");
        assert_eq!(error.code, ErrorCode::PermissionDenied);

        std::fs::remove_dir_all(root).expect("the temporary directory should be removed");
        std::fs::remove_dir_all(outside).expect("the temporary directory should be removed");
    }
}
