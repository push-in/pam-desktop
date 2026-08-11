use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, UNIX_EPOCH};

use pam_desktop_protocol::{ClientEvent, FileWatchOperation, validate_identifier};
use serde::Deserialize;
use serde_json::json;

use crate::event_hub::EventHub;
use crate::native::{FileTarget, NativeError, NativeServices};

const INTERVAL: Duration = Duration::from_millis(250);
const MAX_ENTRIES: usize = 10_000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileWatchRequest {
    pub operation: FileWatchOperation,
    pub window_id: String,
    pub watch_id: String,
    pub target: Option<FileTarget>,
}

pub struct FileWatchManager {
    watches: Mutex<HashMap<String, WatchHandle>>,
}

impl FileWatchManager {
    pub fn new() -> Self {
        Self {
            watches: Mutex::new(HashMap::new()),
        }
    }

    pub fn dispatch(
        &self,
        native: &NativeServices,
        events: &EventHub,
        request: &FileWatchRequest,
    ) -> Result<(), NativeError> {
        validate_identifier(&request.watch_id, "file watch").map_err(NativeError::invalid)?;
        let key = format!("{}:{}", request.window_id, request.watch_id);
        let mut watches = self
            .watches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match request.operation {
            FileWatchOperation::Start => {
                let target = request.target.as_ref().ok_or_else(|| {
                    NativeError::invalid("Starting a file watch requires a target.")
                })?;
                let path = native.watch_path(target)?;
                if watches.contains_key(&key) {
                    return Err(NativeError::invalid("The file watch is already active."));
                }
                watches.insert(
                    key,
                    WatchHandle::start(
                        path,
                        request.window_id.clone(),
                        request.watch_id.clone(),
                        events.clone(),
                    )?,
                );
            }
            FileWatchOperation::Stop => {
                watches
                    .remove(&key)
                    .ok_or_else(|| NativeError::invalid("The file watch is not active."))?;
            }
        }
        Ok(())
    }
}

struct WatchHandle {
    stop: std::sync::Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl WatchHandle {
    fn start(
        path: PathBuf,
        window_id: String,
        watch_id: String,
        events: EventHub,
    ) -> Result<Self, NativeError> {
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread = std::thread::Builder::new()
            .name(format!("pam-watch-{watch_id}"))
            .spawn(move || {
                let mut prior = snapshot(&path);
                while !thread_stop.load(Ordering::Acquire) {
                    std::thread::sleep(INTERVAL);
                    let current = snapshot(&path);
                    if current != prior {
                        events.publish(ClientEvent {
                            name: "pam.fs.changed".to_owned(),
                            payload: json!({"watchId": watch_id}),
                            window_id: Some(window_id.clone()),
                        });
                        prior = current;
                    }
                }
            })
            .map_err(|error| NativeError::native("Cannot start file watcher", error))?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}
impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn snapshot(path: &Path) -> BTreeMap<PathBuf, (u64, u128)> {
    let mut result = BTreeMap::new();
    collect(path, path, &mut result);
    result
}
fn collect(root: &Path, path: &Path, output: &mut BTreeMap<PathBuf, (u64, u128)>) {
    if output.len() >= MAX_ENTRIES {
        return;
    }
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_type().is_symlink() {
        return;
    }
    if metadata.is_file() {
        let modified = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |value| value.as_nanos());
        output.insert(
            path.strip_prefix(root).unwrap_or(path).to_path_buf(),
            (metadata.len(), modified),
        );
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        collect(root, &entry.path(), output);
        if output.len() >= MAX_ENTRIES {
            break;
        }
    }
}
