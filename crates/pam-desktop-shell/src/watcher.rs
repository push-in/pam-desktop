use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, UNIX_EPOCH};

const WATCH_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeKind {
    Assets,
    Runtime,
}

pub struct ProjectWatcher {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ProjectWatcher {
    pub fn start(
        root: PathBuf,
        on_change: impl Fn(ChangeKind) + Send + 'static,
    ) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let watcher_stop = stop.clone();
        let thread = std::thread::Builder::new()
            .name("pam-desktop-watcher".to_owned())
            .spawn(move || {
                let mut previous = snapshot(&root);
                while !watcher_stop.load(Ordering::Acquire) {
                    std::thread::sleep(WATCH_INTERVAL);
                    let current = snapshot(&root);
                    if let Some(kind) = changed_kind(&previous, &current) {
                        on_change(kind);
                    }
                    previous = current;
                }
            })
            .map_err(|error| format!("cannot start desktop project watcher: {error}"))?;

        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for ProjectWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileStamp {
    bytes: u64,
    modified_nanos: u128,
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, FileStamp> {
    let mut files = BTreeMap::new();
    collect(root, root, &mut files);
    files
}

fn collect(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, FileStamp>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if !ignored_directory(relative) {
                collect(root, &path, files);
            }
            continue;
        }
        if !file_type.is_file() || !watched_file(relative) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let modified_nanos = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_nanos());
        files.insert(
            relative.to_path_buf(),
            FileStamp {
                bytes: metadata.len(),
                modified_nanos,
            },
        );
    }
}

fn ignored_directory(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some(".git" | ".pam" | "dist" | "node_modules" | "target" | "vendor")
        )
    })
}

fn watched_file(path: &Path) -> bool {
    path.starts_with("resources")
        || path.extension().is_some_and(|extension| extension == "php")
        || matches!(
            path.file_name().and_then(|name| name.to_str()),
            Some("composer.json" | "composer.lock")
        )
}

fn changed_kind(
    previous: &BTreeMap<PathBuf, FileStamp>,
    current: &BTreeMap<PathBuf, FileStamp>,
) -> Option<ChangeKind> {
    let paths = previous
        .keys()
        .chain(current.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let changed = paths
        .into_iter()
        .filter(|path| previous.get(path) != current.get(path))
        .collect::<Vec<_>>();
    if changed.is_empty() {
        return None;
    }
    if changed.iter().any(|path| !path.starts_with("resources")) {
        Some(ChangeKind::Runtime)
    } else {
        Some(ChangeKind::Assets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_resource_and_php_changes() {
        let mut before = BTreeMap::new();
        before.insert(
            PathBuf::from("resources/app.js"),
            FileStamp {
                bytes: 1,
                modified_nanos: 1,
            },
        );
        let mut after = before.clone();
        after.insert(
            PathBuf::from("resources/app.js"),
            FileStamp {
                bytes: 2,
                modified_nanos: 2,
            },
        );
        assert_eq!(changed_kind(&before, &after), Some(ChangeKind::Assets));

        after.insert(
            PathBuf::from("app.php"),
            FileStamp {
                bytes: 2,
                modified_nanos: 2,
            },
        );
        assert_eq!(changed_kind(&before, &after), Some(ChangeKind::Runtime));
    }
}
