//! Development file watcher: restarts the PHP worker when application
//! sources change.
//!
//! It polls modification times instead of using platform notification APIs:
//! the same code runs on every system, a project has a few hundred source
//! files, and the thread only exists in development.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

use crate::runtime::{Inbound, Inbox};

/// A save reaches the window in about a quarter of a second: half a poll on
/// average, the settle delay and the worker restart. A scan of a few hundred
/// files costs well under a millisecond.
const POLL_INTERVAL: Duration = Duration::from_millis(150);
/// How long the tree must stay unchanged before the reload: editors write in
/// several steps (temporary file, rename, formatter).
const SETTLE_INTERVAL: Duration = Duration::from_millis(80);
const IGNORED_DIRECTORIES: [&str; 8] = [
    "vendor",
    "node_modules",
    "dist",
    "target",
    "build",
    "storage",
    "cache",
    "tmp",
];
const WATCHED_EXTENSIONS: [&str; 8] = ["php", "pam", "json", "css", "svg", "png", "jpg", "ttf"];
const MAX_DEPTH: usize = 12;

fn fingerprint_directory(directory: &Path, depth: usize, hasher: &mut DefaultHasher) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if name.starts_with('.') || IGNORED_DIRECTORIES.contains(&name) {
                continue;
            }
            fingerprint_directory(&entry.path(), depth + 1, hasher);
        } else if kind.is_file() {
            let watched = Path::new(name)
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| WATCHED_EXTENSIONS.contains(&extension));
            if !watched {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            name.hash(hasher);
            metadata.len().hash(hasher);
            if let Ok(modified) = metadata.modified()
                && let Ok(elapsed) = modified.duration_since(UNIX_EPOCH)
            {
                elapsed.as_nanos().hash(hasher);
            }
        }
    }
}

/// A hash of every watched file's name, size and modification time.
#[must_use]
pub fn fingerprint(roots: &[PathBuf]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for root in roots {
        fingerprint_directory(root, 0, &mut hasher);
    }
    hasher.finish()
}

/// Posts [`Inbound::Reload`] whenever the sources under `roots` change. The
/// thread ends with the process.
pub fn watch(roots: Vec<PathBuf>, inbox: Arc<Inbox>) {
    let _ = thread::Builder::new()
        .name("pam-watch".into())
        .spawn(move || {
            let mut current = fingerprint(&roots);
            loop {
                thread::sleep(POLL_INTERVAL);
                let next = fingerprint(&roots);
                if next == current {
                    continue;
                }
                // Editors write in several steps: wait until the tree is quiet.
                let mut settled = next;
                loop {
                    thread::sleep(SETTLE_INTERVAL);
                    let again = fingerprint(&roots);
                    if again == settled {
                        break;
                    }
                    settled = again;
                }
                current = settled;
                inbox.post(Inbound::Reload);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_tracks_watched_files_only() {
        let root = std::env::temp_dir().join(format!("pam-watch-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("vendor/package")).unwrap();
        std::fs::write(root.join("src/App.php"), "<?php").unwrap();
        let roots = vec![root.clone()];
        let initial = fingerprint(&roots);

        std::fs::write(root.join("vendor/package/Ignored.php"), "<?php").unwrap();
        std::fs::write(root.join("src/notes.txt"), "ignored").unwrap();
        assert_eq!(fingerprint(&roots), initial);

        std::fs::write(root.join("src/App.php"), "<?php // changed").unwrap();
        assert_ne!(fingerprint(&roots), initial);
        let _ = std::fs::remove_dir_all(&root);
    }
}
