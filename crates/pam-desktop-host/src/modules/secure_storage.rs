//! `secure-storage`: small secrets (tokens, PIN hashes) in the keychain of
//! the system: the Secret Service on Linux, the Keychain on macOS, the
//! Credential Manager on Windows.
//!
//! Each secret is one keychain entry of the application (service = the
//! application id, account = the SHA-256 of the key). Keychains limit how
//! much one entry holds, so a value over [`INLINE_BYTES`] is kept in a file
//! under `<data dir>/secure/`, encrypted with AES-256-GCM under a random key
//! that is itself the keychain entry. A new key and nonce are drawn for every
//! write.
//!
//! When no keychain can be reached (a session without a Secret Service, for
//! instance) every method fails with its usual message; nothing is ever
//! stored in the clear.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use pam_desktop_ui::modules::support::{sha256_hex, uuid};
use pam_desktop_ui::modules::{Module, Reply, SerialWorker, request_map};
use pam_desktop_ui::wiremap::WireMap;

/// The directory of the encrypted values inside the application's data directory.
pub const SECURE_DIRECTORY: &str = "secure";
/// Largest value, in bytes, stored directly in the keychain. Every backend
/// takes this much; the tightest is the Credential Manager of Windows, whose
/// entries hold 2560 bytes of UTF-16.
pub const INLINE_BYTES: usize = 1024;

const MAX_KEY_CHARACTERS: usize = 256;
const MAX_VALUE_CHARACTERS: usize = 65_536;
const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 12;
/// First byte of an encrypted file: the layout version.
const FILE_VERSION: u8 = 1;
/// Keychain entry holding the value itself.
const INLINE_PREFIX: &str = "v:";
/// Keychain entry holding the base64 key of the encrypted file.
const FILE_PREFIX: &str = "k:";

const FAILED: &str = "Secure storage failed";
const NOT_STORED: &str = "Secure value could not be stored";

/// The keychain could not be reached or refused the operation.
#[derive(Debug)]
pub struct Unavailable;

/// Where the keychain entries of one application live.
pub trait SecretStore: Send + Sync {
    /// The secret of `account`, `None` when there is no such entry.
    fn get(&self, account: &str) -> Result<Option<String>, Unavailable>;
    fn set(&self, account: &str, secret: &str) -> Result<(), Unavailable>;
    /// Removes the entry; an entry that does not exist is not an error.
    fn delete(&self, account: &str) -> Result<(), Unavailable>;
}

/// The keychain of the system, through the `keyring` crate.
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
struct Keychain {
    service: String,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl Keychain {
    fn entry(&self, account: &str) -> Result<keyring::Entry, Unavailable> {
        keyring::Entry::new(&self.service, account).map_err(|_| Unavailable)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl SecretStore for Keychain {
    fn get(&self, account: &str) -> Result<Option<String>, Unavailable> {
        match self.entry(account)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(Unavailable),
        }
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), Unavailable> {
        self.entry(account)?
            .set_password(secret)
            .map_err(|_| Unavailable)
    }

    fn delete(&self, account: &str) -> Result<(), Unavailable> {
        match self.entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(Unavailable),
        }
    }
}

/// A system without a keychain this host knows: every operation fails.
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
struct Keychain {
    #[allow(dead_code)]
    service: String,
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
impl SecretStore for Keychain {
    fn get(&self, _: &str) -> Result<Option<String>, Unavailable> {
        Err(Unavailable)
    }

    fn set(&self, _: &str, _: &str) -> Result<(), Unavailable> {
        Err(Unavailable)
    }

