//! Where an application keeps its files on each system.

use std::path::{Path, PathBuf};

/// Per-application directories following the platform conventions: XDG on
/// Linux, `~/Library` on macOS, `%APPDATA%` / `%LOCALAPPDATA%` on Windows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppPaths {
    /// Durable user data (databases, documents the app owns).
    pub data: PathBuf,
    /// Files that can be deleted at any time.
    pub cache: PathBuf,
    /// Session state (navigation, window geometry, hot reload checkpoints).
    pub state: PathBuf,
}

fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map_or_else(std::env::temp_dir, PathBuf::from)
}

fn from_env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

impl AppPaths {
    /// Directories of the installed application `id`.
    #[must_use]
    pub fn for_application(id: &str) -> Self {
        if cfg!(target_os = "macos") {
            let support = home().join("Library/Application Support").join(id);
            Self {
                state: support.join("State"),
                data: support,
                cache: home().join("Library/Caches").join(id),
            }
        } else if cfg!(windows) {
            let roaming = from_env("APPDATA").unwrap_or_else(|| home().join("AppData/Roaming"));
            let local = from_env("LOCALAPPDATA").unwrap_or_else(|| home().join("AppData/Local"));
            Self {
                data: roaming.join(id),
                cache: local.join(id).join("Cache"),
                state: local.join(id).join("State"),
            }
        } else {
            Self {
                data: from_env("XDG_DATA_HOME")
                    .unwrap_or_else(|| home().join(".local/share"))
                    .join(id),
                cache: from_env("XDG_CACHE_HOME")
                    .unwrap_or_else(|| home().join(".cache"))
                    .join(id),
                state: from_env("XDG_STATE_HOME")
                    .unwrap_or_else(|| home().join(".local/state"))
                    .join(id),
            }
        }
    }

    /// Development keeps everything inside the project, out of the user's profile.
    #[must_use]
    pub fn for_development(project: &Path) -> Self {
        let root = project.join(".pam-desktop");
        Self {
            data: root.join("data"),
            cache: root.join("cache"),
            state: root.join("state"),
        }
    }

    /// Environment overrides win (tests, sandboxes), directory by directory.
    #[must_use]
    pub fn with_overrides(mut self) -> Self {
        if let Some(path) = from_env("PAM_DESKTOP_DATA_DIR") {
            self.data = path;
        }
        if let Some(path) = from_env("PAM_DESKTOP_CACHE_DIR") {
            self.cache = path;
        }
        if let Some(path) = from_env("PAM_DESKTOP_STATE_DIR") {
            self.state = path;
        }
        self
    }

    pub fn create(&self) -> std::io::Result<()> {
        for directory in [&self.data, &self.cache, &self.state] {
            std::fs::create_dir_all(directory)?;
        }
        Ok(())
    }

    #[must_use]
    pub fn environment(&self) -> Vec<(String, String)> {
        vec![
            (
                "PAM_DESKTOP_DATA_DIR".into(),
                self.data.display().to_string(),
            ),
            (
                "PAM_DESKTOP_CACHE_DIR".into(),
                self.cache.display().to_string(),
            ),
            (
                "PAM_DESKTOP_STATE_DIR".into(),
                self.state.display().to_string(),
            ),
        ]
    }
}

/// A reverse-DNS application id usable as a directory name and a D-Bus name.
#[must_use]
pub fn valid_application_id(id: &str) -> bool {
    let segments: Vec<&str> = id.split('.').collect();
    id.len() <= 255
        && segments.len() >= 2
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && !segment.starts_with(|first: char| first.is_ascii_digit())
                && segment.chars().all(|character| {
                    character.is_ascii_alphanumeric() || character == '_' || character == '-'
                })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_ids_are_reverse_dns() {
        assert!(valid_application_id("dev.pam.showcase"));
        assert!(valid_application_id("com.acme.my-app_2"));
        assert!(!valid_application_id("showcase"));
        assert!(!valid_application_id("dev..pam"));
        assert!(!valid_application_id("dev.9pam"));
        assert!(!valid_application_id("dev.pam/../etc"));
    }

    #[test]
    fn development_paths_stay_inside_the_project() {
        let paths = AppPaths::for_development(Path::new("/work/app"));
        assert_eq!(paths.state, PathBuf::from("/work/app/.pam-desktop/state"));
        assert!(
            paths
                .environment()
                .iter()
                .any(|(key, _)| key == "PAM_DESKTOP_CACHE_DIR")
        );
    }
}
