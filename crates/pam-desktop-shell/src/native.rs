use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use arboard::Clipboard;
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use getrandom::fill;
use notify_rust::{Notification, Urgency};
use pam_desktop_protocol::{
    ClipboardOperation, DialogKind, ErrorCode, FileAccess, FileEntryKind, FileOperation,
    NativeCapabilities, NotificationUrgency,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::oneshot;

use crate::database::{DatabaseRequest, DatabaseServices};
use crate::http_client::{HttpRequest, HttpServices};
use crate::process_runner::{ProcessRequest, ProcessServices};

const MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_CLIPBOARD_BYTES: usize = 1024 * 1024;
const MAX_NOTIFICATION_TITLE_BYTES: usize = 256;
const MAX_NOTIFICATION_BODY_BYTES: usize = 4 * 1024;
const MAX_DIALOG_FILTERS: usize = 16;
const MAX_DIALOG_EXTENSIONS: usize = 32;
pub const MAX_STREAM_BYTES: u64 = 4 * 1024 * 1024 * 1024;

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
    clipboard: Mutex<Option<Clipboard>>,
    databases: DatabaseServices,
    http: HttpServices,
    processes: ProcessServices,
}

impl NativeServices {
    pub fn prepare(project_root: &Path, capabilities: &NativeCapabilities) -> Result<Self, String> {
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

        Ok(Self {
            capabilities: capabilities.clone(),
            roots,
            grants: Mutex::new(HashMap::new()),
            clipboard: Mutex::new(None),
            databases: DatabaseServices::prepare(project_root, &capabilities.databases)?,
            http: HttpServices::prepare(&capabilities.http_origins)?,
            processes: ProcessServices::prepare(project_root, &capabilities.processes)?,
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
        serde_json::to_value(self.processes.execute(request)?)
            .map_err(|error| NativeError::native("Cannot encode process result", error))
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

    pub fn clipboard(&self, request: &ClipboardRequest) -> Result<Value, NativeError> {
        match request.operation {
            ClipboardOperation::ReadText if !self.capabilities.clipboard_read => {
                return Err(NativeError::disabled("clipboard read"));
            }
            ClipboardOperation::WriteText | ClipboardOperation::Clear
                if !self.capabilities.clipboard_write =>
            {
                return Err(NativeError::disabled("clipboard write"));
            }
            _ => {}
        }
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
            ClipboardOperation::Clear => {
                clipboard
                    .clear()
                    .map_err(|error| NativeError::native("Cannot clear the clipboard", error))?;
                Ok(Value::Null)
            }
        }
    }

    pub fn notify(&self, request: &NotificationRequest) -> Result<Value, NativeError> {
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
        let urgency = match request.urgency {
            NotificationUrgency::Low => Urgency::Low,
            NotificationUrgency::Normal => Urgency::Normal,
            NotificationUrgency::Critical => Urgency::Critical,
        };
        Notification::new()
            .appname("Pam Desktop")
            .summary(&request.title)
            .body(&request.body)
            .urgency(urgency)
            .show()
            .map_err(|error| NativeError::native("Cannot show the notification", error))?;
        Ok(Value::Null)
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
    ) -> Result<Vec<FileReference>, NativeError> {
        paths
            .into_iter()
            .map(|path| self.grant_path(&path, access))
            .collect()
    }

    pub fn grant_path(
        &self,
        selected: &Path,
        access: FileAccess,
    ) -> Result<FileReference, NativeError> {
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
                    let file_name = canonical.file_name().ok_or_else(|| {
                        NativeError::invalid("The selected file has no filename.")
                    })?;
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
                let parent = std::fs::canonicalize(parent).map_err(|error| {
                    NativeError::io("Cannot resolve the save directory", &error)
                })?;
                let file_name = selected.file_name().ok_or_else(|| {
                    NativeError::invalid("The selected save path has no filename.")
                })?;
                (
                    parent,
                    PathBuf::from(file_name),
                    FileEntryKind::File,
                    file_name.to_string_lossy().into_owned(),
                )
            }
            Err(error) => return Err(NativeError::io("Cannot inspect the selected path", &error)),
        };

        let directory = Dir::open_ambient_dir(&directory_path, ambient_authority())
            .map_err(|error| NativeError::io("Cannot open the granted directory", &error))?;
        let grant_id = secure_grant_id()?;
        self.grants
            .lock()
            .map_err(|_| NativeError::native("Cannot store the file grant", "lock is poisoned"))?
            .insert(
                grant_id.clone(),
                FileGrant {
                    directory: Arc::new(directory),
                    relative,
                    kind,
                    access,
                    name: name.clone(),
                },
            );

        Ok(FileReference {
            grant_id,
            name,
            kind,
            access,
        })
    }

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
}

struct ResolvedTarget {
    directory: Arc<Dir>,
    relative: PathBuf,
    access: FileAccess,
    display_name: String,
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

        std::fs::remove_dir_all(root).expect("the temporary directory should be removed");
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
