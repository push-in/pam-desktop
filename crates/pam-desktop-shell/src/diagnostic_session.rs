use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_DESCRIPTOR_BYTES: u64 = 8 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const BRIDGE_HEADER: &str = "x-pam-bridge";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SessionDescriptor {
    schema_version: u8,
    surface_code: u8,
    process_id: u32,
    started_at_unix_ms: u64,
    origin: String,
    bridge_token: String,
    window_id: String,
}

pub struct DiagnosticSession {
    path: PathBuf,
    process_id: u32,
    bridge_token: String,
}

impl DiagnosticSession {
    pub fn create(root: &Path, origin: &str, token: &str, window_id: &str) -> Result<Self, String> {
        validate_origin(origin)?;
        validate_token(token)?;
        validate_window_id(window_id)?;
        let directory = root.join(".pam");
        prepare_private_directory(&directory)?;
        let path = directory.join("desktop-session.json");
        reject_symlink(&path)?;
        let process_id = std::process::id();
        let descriptor = SessionDescriptor {
            schema_version: 1,
            surface_code: 3,
            process_id,
            started_at_unix_ms: unix_milliseconds(),
            origin: origin.to_owned(),
            bridge_token: token.to_owned(),
            window_id: window_id.to_owned(),
        };
        let encoded = serde_json::to_vec_pretty(&descriptor)
            .map_err(|error| format!("cannot encode desktop diagnostic session: {error}"))?;
        if u64::try_from(encoded.len()).unwrap_or(u64::MAX) > MAX_DESCRIPTOR_BYTES {
            return Err("desktop diagnostic session descriptor is too large".to_owned());
        }
        let temporary = directory.join(format!(".desktop-session-{process_id}.tmp"));
        reject_symlink(&temporary)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut output = options
            .open(&temporary)
            .map_err(|error| format!("cannot create {}: {error}", temporary.display()))?;
        if let Err(error) = output.write_all(&encoded).and_then(|()| output.sync_all()) {
            let _ = fs::remove_file(&temporary);
            return Err(format!("cannot write {}: {error}", temporary.display()));
        }
        if let Err(error) = fs::rename(&temporary, &path) {
            let _ = fs::remove_file(&temporary);
            return Err(format!("cannot publish {}: {error}", path.display()));
        }
        Ok(Self {
            path,
            process_id,
            bridge_token: token.to_owned(),
        })
    }
}

impl Drop for DiagnosticSession {
    fn drop(&mut self) {
        let Ok(descriptor) = read_descriptor_path(&self.path) else {
            return;
        };
        if descriptor.process_id == self.process_id && descriptor.bridge_token == self.bridge_token
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub fn capture(root: &Path) -> Result<Value, String> {
    let path = root.join(".pam/desktop-session.json");
    let descriptor = read_descriptor_path(&path).map_err(|error| {
        format!("{error}. Start this project with `pam dev`, then retry `pam diagnostics`")
    })?;
    let endpoint = format!("{}/_pam/diagnostics", descriptor.origin);
    let agent = ureq::Agent::config_builder()
        .https_only(false)
        .max_redirects(0)
        .timeout_global(Some(REQUEST_TIMEOUT))
        .user_agent(concat!("pam-desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent();
    let mut response = agent
        .post(&endpoint)
        .header("origin", &descriptor.origin)
        .header(BRIDGE_HEADER, &descriptor.bridge_token)
        .send_json(serde_json::json!({"windowId": descriptor.window_id}))
        .map_err(|error| {
            format!(
                "cannot reach the active PAM Desktop diagnostic session: {error}. Restart `pam dev` to replace a stale descriptor"
            )
        })?;
    let body = response
        .body_mut()
        .with_config()
        .limit(64 * 1024)
        .read_to_string()
        .map_err(|error| format!("cannot read the bounded diagnostic response: {error}"))?;
    let envelope: Value = serde_json::from_str(&body)
        .map_err(|error| format!("desktop diagnostic response is invalid JSON: {error}"))?;
    if envelope.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err("desktop diagnostic session rejected the snapshot request".to_owned());
    }
    let snapshot = envelope
        .get("data")
        .cloned()
        .ok_or_else(|| "desktop diagnostic response has no data".to_owned())?;
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

fn read_descriptor_path(path: &Path) -> Result<SessionDescriptor, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_DESCRIPTOR_BYTES
    {
        return Err(format!(
            "invalid diagnostic session descriptor: {}",
            path.display()
        ));
    }
    let descriptor: SessionDescriptor = serde_json::from_slice(
        &fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("invalid {}: {error}", path.display()))?;
    if descriptor.schema_version != 1 || descriptor.surface_code != 3 || descriptor.process_id == 0
    {
        return Err("unsupported desktop diagnostic session descriptor".to_owned());
    }
    validate_origin(&descriptor.origin)?;
    validate_token(&descriptor.bridge_token)?;
    validate_window_id(&descriptor.window_id)?;
    Ok(descriptor)
}

fn validate_origin(origin: &str) -> Result<(), String> {
    let parsed =
        url::Url::parse(origin).map_err(|error| format!("invalid gateway origin: {error}"))?;
    if parsed.scheme() != "http"
        || parsed.host_str() != Some("127.0.0.1")
        || parsed.port().is_none()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("desktop diagnostic origin must be a loopback HTTP origin".to_owned());
    }
    Ok(())
}

fn validate_token(token: &str) -> Result<(), String> {
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(
            "desktop diagnostic bridge token must contain 64 hexadecimal characters".to_owned(),
        );
    }
    Ok(())
}

fn validate_window_id(window_id: &str) -> Result<(), String> {
    if window_id.is_empty()
        || window_id.len() > 80
        || !window_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err("desktop diagnostic window id is invalid".to_owned());
    }
    Ok(())
}