    fn delete(&self, _: &str) -> Result<(), Unavailable> {
        Err(Unavailable)
    }
}

/// The keychain account of a key: keys are arbitrary text, accounts are not.
fn account(key: &str) -> String {
    sha256_hex(key.as_bytes())
}

/// Encrypts `value` under `key`; the result is `version · nonce · ciphertext`.
/// The account is authenticated with it, so a file cannot be moved to
/// another key.
fn seal(key: &[u8; KEY_BYTES], account: &str, value: &str) -> Option<Vec<u8>> {
    let mut nonce = [0_u8; NONCE_BYTES];
    getrandom::fill(&mut nonce).ok()?;
    let ciphertext = Aes256Gcm::new_from_slice(key)
        .ok()?
        .encrypt(
            &Nonce::from(nonce),
            Payload {
                msg: value.as_bytes(),
                aad: account.as_bytes(),
            },
        )
        .ok()?;
    let mut sealed = Vec::with_capacity(1 + NONCE_BYTES + ciphertext.len());
    sealed.push(FILE_VERSION);
    sealed.extend_from_slice(&nonce);
    sealed.extend_from_slice(&ciphertext);
    Some(sealed)
}

/// Decrypts what [`seal`] produced; `None` when it was tampered with, is
/// truncated, or belongs to another key or account.
fn open(key: &[u8], account: &str, sealed: &[u8]) -> Option<String> {
    let (version, rest) = sealed.split_first()?;
    if *version != FILE_VERSION || rest.len() < NONCE_BYTES {
        return None;
    }
    let (nonce, ciphertext) = rest.split_at(NONCE_BYTES);
    let nonce = <[u8; NONCE_BYTES]>::try_from(nonce).ok()?;
    // A key of the wrong length is refused here.
    let plaintext = Aes256Gcm::new_from_slice(key)
        .ok()?
        .decrypt(
            &Nonce::from(nonce),
            Payload {
                msg: ciphertext,
                aad: account.as_bytes(),
            },
        )
        .ok()?;
    String::from_utf8(plaintext).ok()
}

/// Creates a file only its owner can read.
fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn create_private_directory(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

/// The keychain and the directory of the encrypted values.
struct Vault {
    store: Box<dyn SecretStore>,
    directory: PathBuf,
}

impl Vault {
    fn file(&self, account: &str) -> PathBuf {
        self.directory.join(format!("{account}.bin"))
    }

    /// The value of `key`: `Ok(None)` when it has none or its stored form
    /// cannot be decrypted any more.
    fn get(&self, key: &str) -> Result<Option<String>, Unavailable> {
        let account = account(key);
        let Some(entry) = self.store.get(&account)? else {
            return Ok(None);
        };
        if let Some(value) = entry.strip_prefix(INLINE_PREFIX) {
            return Ok(Some(value.to_owned()));
        }
        let Some(encoded) = entry.strip_prefix(FILE_PREFIX) else {
            return Ok(None);
        };
        let value = BASE64.decode(encoded).ok().and_then(|secret| {
            let sealed = std::fs::read(self.file(&account)).ok()?;
            open(&secret, &account, &sealed)
        });
        Ok(value)
    }

    fn set(&self, key: &str, value: &str) -> Result<(), Unavailable> {
        let account = account(key);
        if value.len() <= INLINE_BYTES {
            self.store
                .set(&account, &format!("{INLINE_PREFIX}{value}"))?;
            // A previous, larger value is of no use any more.
            let _ = std::fs::remove_file(self.file(&account));
            return Ok(());
        }
        let mut secret = [0_u8; KEY_BYTES];
        getrandom::fill(&mut secret).map_err(|_| Unavailable)?;
        let sealed = seal(&secret, &account, value).ok_or(Unavailable)?;
        // The new file only replaces the old one once its key is in the
        // keychain, so a failure leaves the previous value readable.
        create_private_directory(&self.directory).map_err(|_| Unavailable)?;
        let staged = self.directory.join(format!("{account}.tmp-{}", uuid()));
        let written = create_private(&staged).and_then(|mut file| {
            file.write_all(&sealed)?;
            file.sync_all()
        });
        let stored = written.map_err(|_| Unavailable).and_then(|()| {
            self.store
                .set(&account, &format!("{FILE_PREFIX}{}", BASE64.encode(secret)))
        });
        let published = stored
            .and_then(|()| std::fs::rename(&staged, self.file(&account)).map_err(|_| Unavailable));
        if published.is_err() {
            let _ = std::fs::remove_file(&staged);
        }
        published
    }

    fn delete(&self, key: &str) -> Result<(), Unavailable> {
        let account = account(key);
        self.store.delete(&account)?;
        let _ = std::fs::remove_file(self.file(&account));
        Ok(())
    }
}

/// Secrets held by this process only (`PAM_DESKTOP_SECURE_STORAGE=memory`).
#[derive(Default)]
struct ProcessMemory(std::sync::Mutex<std::collections::HashMap<String, String>>);

impl SecretStore for ProcessMemory {
    fn get(&self, account: &str) -> Result<Option<String>, Unavailable> {
        Ok(self
            .0
            .lock()
            .map_err(|_| Unavailable)?
            .get(account)
            .cloned())
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), Unavailable> {
        self.0
            .lock()
            .map_err(|_| Unavailable)?
            .insert(account.to_owned(), secret.to_owned());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), Unavailable> {
        self.0.lock().map_err(|_| Unavailable)?.remove(account);
        Ok(())
    }
}

pub struct SecureStorage {
    vault: Arc<Vault>,
    worker: SerialWorker,
}

impl SecureStorage {
    /// `data_dir` is the application's data directory and `application_id`
    /// names the application in the keychain of the system.
    #[must_use]
    ///
    /// `PAM_DESKTOP_SECURE_STORAGE=memory` keeps the secrets in this process
    /// instead (for tests and screenshots: the keychain of the machine is
    /// never read or written, and everything is gone when the host exits).
    pub fn new(data_dir: &Path, application_id: &str) -> Self {
        if std::env::var_os("PAM_DESKTOP_SECURE_STORAGE").is_some_and(|value| value == "memory") {
            eprintln!(
                "pam-desktop: PAM_DESKTOP_SECURE_STORAGE=memory: secrets are kept in memory only"
            );
            return Self::with_store(data_dir, Box::new(ProcessMemory::default()));
        }
        Self::with_store(
            data_dir,
            Box::new(Keychain {
                service: application_id.to_owned(),
            }),
        )
    }

    /// The module over another secret store.
    #[must_use]
    pub fn with_store(data_dir: &Path, store: Box<dyn SecretStore>) -> Self {
        Self {
            vault: Arc::new(Vault {
                store,
                directory: data_dir.join(SECURE_DIRECTORY),
            }),
            worker: SerialWorker::new("pam-secure-storage"),
        }
    }
}

impl Module for SecureStorage {
    fn call(&mut self, method: &str, payload: &[u8], reply: Reply) {
        let request = match request_map(payload) {
            Ok(request) => request,
            Err(error) => return reply.failure(error),
        };
        let key = request
            .text("key")
            .filter(|key| !key.is_empty() && key.chars().count() <= MAX_KEY_CHARACTERS)
            .map(str::to_owned);
        let Some(key) = key else {
            return reply.failure("Secure storage key is required");
        };
        let vault = Arc::clone(&self.vault);
        match method {
            // Keychains answer over IPC and may prompt: never on the UI thread.
            "get" => self.worker.run(move || match vault.get(&key) {
                Ok(value) => reply.map(
                    &WireMap::new()
                        .with("found", value.is_some())
                        .with("value", value.unwrap_or_default()),
                ),
                Err(Unavailable) => reply.failure(FAILED),
            }),
            "set" => {
                let Some(value) = request.text("value").map(str::to_owned) else {
                    return reply.failure("Secure storage value is required");
                };
                if value.chars().count() > MAX_VALUE_CHARACTERS {
                    return reply.failure("Secure values are limited to 65536 characters");
                }
                self.worker.run(move || match vault.set(&key, &value) {
                    Ok(()) => reply.done(),
                    Err(Unavailable) => reply.failure(NOT_STORED),
                });
            }
            "delete" => self.worker.run(move || match vault.delete(&key) {
                Ok(()) => reply.done(),
                Err(Unavailable) => reply.failure(FAILED),
            }),
            other => reply.failure(format!("Unknown secure storage method {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pam_desktop_ui::modules::test_support::temporary_directory;
    use pam_desktop_ui::runtime::{MODULE_FAILURE, MODULE_SUCCESS};
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(10);

    /// A keychain in memory that can be made unreachable.
    #[derive(Default)]
    struct MemoryStore {
        entries: Mutex<HashMap<String, String>>,
        unreachable: AtomicBool,
        read_only: AtomicBool,
    }

    impl MemoryStore {
        fn reachable(&self) -> Result<(), Unavailable> {
            if self.unreachable.load(Ordering::Relaxed) {
                Err(Unavailable)
            } else {
                Ok(())
            }
        }
    }

    impl SecretStore for Arc<MemoryStore> {
        fn get(&self, account: &str) -> Result<Option<String>, Unavailable> {
            self.reachable()?;
            Ok(self.entries.lock().unwrap().get(account).cloned())
        }

        fn set(&self, account: &str, secret: &str) -> Result<(), Unavailable> {
            self.reachable()?;
            if self.read_only.load(Ordering::Relaxed) {
                return Err(Unavailable);
            }
            // What the tightest real backend accepts.
            assert!(secret.encode_utf16().count() * 2 <= 2560);
            self.entries
                .lock()
                .unwrap()
                .insert(account.to_owned(), secret.to_owned());
            Ok(())
        }

        fn delete(&self, account: &str) -> Result<(), Unavailable> {
            self.reachable()?;
            self.entries.lock().unwrap().remove(account);
            Ok(())
        }
    }

    #[test]
    fn the_memory_store_keeps_secrets_for_the_process_only() {
        let store = ProcessMemory::default();
        assert_eq!(store.get("a").unwrap(), None);
        store.set("a", "secret").unwrap();
        assert_eq!(store.get("a").unwrap().as_deref(), Some("secret"));
        store.delete("a").unwrap();
        assert_eq!(store.get("a").unwrap(), None);
        // The module over it never reaches the keychain.
        let directory = temporary_directory("secure-memory");
        let mut module = SecureStorage::with_store(&directory, Box::new(ProcessMemory::default()));
        let (reply, answer) = Reply::channel();
        let mut request = WireMap::new();
        request.insert("key", "token");
        request.insert("value", "x".repeat(INLINE_BYTES * 2));
        module.call("set", &request.encode(), reply);
        assert_eq!(answer.recv_timeout(WAIT).unwrap().0, MODULE_SUCCESS);
        let _ = std::fs::remove_dir_all(&directory);
    }

    struct Fixture {
        directory: PathBuf,
        store: Arc<MemoryStore>,
        module: SecureStorage,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let directory = temporary_directory(name);
            let store = Arc::new(MemoryStore::default());
            let module = SecureStorage::with_store(&directory, Box::new(Arc::clone(&store)));
            Self {
                directory,
                store,
                module,
            }
        }

        fn call(&mut self, method: &str, request: &WireMap) -> Result<Vec<u8>, String> {
            let (reply, answer) = Reply::channel();
            self.module.call(method, &request.encode(), reply);
            let (status, payload) = answer.recv_timeout(WAIT).unwrap();
            if status == MODULE_SUCCESS {
                Ok(payload)
            } else {
                assert_eq!(status, MODULE_FAILURE);
                Err(String::from_utf8(payload).unwrap())
            }
        }

        fn get(&mut self, key: &str) -> Option<String> {
            let payload = self.call("get", &WireMap::new().with("key", key)).unwrap();
            // PHP decodes this result unconditionally: it is never empty.
            let result = WireMap::decode(&payload).unwrap();
            let value = result.text("value").unwrap().to_owned();
            if result.boolean("found").unwrap() {
                Some(value)
            } else {
                assert_eq!(value, "");
                None
            }
        }

        fn set(&mut self, key: &str, value: &str) -> Result<Vec<u8>, String> {
            self.call("set", &WireMap::new().with("key", key).with("value", value))
        }

        fn delete(&mut self, key: &str) -> Result<Vec<u8>, String> {
            self.call("delete", &WireMap::new().with("key", key))
        }

        fn files(&self) -> usize {
            std::fs::read_dir(self.directory.join(SECURE_DIRECTORY)).map_or(0, Iterator::count)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn small_values_live_in_the_keychain() {
        let mut fixture = Fixture::new("secure-inline");
        assert_eq!(fixture.get("session.token"), None);
        assert_eq!(fixture.set("session.token", "s3cr€t"), Ok(Vec::new()));
        assert_eq!(fixture.get("session.token").as_deref(), Some("s3cr€t"));
        assert_eq!(fixture.set("session.token", ""), Ok(Vec::new()));
        assert_eq!(fixture.get("session.token").as_deref(), Some(""));
        // Keys are arbitrary text; the keychain account is their digest.
        assert_eq!(fixture.set("chave com espaço/é", "v"), Ok(Vec::new()));
        assert_eq!(fixture.get("chave com espaço/é").as_deref(), Some("v"));
        let entries = fixture.store.entries.lock().unwrap().clone();
        assert_eq!(entries.len(), 2);
        assert!(
            entries.keys().all(|account| account.len() == 64
                && account.bytes().all(|byte| byte.is_ascii_hexdigit()))
        );
        assert_eq!(entries[&account("chave com espaço/é")], "v:v");
        assert_eq!(fixture.files(), 0);

        assert_eq!(fixture.delete("session.token"), Ok(Vec::new()));
        assert_eq!(fixture.get("session.token"), None);
        // Deleting what is not there succeeds.
        assert_eq!(fixture.delete("session.token"), Ok(Vec::new()));
        assert_eq!(fixture.delete("never-set"), Ok(Vec::new()));
    }

    #[test]
    fn large_values_are_encrypted_on_disk_under_a_keychain_key() {
        let mut fixture = Fixture::new("secure-file");
        let large = "segredo-".repeat(2000);
        assert_eq!(fixture.set("blob", &large), Ok(Vec::new()));
        assert_eq!(fixture.get("blob").as_deref(), Some(large.as_str()));
        assert_eq!(fixture.files(), 1);
        let file = fixture
            .directory
            .join(SECURE_DIRECTORY)
            .join(format!("{}.bin", account("blob")));
        let sealed = std::fs::read(&file).unwrap();
        // Version, nonce, ciphertext and tag; nothing readable.
        assert_eq!(sealed.len(), 1 + NONCE_BYTES + large.len() + 16);
        assert!(!sealed.windows(8).any(|window| window == b"segredo-"));
        let entry = fixture.store.entries.lock().unwrap()[&account("blob")].clone();
        assert_eq!(
            BASE64
                .decode(entry.strip_prefix("k:").unwrap())
                .unwrap()
                .len(),
            KEY_BYTES
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        // Writing again draws a new key and nonce.
        assert_eq!(fixture.set("blob", &large), Ok(Vec::new()));
        assert_ne!(
            fixture.store.entries.lock().unwrap()[&account("blob")],
            entry
        );
        assert_ne!(std::fs::read(&file).unwrap(), sealed);
        assert_eq!(fixture.get("blob").as_deref(), Some(large.as_str()));

        // A small value takes the file away, and so does deleting.
        assert_eq!(fixture.set("blob", "small"), Ok(Vec::new()));
        assert_eq!(fixture.files(), 0);
        assert_eq!(fixture.get("blob").as_deref(), Some("small"));
        assert_eq!(fixture.set("blob", &large), Ok(Vec::new()));
        assert_eq!(fixture.delete("blob"), Ok(Vec::new()));
        assert_eq!(fixture.files(), 0);
        assert_eq!(fixture.get("blob"), None);
    }

    #[test]
    fn the_inline_threshold_is_in_bytes() {
        let mut fixture = Fixture::new("secure-threshold");
        assert_eq!(fixture.set("a", &"x".repeat(INLINE_BYTES)), Ok(Vec::new()));
        assert_eq!(fixture.files(), 0);
        assert_eq!(
            fixture.set("b", &"x".repeat(INLINE_BYTES + 1)),
            Ok(Vec::new())
        );
        assert_eq!(fixture.files(), 1);
        // 400 characters, 1200 bytes.
        assert_eq!(fixture.set("c", &"€".repeat(400)), Ok(Vec::new()));
        assert_eq!(fixture.files(), 2);
        assert_eq!(fixture.get("c").as_deref(), Some("€".repeat(400).as_str()));
    }

    #[test]
    fn values_that_cannot_be_decrypted_are_not_found() {
        let mut fixture = Fixture::new("secure-tamper");
        let large = "x".repeat(5000);
        fixture.set("a", &large).unwrap();
        fixture.set("b", &large).unwrap();
        let directory = fixture.directory.join(SECURE_DIRECTORY);
        let file = |key: &str| directory.join(format!("{}.bin", account(key)));

        // A flipped bit fails authentication.
        let mut sealed = std::fs::read(file("a")).unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        std::fs::write(file("a"), &sealed).unwrap();
        assert_eq!(fixture.get("a"), None);
        // The file of one key does not open under another.
        std::fs::copy(file("b"), file("a")).unwrap();
        assert_eq!(fixture.get("a"), None);
        assert_eq!(fixture.get("b").as_deref(), Some(large.as_str()));
        std::fs::remove_file(file("b")).unwrap();
        assert_eq!(fixture.get("b"), None);
        // An entry of an unknown form is not a value.
        fixture
            .store
            .entries
            .lock()
            .unwrap()
            .insert(account("c"), String::from("plain"));
        assert_eq!(fixture.get("c"), None);
    }

    #[test]
    fn sealing_is_bound_to_the_key_and_the_account() {
        let key = [7_u8; KEY_BYTES];
        let sealed = seal(&key, "account", "valor").unwrap();
        assert_eq!(open(&key, "account", &sealed).as_deref(), Some("valor"));
        assert_ne!(seal(&key, "account", "valor").unwrap(), sealed);
        assert_eq!(open(&[8_u8; KEY_BYTES], "account", &sealed), None);
        assert_eq!(open(&key, "other", &sealed), None);
        assert_eq!(open(&key[..16], "account", &sealed), None);
        assert_eq!(open(&key, "account", &sealed[..sealed.len() - 1]), None);
        assert_eq!(open(&key, "account", &[]), None);
        assert_eq!(open(&key, "account", &[FILE_VERSION, 1, 2]), None);
        let mut other_version = sealed.clone();
        other_version[0] = 2;
        assert_eq!(open(&key, "account", &other_version), None);
    }

    #[test]
    fn an_unreachable_keychain_fails_with_the_sdk_messages() {
        let mut fixture = Fixture::new("secure-unreachable");
        fixture.set("token", "kept").unwrap();
        fixture.store.unreachable.store(true, Ordering::Relaxed);
        assert_eq!(
            fixture.call("get", &WireMap::new().with("key", "token")),
            Err(FAILED.into())
        );
        assert_eq!(fixture.set("token", "new"), Err(NOT_STORED.into()));
        assert_eq!(
            fixture.set("token", &"x".repeat(4000)),
            Err(NOT_STORED.into())
        );
        assert_eq!(fixture.delete("token"), Err(FAILED.into()));
        // Nothing was written in the meantime.
        assert_eq!(fixture.files(), 0);
        fixture.store.unreachable.store(false, Ordering::Relaxed);
        assert_eq!(fixture.get("token").as_deref(), Some("kept"));
    }

    #[test]
    fn a_failed_write_keeps_the_previous_value() {
        let mut fixture = Fixture::new("secure-keep");
        let first = "a".repeat(3000);
        fixture.set("doc", &first).unwrap();
        fixture.store.read_only.store(true, Ordering::Relaxed);
        assert_eq!(
            fixture.set("doc", &"b".repeat(3000)),
            Err(NOT_STORED.into())
        );
        assert_eq!(fixture.get("doc").as_deref(), Some(first.as_str()));
        // The staged file of the failed write is gone.
        assert_eq!(fixture.files(), 1);
    }

    #[test]
    fn requests_are_validated() {
        let mut fixture = Fixture::new("secure-validation");
        const KEY: &str = "Secure storage key is required";
        assert_eq!(fixture.call("get", &WireMap::new()), Err(KEY.into()));
        assert_eq!(
            fixture.call("get", &WireMap::new().with("key", "")),
            Err(KEY.into())
        );
        assert_eq!(
            fixture.call("get", &WireMap::new().with("key", 5)),
            Err(KEY.into())
        );
        assert_eq!(
            fixture.call("get", &WireMap::new().with("key", "k".repeat(257))),
            Err(KEY.into())
        );
        // The limit counts characters, not bytes.
        assert_eq!(fixture.set(&"é".repeat(256), "v"), Ok(Vec::new()));
        assert_eq!(
            fixture.call("set", &WireMap::new().with("key", "k")),
            Err("Secure storage value is required".into())
        );
        assert_eq!(
            fixture.set("k", &"v".repeat(65_537)),
            Err("Secure values are limited to 65536 characters".into())
        );
        assert_eq!(fixture.set("k", &"v".repeat(65_536)), Ok(Vec::new()));
        assert_eq!(fixture.get("k").map(|value| value.len()), Some(65_536));
        assert_eq!(
            fixture.call("list", &WireMap::new().with("key", "k")),
            Err("Unknown secure storage method list".into())
        );
    }

    /// Run it on purpose (`--ignored`): it talks to the keychain of the
    /// system, which may ask the user to unlock it. With
    /// `DBUS_SESSION_BUS_ADDRESS` pointing nowhere and
    /// `PAM_DESKTOP_EXPECT_NO_KEYCHAIN` set, it checks what a session without
    /// a Secret Service answers.
    #[test]
    #[ignore = "talks to the keychain of the system"]
    fn the_system_keychain_never_panics_without_a_service() {
        // Without a reachable keychain this must be a failure message; with
        // one, the entry simply does not exist.
        let directory = temporary_directory("secure-system");
        let mut module = SecureStorage::new(&directory, "dev.pam.desktop.tests");
        let mut call = |method: &str, request: &WireMap| {
            let (reply, answer) = Reply::channel();
            module.call(method, &request.encode(), reply);
            answer.recv_timeout(Duration::from_secs(60)).unwrap()
        };
        let request = WireMap::new().with("key", format!("missing-{}", uuid()));
        let (status, payload) = call("get", &request);
        if status == MODULE_SUCCESS {
            let result = WireMap::decode(&payload).unwrap();
            assert_eq!(result.boolean("found"), Some(false));
        } else {
            assert_eq!(payload, FAILED.as_bytes());
            // Nothing can be stored or removed either, small or large.
            for value in ["small".to_owned(), "x".repeat(INLINE_BYTES + 1)] {
                assert_eq!(
                    call("set", &request.clone().with("value", value)),
                    (MODULE_FAILURE, NOT_STORED.as_bytes().to_vec())
                );
            }
            assert_eq!(
                call("delete", &request),
                (MODULE_FAILURE, FAILED.as_bytes().to_vec())
            );
            assert_eq!(
                std::fs::read_dir(directory.join(SECURE_DIRECTORY)).map_or(0, Iterator::count),
                0
            );
        }
        // Set by a run that pointed the session bus nowhere on purpose.
        if std::env::var_os("PAM_DESKTOP_EXPECT_NO_KEYCHAIN").is_some() {
            assert_eq!(status, MODULE_FAILURE);
        }
        let _ = std::fs::remove_dir_all(&directory);
    }
}
