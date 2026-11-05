//! `storage`: the small durable key/value store behind `Storage::get/set`.
//!
//! Values live in one JSON file in the application's data directory. Writes
//! are serialized on a worker thread and committed atomically (write, sync,
//! rename), so a `get` after a `set` sees the value and a crash never leaves
//! a half-written file.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::{request_map, Module, Reply, SerialWorker};
use crate::wiremap::WireMap;

const MAX_VALUE_BYTES: usize = 262_144;

fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 128 && key.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

struct Store {
    path: PathBuf,
    values: Option<BTreeMap<String, String>>,
}

impl Store {
    fn values(&mut self) -> &mut BTreeMap<String, String> {
        if self.values.is_none() {
            let loaded = fs::read_to_string(&self.path)
                .ok()
                .and_then(|json| serde_json::from_str::<BTreeMap<String, String>>(&json).ok())
                .unwrap_or_default();
            self.values = Some(loaded);
        }
        self.values.get_or_insert_with(BTreeMap::new)
    }

    fn commit(&mut self) -> std::io::Result<()> {
        let json = serde_json::to_vec(self.values()).map_err(std::io::Error::other)?;
        write_atomically(&self.path, &json)
    }
}

/// Writes `bytes` to `path` so that readers see either the old or the new file.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&temporary, path).inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
    })
}

pub struct Storage {
    store: Arc<Mutex<Store>>,
    worker: SerialWorker,
}

impl Storage {
    /// `directory` is the application's data directory.
    #[must_use]
    pub fn new(directory: &Path) -> Self {
        Self {
            store: Arc::new(Mutex::new(Store { path: directory.join("storage.json"), values: None })),
            worker: SerialWorker::new("pam-storage"),
        }
    }
}

impl Module for Storage {
    fn call(&mut self, method: &str, payload: &[u8], reply: Reply) {
        let request = match request_map(payload) {
            Ok(request) => request,
            Err(error) => return reply.failure(error),
        };
        let Some(key) = request.text("key").map(str::to_owned) else { return reply.failure("Storage key is required") };
        if !valid_key(&key) {
            return reply.failure("Invalid storage key");
        }
        let store = Arc::clone(&self.store);
        match method {
            "get" => self.worker.run(move || {
                let Ok(mut store) = store.lock() else { return reply.failure("Storage module is closed") };
                let mut result = WireMap::new();
                if let Some(value) = store.values().get(&key) {
                    result.insert("value", value.as_str());
                }
                reply.map(&result);
            }),
            "set" => {
                let Some(value) = request.text("value").map(str::to_owned) else { return reply.failure("Storage value is required") };
                if value.len() > MAX_VALUE_BYTES {
                    return reply.failure("Storage value exceeds 256 KiB");
                }
                self.worker.run(move || {
                    let Ok(mut store) = store.lock() else { return reply.failure("Storage module is closed") };
                    store.values().insert(key, value);
                    match store.commit() {
                        Ok(()) => reply.done(),
                        Err(_) => reply.failure("Storage write failed"),
                    }
                });
            }
            _ => reply.failure("Unknown storage method"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_follow_the_sdk_format() {
        assert!(valid_key("session.user-1_token"));
        assert!(!valid_key(""));
        assert!(!valid_key("a/b"));
        assert!(!valid_key(&"k".repeat(129)));
    }

    #[test]
    fn atomic_writes_replace_the_file() {
        let directory = std::env::temp_dir().join(format!("pam-storage-test-{}", std::process::id()));
        let path = directory.join("nested/storage.json");
        write_atomically(&path, b"{\"a\":\"1\"}").unwrap();
        write_atomically(&path, b"{\"a\":\"2\"}").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"a\":\"2\"}");
        let mut store = Store { path: path.clone(), values: None };
        assert_eq!(store.values().get("a").map(String::as_str), Some("2"));
        store.values().insert("b".into(), "x".into());
        store.commit().unwrap();
        let mut reloaded = Store { path, values: None };
        assert_eq!(reloaded.values().len(), 2);
        let _ = fs::remove_dir_all(&directory);
    }
}