fn validate_snapshot(snapshot: &Value) -> Result<(), String> {
    if snapshot.get("schemaVersion").and_then(Value::as_u64) != Some(1)
        || snapshot.get("surfaceCode").and_then(Value::as_u64) != Some(3)
        || snapshot
            .get("capturedAtUnixMs")
            .and_then(Value::as_u64)
            .is_none()
    {
        return Err(
            "desktop diagnostic response violates the DevTools snapshot envelope".to_owned(),
        );
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "refusing diagnostic session symlink: {}",
            path.display()
        )),
        Ok(metadata) if !metadata.is_file() => Err(format!(
            "diagnostic session path is not a regular file: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
    }
}

fn prepare_private_directory(directory: &Path) -> Result<(), String> {
    match fs::symlink_metadata(directory) {
        Ok(metadata) if !metadata.is_dir() || metadata.file_type().is_symlink() => {
            return Err(format!(
                "invalid diagnostic directory: {}",
                directory.display()
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(directory)
                .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
        }
        Err(error) => return Err(format!("cannot inspect {}: {error}", directory.display())),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("cannot secure {}: {error}", directory.display()))?;
    }
    Ok(())
}

fn unix_milliseconds() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "pam-desktop-{name}-{}-{}",
            std::process::id(),
            unix_milliseconds()
        ))
    }

    #[test]
    fn an_old_process_does_not_remove_a_replacement_session() {
        let root = temporary_root("diagnostic-replacement");
        fs::create_dir(&root).unwrap();
        let session =
            DiagnosticSession::create(&root, "http://127.0.0.1:3210/", &"ab".repeat(32), "main")
                .unwrap();
        let path = root.join(".pam/desktop-session.json");
        let mut replacement = read_descriptor_path(&path).unwrap();
        replacement.bridge_token = "cd".repeat(32);
        fs::write(&path, serde_json::to_vec(&replacement).unwrap()).unwrap();

        drop(session);

        assert!(path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn writes_private_descriptor_and_removes_its_session() {
        let root = temporary_root("diagnostic-session");
        fs::create_dir(&root).unwrap();
        let token = "ab".repeat(32);
        let session =
            DiagnosticSession::create(&root, "http://127.0.0.1:3210/", &token, "main").unwrap();
        let path = root.join(".pam/desktop-session.json");
        let descriptor = read_descriptor_path(&path).unwrap();
        assert_eq!(descriptor.schema_version, 1);
        assert_eq!(descriptor.surface_code, 3);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(root.join(".pam"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        drop(session);
        assert!(!path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_non_loopback_and_unknown_descriptor_fields() {
        let root = temporary_root("diagnostic-invalid");
        fs::create_dir(&root).unwrap();
        assert!(
            DiagnosticSession::create(&root, "https://example.com/", &"cd".repeat(32), "main")
                .is_err()
        );
        fs::create_dir(root.join(".pam")).unwrap();
        fs::write(
            root.join(".pam/desktop-session.json"),
            r#"{"schemaVersion":1,"surfaceCode":3,"processId":1,"startedAtUnixMs":1,"origin":"http://127.0.0.1:1/","bridgeToken":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","windowId":"main","extra":true}"#,
        )
        .unwrap();
        assert!(read_descriptor_path(&root.join(".pam/desktop-session.json")).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
