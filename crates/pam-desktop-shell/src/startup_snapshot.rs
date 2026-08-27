use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use pam_desktop_protocol::{Bootstrap, PROTOCOL_VERSION};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::project::Project;

const SNAPSHOT_SCHEMA: u16 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SnapshotFile {
    schema: u16,
    protocol: u16,
    project_fingerprint: String,
    bootstrap: Bootstrap,
}

pub struct StartupSnapshot {
    path: PathBuf,
    fingerprint: String,
}

impl StartupSnapshot {
    pub fn prepare(project: &Project) -> Result<Self, String> {
        Ok(Self {
            path: snapshot_path(project.root())?,
            fingerprint: project_fingerprint(project.root())?,
        })
    }

    pub fn load(&self, project: &Project) -> Option<Bootstrap> {
        let bytes = fs::read(&self.path).ok()?;
        let snapshot: SnapshotFile = serde_json::from_slice(&bytes).ok()?;
        if snapshot.schema != SNAPSHOT_SCHEMA
            || snapshot.protocol != PROTOCOL_VERSION
            || snapshot.project_fingerprint != self.fingerprint
            || project.validate_bootstrap(&snapshot.bootstrap).is_err()
            || !snapshot.bootstrap.workstation.startup_snapshot
        {
            return None;
        }
        Some(snapshot.bootstrap)
    }

    pub fn publish(&self, bootstrap: &Bootstrap) -> Result<(), String> {
        if !bootstrap.workstation.startup_snapshot {
            return self.remove();
        }
        let snapshot = SnapshotFile {
            schema: SNAPSHOT_SCHEMA,
            protocol: PROTOCOL_VERSION,
            project_fingerprint: self.fingerprint.clone(),
            bootstrap: bootstrap.clone(),
        };
        let bytes = serde_json::to_vec(&snapshot)
            .map_err(|error| format!("cannot encode startup snapshot: {error}"))?;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| "startup snapshot path has no parent".to_owned())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create startup snapshot directory: {error}"))?;
        restrict_directory(parent)?;
        let temporary = self.path.with_extension("json.tmp");
        let mut options = fs::OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|error| format!("cannot create startup snapshot: {error}"))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("cannot persist startup snapshot: {error}"))?;
        fs::rename(&temporary, &self.path)
            .map_err(|error| format!("cannot publish startup snapshot: {error}"))
    }

    fn remove(&self) -> Result<(), String> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("cannot remove disabled startup snapshot: {error}")),
        }
    }
}

fn project_fingerprint(root: &Path) -> Result<String, String> {
    let mut candidates = Vec::new();
    collect_inputs(root, &mut candidates)?;
    candidates.sort();
    let mut digest = Sha256::new();
    digest.update(b"pam-desktop-startup-snapshot-v1\0");
    for path in candidates {
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "startup snapshot input escaped the project".to_owned())?;
        digest.update(relative.to_string_lossy().as_bytes());
        digest.update([0]);
        let bytes = fs::read(&path)
            .map_err(|error| format!("cannot fingerprint {}: {error}", path.display()))?;
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn collect_inputs(directory: &Path, output: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("cannot inspect {}: {error}", directory.display()))?
    {
        let entry = entry.map_err(|error| format!("cannot inspect project entry: {error}"))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if matches!(
                name.as_ref(),
                ".git" | ".pam" | "dist" | "node_modules" | "target" | "vendor"
            ) {
                continue;
            }
            collect_inputs(&path, output)?;
            continue;
        }
        if path.is_file()
            && (matches!(
                name.as_ref(),
                "composer.json" | "composer.lock" | "pam.json"
            ) || matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("php")
            ))
        {
            output.push(path);
        }
    }
    Ok(())
}

fn snapshot_path(project_root: &Path) -> Result<PathBuf, String> {
    let canonical = project_root
        .canonicalize()
        .map_err(|error| format!("cannot identify project for startup snapshot: {error}"))?;
    let project_id = format!(
        "{:x}",
        Sha256::digest(canonical.to_string_lossy().as_bytes())
    );
    Ok(cache_root()?
        .join("pam-desktop/snapshots")
        .join(format!("{project_id}.json")))
}

fn cache_root() -> Result<PathBuf, String> {
    if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Caches"))
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
    }
    .ok_or_else(|| "cannot locate operating-system cache directory".to_owned())
}

fn restrict_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("cannot restrict startup snapshot directory: {error}"))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_ignores_build_and_dependency_directories() {
        let temporary =
            std::env::temp_dir().join(format!("pam-desktop-snapshot-test-{}", std::process::id()));
        fs::create_dir(&temporary).expect("temporary directory");
        fs::write(temporary.join("app.php"), "<?php return 1;").expect("app");
        fs::create_dir(temporary.join("vendor")).expect("vendor");
        fs::write(temporary.join("vendor/ignored.php"), "first").expect("vendor file");
        let before = project_fingerprint(&temporary).expect("fingerprint");
        fs::write(temporary.join("vendor/ignored.php"), "second").expect("vendor file");
        let after = project_fingerprint(&temporary).expect("fingerprint");
        assert_eq!(before, after);
        fs::write(temporary.join("app.php"), "<?php return 2;").expect("app");
        assert_ne!(before, project_fingerprint(&temporary).expect("changed"));
        fs::remove_dir_all(temporary).expect("cleanup");
    }
}
