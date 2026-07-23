use std::cmp::Ordering;
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use getrandom::fill;
use pam_desktop_protocol::{
    ApplicationManifest, ErrorCode, PROTOCOL_VERSION, SignedUpdateRelease, UPDATE_FEED_VERSION,
    UpdateArtifact, UpdateArtifactKind, UpdateConfig, UpdatePlatform, UpdatePolicy, UpdateRelease,
    UpdateState,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_FEED_BYTES: u64 = 1024 * 1024;
const MAX_BUNDLE_MANIFEST_BYTES: u64 = 10 * 1024 * 1024;
const NETWORK_TIMEOUT: Duration = Duration::from_secs(30);
const PARENT_EXIT_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone)]
pub struct Updater {
    inner: Arc<UpdaterInner>,
}

struct UpdaterInner {
    application_id: String,
    current_version: String,
    config: Option<UpdateConfig>,
    bundle_root: Option<PathBuf>,
    state: Mutex<InternalState>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSnapshot {
    pub state: UpdateState,
    pub current_version: String,
    pub available_version: Option<String>,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub notes_url: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
struct Candidate {
    artifact: UpdateArtifact,
    version: String,
    notes_url: Option<String>,
}

struct InternalState {
    snapshot: UpdateSnapshot,
    candidate: Option<Candidate>,
    archive: Option<PathBuf>,
}

#[derive(Debug)]
pub struct UpdateError {
    pub code: ErrorCode,
    pub message: String,
}

impl UpdateError {
    fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl Updater {
    #[must_use]
    pub fn prepare(manifest: &ApplicationManifest) -> Self {
        let state = if manifest.updates.is_some() {
            UpdateState::Idle
        } else {
            UpdateState::Disabled
        };
        Self {
            inner: Arc::new(UpdaterInner {
                application_id: manifest.identifier.clone(),
                current_version: manifest.version.clone(),
                config: manifest.updates.clone(),
                bundle_root: discover_bundle_root(),
                state: Mutex::new(InternalState {
                    snapshot: UpdateSnapshot {
                        state,
                        current_version: manifest.version.clone(),
                        available_version: None,
                        downloaded_bytes: 0,
                        total_bytes: 0,
                        notes_url: None,
                        error: None,
                    },
                    candidate: None,
                    archive: None,
                }),
            }),
        }
    }

    #[must_use]
    pub fn policy(&self) -> Option<UpdatePolicy> {
        self.inner.config.as_ref().map(|config| config.policy)
    }

    #[must_use]
    pub fn snapshot(&self) -> UpdateSnapshot {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .snapshot
            .clone()
    }

    /// Fetches and verifies the configured release feed.
    ///
    /// # Errors
    ///
    /// Returns a typed update error when updates are disabled, the transport
    /// fails, or the signed feed does not match this application and target.
    pub fn check(&self) -> Result<UpdateSnapshot, UpdateError> {
        let config = self.config()?.clone();
        self.begin(UpdateState::Checking)?;
        let result = self.check_inner(&config);
        match result {
            Ok(candidate) => {
                let mut state = self.lock_state();
                state.archive = None;
                state.snapshot.error = None;
                state.snapshot.downloaded_bytes = 0;
                if let Some(candidate) = candidate {
                    state.snapshot.state = UpdateState::Available;
                    state.snapshot.available_version = Some(candidate.version.clone());
                    state.snapshot.total_bytes = candidate.artifact.bytes;
                    state.snapshot.notes_url.clone_from(&candidate.notes_url);
                    state.candidate = Some(candidate);
                } else {
                    state.snapshot.state = UpdateState::UpToDate;
                    state.snapshot.available_version = None;
                    state.snapshot.total_bytes = 0;
                    state.snapshot.notes_url = None;
                    state.candidate = None;
                }
                Ok(state.snapshot.clone())
            }
            Err(error) => {
                self.fail(&error);
                Err(error)
            }
        }
    }

    /// Downloads the selected portable archive and verifies its exact signed
    /// byte length and SHA-256 digest.
    ///
    /// # Errors
    ///
    /// Returns a typed update error when there is no available release,
    /// storage cannot be prepared, or integrity verification fails.
    pub fn download(&self) -> Result<UpdateSnapshot, UpdateError> {
        let candidate = {
            let state = self.lock_state();
            state.candidate.clone().ok_or_else(|| {
                UpdateError::new(
                    ErrorCode::UpdateUnavailable,
                    "No update is available. Check for updates first.",
                )
            })?
        };
        self.begin(UpdateState::Downloading)?;
        let result = self.download_inner(&candidate);
        match result {
            Ok(archive) => {
                let mut state = self.lock_state();
                state.archive = Some(archive);
                state.snapshot.state = UpdateState::Ready;
                state.snapshot.downloaded_bytes = candidate.artifact.bytes;
                state.snapshot.error = None;
                Ok(state.snapshot.clone())
            }
            Err(error) => {
                self.fail(&error);
                Err(error)
            }
        }
    }

    /// Starts the detached swap helper. The host must exit immediately after
    /// this method succeeds so Windows can replace locked executables.
    ///
    /// # Errors
    ///
    /// Returns a typed update error when no package is staged or this process
    /// is not running from a writable Pam Desktop bundle.
    pub fn install(&self) -> Result<UpdateSnapshot, UpdateError> {
        let (archive, candidate) = {
            let state = self.lock_state();
            let archive = state.archive.clone().ok_or_else(|| {
                UpdateError::new(
                    ErrorCode::UpdateUnavailable,
                    "No verified update has been downloaded.",
                )
            })?;
            let candidate = state.candidate.clone().ok_or_else(|| {
                UpdateError::new(
                    ErrorCode::UpdateUnavailable,
                    "The staged update has no matching release metadata.",
                )
            })?;
            (archive, candidate)
        };
        let bundle_root = self.inner.bundle_root.clone().ok_or_else(|| {
            UpdateError::new(
                ErrorCode::UpdateInstallFailed,
                "Updates can only be installed from a packaged Pam Desktop application.",
            )
        })?;
        self.begin(UpdateState::Applying)?;
        if let Err(error) = launch_apply_helper(
            &bundle_root,
            &archive,
            &candidate.artifact.sha256,
            std::process::id(),
        ) {
            self.fail(&error);
            return Err(error);
        }
        Ok(self.snapshot())
    }

    fn check_inner(&self, config: &UpdateConfig) -> Result<Option<Candidate>, UpdateError> {
        let bytes = fetch_small(&config.endpoint, MAX_FEED_BYTES)?;
        let feed: SignedUpdateRelease = serde_json::from_slice(&bytes).map_err(|error| {
            UpdateError::new(
                ErrorCode::UpdateIntegrityFailed,
                format!("The update feed is not valid JSON: {error}"),
            )
        })?;
        verify_signed_feed(&feed, &config.public_key)?;
        if feed.release.application_id != self.inner.application_id {
            return Err(UpdateError::new(
                ErrorCode::UpdateIntegrityFailed,
                "The signed release belongs to a different application.",
            ));
        }
        if feed.release.channel != config.channel {
            return Err(UpdateError::new(
                ErrorCode::UpdateIntegrityFailed,
                "The signed release belongs to a different update channel.",
            ));
        }
        if !version_is_newer(&feed.release.version, &self.inner.current_version) {
            return Ok(None);
        }
        let platform = current_platform().ok_or_else(|| {
            UpdateError::new(
                ErrorCode::UpdateUnavailable,
                "This operating system has no Pam Desktop update target.",
            )
        })?;
        let artifact = feed
            .release
            .artifacts
            .into_iter()
            .find(|artifact| {
                artifact.platform == platform
                    && artifact.architecture == std::env::consts::ARCH
                    && artifact.kind == UpdateArtifactKind::PortableArchive
            })
            .ok_or_else(|| {
                UpdateError::new(
                    ErrorCode::UpdateUnavailable,
                    format!(
                        "Release {} has no portable artifact for {}/{}.",
                        feed.release.version,
                        std::env::consts::OS,
                        std::env::consts::ARCH
                    ),
                )
            })?;
        Ok(Some(Candidate {
            artifact,
            version: feed.release.version,
            notes_url: feed.release.notes_url,
        }))
    }

    fn download_inner(&self, candidate: &Candidate) -> Result<PathBuf, UpdateError> {
        let stage = update_stage(&self.inner.application_id, &candidate.version)?;
        let archive = stage.join(archive_name());
        let mut response = http_agent(&candidate.artifact.url)
            .get(&candidate.artifact.url)
            .call()
            .map_err(|error| {
                UpdateError::new(
                    ErrorCode::UpdateUnavailable,
                    format!("Cannot download the update artifact: {error}"),
                )
            })?;
        if let Some(length) = response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            && length != candidate.artifact.bytes
        {
            return Err(UpdateError::new(
                ErrorCode::UpdateIntegrityFailed,
                "The update server returned an unexpected artifact size.",
            ));
        }

        let mut reader = response
            .body_mut()
            .with_config()
            .limit(candidate.artifact.bytes.saturating_add(1))
            .reader();
        let mut output = File::create(&archive).map_err(|error| {
            UpdateError::new(
                ErrorCode::UpdateInstallFailed,
                format!("Cannot create update staging file: {error}"),
            )
        })?;
        let mut digest = Sha256::new();
        let mut total = 0_u64;
        let mut buffer = vec![0_u8; 128 * 1024].into_boxed_slice();
        loop {
            let read = reader.read(&mut buffer).map_err(|error| {
                UpdateError::new(
                    ErrorCode::UpdateUnavailable,
                    format!("The update download was interrupted: {error}"),
                )
            })?;
            if read == 0 {
                break;
            }
            total = total.saturating_add(read as u64);
            if total > candidate.artifact.bytes {
                return Err(UpdateError::new(
                    ErrorCode::UpdateIntegrityFailed,
                    "The downloaded update is larger than its signed size.",
                ));
            }
            output.write_all(&buffer[..read]).map_err(|error| {
                UpdateError::new(
                    ErrorCode::UpdateInstallFailed,
                    format!("Cannot write the update staging file: {error}"),
                )
            })?;
            digest.update(&buffer[..read]);
            self.lock_state().snapshot.downloaded_bytes = total;
        }
        output.sync_all().map_err(|error| {
            UpdateError::new(
                ErrorCode::UpdateInstallFailed,
                format!("Cannot flush the update staging file: {error}"),
            )
        })?;
        if total != candidate.artifact.bytes
            || format!("{:x}", digest.finalize()) != candidate.artifact.sha256
        {
            return Err(UpdateError::new(
                ErrorCode::UpdateIntegrityFailed,
                "The downloaded update does not match its signed size and SHA-256 digest.",
            ));
        }
        Ok(archive)
    }

    fn config(&self) -> Result<&UpdateConfig, UpdateError> {
        self.inner.config.as_ref().ok_or_else(|| {
            UpdateError::new(
                ErrorCode::UpdateDisabled,
                "Updates are not configured for this application.",
            )
        })
    }

    fn begin(&self, next: UpdateState) -> Result<(), UpdateError> {
        let mut state = self.lock_state();
        if matches!(
            state.snapshot.state,
            UpdateState::Checking | UpdateState::Downloading | UpdateState::Applying
        ) {
            return Err(UpdateError::new(
                ErrorCode::UpdateUnavailable,
                "Another update operation is already running.",
            ));
        }
        state.snapshot.state = next;
        state.snapshot.error = None;
        if next == UpdateState::Downloading {
            state.snapshot.downloaded_bytes = 0;
        }
        Ok(())
    }

    fn fail(&self, error: &UpdateError) {
        let mut state = self.lock_state();
        state.snapshot.state = UpdateState::Failed;
        state.snapshot.error = Some(error.message.clone());
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, InternalState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn verify_signed_feed(feed: &SignedUpdateRelease, public_key: &str) -> Result<(), UpdateError> {
    feed.validate().map_err(|error| {
        UpdateError::new(
            ErrorCode::UpdateIntegrityFailed,
            format!("The update feed contract is invalid: {error}"),
        )
    })?;
    let public_key = decode_hex::<32>(public_key, "update public key")?;
    let signature = decode_hex::<64>(&feed.signature, "update signature")?;
    let verifying_key = VerifyingKey::from_bytes(&public_key).map_err(|_| {
        UpdateError::new(
            ErrorCode::UpdateIntegrityFailed,
            "The configured Ed25519 update public key is invalid.",
        )
    })?;
    let signature = Signature::from_slice(&signature).map_err(|_| {
        UpdateError::new(
            ErrorCode::UpdateIntegrityFailed,
            "The update feed signature is invalid.",
        )
    })?;
    let payload = serde_json::to_vec(&feed.release).map_err(|error| {
        UpdateError::new(
            ErrorCode::UpdateIntegrityFailed,
            format!("Cannot canonicalize the signed release: {error}"),
        )
    })?;
    verifying_key
        .verify_strict(&payload, &signature)
        .map_err(|_| {
            UpdateError::new(
                ErrorCode::UpdateIntegrityFailed,
                "The update feed signature did not verify.",
            )
        })
}

fn decode_hex<const N: usize>(value: &str, label: &str) -> Result<[u8; N], UpdateError> {
    if value.len() != N * 2 {
        return Err(UpdateError::new(
            ErrorCode::UpdateIntegrityFailed,
            format!("The {label} has an invalid length."),
        ));
    }
    let mut decoded = [0_u8; N];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_nibble(pair[0]).ok_or_else(|| {
            UpdateError::new(
                ErrorCode::UpdateIntegrityFailed,
                format!("The {label} is not lowercase hexadecimal."),
            )
        })?;
        let low = hex_nibble(pair[1]).ok_or_else(|| {
            UpdateError::new(
                ErrorCode::UpdateIntegrityFailed,
                format!("The {label} is not lowercase hexadecimal."),
            )
        })?;
        decoded[index] = high << 4 | low;
    }
    Ok(decoded)
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn fetch_small(url: &str, limit: u64) -> Result<Vec<u8>, UpdateError> {
    let mut response = http_agent(url).get(url).call().map_err(|error| {
        UpdateError::new(
            ErrorCode::UpdateUnavailable,
            format!("Cannot fetch the update feed: {error}"),
        )
    })?;
    response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|error| {
            UpdateError::new(
                ErrorCode::UpdateIntegrityFailed,
                format!("Cannot read the bounded update feed: {error}"),
            )
        })
}

fn http_agent(url: &str) -> ureq::Agent {
    let local_http = url.starts_with("http://127.0.0.1")
        || url.starts_with("http://localhost")
        || url.starts_with("http://[::1]");
    ureq::Agent::config_builder()
        .https_only(!local_http)
        .max_redirects(3)
        .max_redirects_will_error(true)
        .timeout_global(Some(NETWORK_TIMEOUT))
        .user_agent(concat!("pam-desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

fn current_platform() -> Option<UpdatePlatform> {
    match std::env::consts::OS {
        "linux" => Some(UpdatePlatform::Linux),
        "windows" => Some(UpdatePlatform::Windows),
        "macos" => Some(UpdatePlatform::MacOs),
        _ => None,
    }
}

fn version_is_newer(candidate: &str, current: &str) -> bool {
    compare_versions(candidate, current) == Ordering::Greater
}

fn compare_versions(left: &str, right: &str) -> Ordering {
    let left = left.split_once('+').map_or(left, |(version, _)| version);
    let right = right.split_once('+').map_or(right, |(version, _)| version);
    let (left_core, left_pre) = left
        .split_once('-')
        .map_or((left, None), |(core, pre)| (core, Some(pre)));
    let (right_core, right_pre) = right
        .split_once('-')
        .map_or((right, None), |(core, pre)| (core, Some(pre)));
    let left_parts = left_core.split('.').collect::<Vec<_>>();
    let right_parts = right_core.split('.').collect::<Vec<_>>();
    for index in 0..left_parts.len().max(right_parts.len()) {
        let left = left_parts.get(index).copied().unwrap_or("0");
        let right = right_parts.get(index).copied().unwrap_or("0");
        let order = compare_version_part(left, right);
        if order != Ordering::Equal {
            return order;
        }
    }
    match (left_pre, right_pre) {
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left), Some(right)) => compare_prerelease(left, right),
        (None, None) => Ordering::Equal,
    }
}

fn compare_version_part(left: &str, right: &str) -> Ordering {
    match (left.parse::<u64>(), right.parse::<u64>()) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        _ => left.cmp(right),
    }
}

fn compare_prerelease(left: &str, right: &str) -> Ordering {
    let left = left.split('.').collect::<Vec<_>>();
    let right = right.split('.').collect::<Vec<_>>();
    for index in 0..left.len().max(right.len()) {
        match (left.get(index), right.get(index)) {
            (Some(left), Some(right)) => {
                let order = match (left.parse::<u64>(), right.parse::<u64>()) {
                    (Ok(left), Ok(right)) => left.cmp(&right),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => left.cmp(right),
                };
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(_), None) => return Ordering::Greater,
            (None, Some(_)) => return Ordering::Less,
            (None, None) => return Ordering::Equal,
        }
    }
    Ordering::Equal
}

fn discover_bundle_root() -> Option<PathBuf> {
    let configured = std::env::var_os("PAM_DESKTOP_UPDATE_ROOT")
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var_os("PAM_DESKTOP_BUNDLE_ROOT").filter(|value| !value.is_empty()))
        .map(PathBuf::from);
    let inferred = std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent()?.parent().map(Path::to_path_buf));
    configured
        .or(inferred)
        .and_then(|root| root.canonicalize().ok())
        .filter(|root| {
            root.join("manifest.json").is_file()
                || root
                    .join("Contents/Resources/runtime/manifest.json")
                    .is_file()
        })
}

fn update_stage(application_id: &str, version: &str) -> Result<PathBuf, UpdateError> {
    let parent = discover_bundle_root()
        .and_then(|root| root.parent().map(Path::to_path_buf))
        .unwrap_or_else(std::env::temp_dir);
    let mut entropy = [0_u8; 8];
    fill(&mut entropy).map_err(|error| {
        UpdateError::new(
            ErrorCode::UpdateInstallFailed,
            format!("Cannot create secure update staging entropy: {error}"),
        )
    })?;
    let suffix = encode_hex(&entropy);
    let stage = parent.join(format!(
        ".pam-update-{}-{}-{suffix}",
        safe_path_segment(application_id),
        safe_path_segment(version)
    ));
    fs::create_dir(&stage).map_err(|error| {
        UpdateError::new(
            ErrorCode::UpdateInstallFailed,
            format!("Cannot create update staging directory: {error}"),
        )
    })?;
    Ok(stage)
}

fn safe_path_segment(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}

const fn archive_name() -> &'static str {
    if cfg!(target_os = "linux") {
        "update.tar.gz"
    } else {
        "update.zip"
    }
}

fn launch_apply_helper(
    bundle_root: &Path,
    archive: &Path,
    sha256: &str,
    parent_pid: u32,
) -> Result<(), UpdateError> {
    let current_exe = std::env::current_exe().map_err(|error| {
        UpdateError::new(
            ErrorCode::UpdateInstallFailed,
            format!("Cannot locate the Pam Desktop host: {error}"),
        )
    })?;
    let stage = archive.parent().ok_or_else(|| {
        UpdateError::new(
            ErrorCode::UpdateInstallFailed,
            "The update staging path has no parent.",
        )
    })?;
    let helper = stage.join(if cfg!(windows) {
        "pam-desktop-update.exe"
    } else {
        "pam-desktop-update"
    });
    fs::copy(&current_exe, &helper).map_err(|error| {
        UpdateError::new(
            ErrorCode::UpdateInstallFailed,
            format!("Cannot prepare the detached update helper: {error}"),
        )
    })?;
    let launcher = std::env::var_os("PAM_DESKTOP_LAUNCHER")
        .map(PathBuf::from)
        .and_then(|path| path.canonicalize().ok())
        .and_then(|path| path.strip_prefix(bundle_root).ok().map(Path::to_path_buf));
    let mut command = Command::new(&helper);
    command
        .arg("apply-update")
        .arg("--parent")
        .arg(parent_pid.to_string())
        .arg("--bundle")
        .arg(bundle_root)
        .arg("--archive")
        .arg(archive)
        .arg("--sha256")
        .arg(sha256)
        .current_dir(bundle_root.parent().unwrap_or(bundle_root))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(launcher) = launcher {
        command.arg("--launcher").arg(launcher);
    }
    command.spawn().map_err(|error| {
        UpdateError::new(
            ErrorCode::UpdateInstallFailed,
            format!("Cannot start the detached update helper: {error}"),
        )
    })?;
    Ok(())
}

pub struct ApplyOptions {
    parent_pid: u32,
    bundle: PathBuf,
    archive: PathBuf,
    sha256: String,
    launcher: Option<PathBuf>,
}

pub struct PublishOptions {
    private_key: PathBuf,
    output: PathBuf,
    channel: String,
    published_at: String,
    notes_url: Option<String>,
    artifacts: Vec<PublishArtifact>,
    force: bool,
}

struct PublishArtifact {
    platform: UpdatePlatform,
    architecture: String,
    kind: UpdateArtifactKind,
    path: PathBuf,
    url: String,
}

impl PublishOptions {
    /// Parses deterministic signed-feed publication inputs.
    ///
    /// # Errors
    ///
    /// Returns an error for missing fields, malformed target tuples, duplicate
    /// scalar options, or unknown arguments.
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut private_key = None;
        let mut output = None;
        let mut channel = None;
        let mut published_at = None;
        let mut notes_url = None;
        let mut artifacts = Vec::new();
        let mut force = false;
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some("--key") => set_path_option(
                    &mut private_key,
                    arguments.next(),
                    "--key requires a private key file",
                    "--key may only be specified once",
                )?,
                Some("--output" | "-o") => set_path_option(
                    &mut output,
                    arguments.next(),
                    "--output requires a feed path",
                    "--output may only be specified once",
                )?,
                Some("--channel") => set_string_option(
                    &mut channel,
                    arguments.next(),
                    "--channel requires a value",
                    "--channel may only be specified once",
                )?,
                Some("--published-at") => set_string_option(
                    &mut published_at,
                    arguments.next(),
                    "--published-at requires an RFC 3339 timestamp",
                    "--published-at may only be specified once",
                )?,
                Some("--notes-url") => set_string_option(
                    &mut notes_url,
                    arguments.next(),
                    "--notes-url requires an HTTPS URL",
                    "--notes-url may only be specified once",
                )?,
                Some("--artifact") => {
                    let value = arguments
                        .next()
                        .ok_or_else(|| "--artifact requires a target tuple".to_owned())?;
                    artifacts.push(parse_publish_artifact(&value)?);
                }
                Some("--force") => force = true,
                Some(value) => return Err(format!("unknown publish-update option {value:?}")),
                None => return Err("publish-update options must be valid UTF-8".to_owned()),
            }
        }
        if artifacts.is_empty() {
            return Err("publish-update requires at least one --artifact tuple".to_owned());
        }
        Ok(Self {
            private_key: private_key.ok_or_else(|| "--key is required".to_owned())?,
            output: output.ok_or_else(|| "--output is required".to_owned())?,
            channel: channel.unwrap_or_else(|| "stable".to_owned()),
            published_at: published_at.ok_or_else(|| "--published-at is required".to_owned())?,
            notes_url,
            artifacts,
            force,
        })
    }
}

fn set_path_option(
    target: &mut Option<PathBuf>,
    value: Option<OsString>,
    missing: &str,
    duplicate: &str,
) -> Result<(), String> {
    let value = value.ok_or_else(|| missing.to_owned())?;
    if target.replace(PathBuf::from(value)).is_some() {
        return Err(duplicate.to_owned());
    }
    Ok(())
}

fn set_string_option(
    target: &mut Option<String>,
    value: Option<OsString>,
    missing: &str,
    duplicate: &str,
) -> Result<(), String> {
    let value = value
        .ok_or_else(|| missing.to_owned())?
        .into_string()
        .map_err(|_| missing.to_owned())?;
    if target.replace(value).is_some() {
        return Err(duplicate.to_owned());
    }
    Ok(())
}

fn parse_publish_artifact(value: &OsStr) -> Result<PublishArtifact, String> {
    let value = value
        .to_str()
        .ok_or_else(|| "artifact tuple must be valid UTF-8".to_owned())?;
    let fields = value.splitn(5, ',').collect::<Vec<_>>();
    if fields.len() != 5 || fields.iter().any(|field| field.is_empty()) {
        return Err(
            "artifact tuple must be platform,architecture,kind,path,url (kind: portable or installer)"
                .to_owned(),
        );
    }
    let platform = match fields[0] {
        "linux" => UpdatePlatform::Linux,
        "windows" => UpdatePlatform::Windows,
        "macos" => UpdatePlatform::MacOs,
        value => return Err(format!("unknown update artifact platform {value:?}")),
    };
    let kind = match fields[2] {
        "portable" => UpdateArtifactKind::PortableArchive,
        "installer" | "native" => UpdateArtifactKind::NativeInstaller,
        value => return Err(format!("unknown update artifact kind {value:?}")),
    };
    Ok(PublishArtifact {
        platform,
        architecture: fields[1].to_owned(),
        kind,
        path: PathBuf::from(fields[3]),
        url: fields[4].to_owned(),
    })
}

/// Creates an Ed25519 seed file without ever printing private key material.
///
/// # Errors
///
/// Returns an error when entropy or exclusive key-file creation fails.
pub fn generate_key(path: &Path) -> Result<String, String> {
    if path.exists() {
        return Err(format!(
            "update private key {} already exists",
            path.display()
        ));
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err(format!(
            "update private key directory does not exist: {}",
            parent.display()
        ));
    }
    let mut seed = [0_u8; 32];
    fill(&mut seed).map_err(|error| format!("cannot generate update signing key: {error}"))?;
    let signing_key = SigningKey::from_bytes(&seed);
    let encoded = format!("{}\n", encode_hex(&seed));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| {
        format!(
            "cannot create update private key {}: {error}",
            path.display()
        )
    })?;
    file.write_all(encoded.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("cannot persist update private key: {error}"))?;
    Ok(encode_hex(signing_key.verifying_key().as_bytes()))
}

/// Builds and signs a multi-platform update feed from immutable artifacts.
///
/// # Errors
///
/// Returns an error for insecure key permissions, a key/config mismatch,
/// invalid artifact metadata, or atomic output publication failure.
pub fn publish_feed(
    manifest: &ApplicationManifest,
    options: PublishOptions,
) -> Result<PathBuf, String> {
    let signing_key = read_signing_key(&options.private_key)?;
    let public_key = encode_hex(signing_key.verifying_key().as_bytes());
    if let Some(config) = &manifest.updates
        && config.public_key != public_key
    {
        return Err(
            "the update signing key does not match the public key configured in the application"
                .to_owned(),
        );
    }
    let mut artifacts = Vec::with_capacity(options.artifacts.len());
    for input in options.artifacts {
        let path = input.path.canonicalize().map_err(|error| {
            format!("cannot resolve artifact {}: {error}", input.path.display())
        })?;
        if !path.is_file() {
            return Err(format!("update artifact is not a file: {}", path.display()));
        }
        let (bytes, sha256) = file_size_and_hash(&path)?;
        artifacts.push(UpdateArtifact {
            platform: input.platform,
            architecture: input.architecture,
            kind: input.kind,
            url: input.url,
            bytes,
            sha256,
        });
    }
    let release = UpdateRelease {
        schema_version: UPDATE_FEED_VERSION,
        application_id: manifest.identifier.clone(),
        channel: options.channel,
        version: manifest.version.clone(),
        published_at: options.published_at,
        notes_url: options.notes_url,
        artifacts,
    };
    release.validate()?;
    let payload = serde_json::to_vec(&release)
        .map_err(|error| format!("cannot serialize update release: {error}"))?;
    let signed = SignedUpdateRelease {
        signature: encode_hex(&signing_key.sign(&payload).to_bytes()),
        release,
    };
    let mut encoded = serde_json::to_vec_pretty(&signed)
        .map_err(|error| format!("cannot serialize signed update feed: {error}"))?;
    encoded.push(b'\n');
    atomic_write(&options.output, &encoded, options.force)?;
    Ok(options.output)
}

fn read_signing_key(path: &Path) -> Result<SigningKey, String> {
    let metadata = fs::metadata(path).map_err(|error| {
        format!(
            "cannot inspect update private key {}: {error}",
            path.display()
        )
    })?;
    if !metadata.is_file() {
        return Err(format!(
            "update private key is not a file: {}",
            path.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(
                "update private key permissions are too broad; use owner-only mode 0600".to_owned(),
            );
        }
    }
    let source = fs::read_to_string(path)
        .map_err(|error| format!("cannot read update private key: {error}"))?;
    let seed =
        decode_hex::<32>(source.trim(), "update private key").map_err(|error| error.message)?;
    Ok(SigningKey::from_bytes(&seed))
}

fn file_size_and_hash(path: &Path) -> Result<(u64, String), String> {
    let mut file =
        File::open(path).map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    let mut digest = Sha256::new();
    let bytes = std::io::copy(&mut file, &mut HashWriter(&mut digest))
        .map_err(|error| format!("cannot hash {}: {error}", path.display()))?;
    Ok((bytes, format!("{:x}", digest.finalize())))
}

fn atomic_write(path: &Path, contents: &[u8], force: bool) -> Result<(), String> {
    if path.exists() && !force {
        return Err(format!(
            "update feed {} already exists; pass --force to replace it",
            path.display()
        ));
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err(format!(
            "update feed directory does not exist: {}",
            parent.display()
        ));
    }
    let mut entropy = [0_u8; 8];
    fill(&mut entropy).map_err(|error| format!("cannot create feed staging path: {error}"))?;
    let temporary = parent.join(format!(".pam-feed-{}", encode_hex(&entropy)));
    fs::write(&temporary, contents)
        .map_err(|error| format!("cannot write update feed staging file: {error}"))?;
    if !force || !path.exists() || cfg!(unix) {
        return fs::rename(&temporary, path)
            .map_err(|error| format!("cannot publish update feed atomically: {error}"));
    }
    let backup = parent.join(format!(".pam-feed-backup-{}", encode_hex(&entropy)));
    fs::rename(path, &backup)
        .map_err(|error| format!("cannot stage the existing update feed: {error}"))?;
    if let Err(error) = fs::rename(&temporary, path) {
        let rollback = fs::rename(&backup, path);
        return Err(match rollback {
            Ok(()) => format!("cannot publish update feed; rollback succeeded: {error}"),
            Err(rollback) => {
                format!("cannot publish update feed ({error}) and rollback failed ({rollback})")
            }
        });
    }
    fs::remove_file(&backup)
        .map_err(|error| format!("feed published but old staging cleanup failed: {error}"))
}

impl ApplyOptions {
    /// Parses the private update-helper command.
    ///
    /// # Errors
    ///
    /// Returns an error for missing, duplicate, or malformed helper arguments.
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut values = HashMap::<String, OsString>::new();
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            let key = argument
                .to_str()
                .filter(|value| value.starts_with("--"))
                .ok_or_else(|| "invalid apply-update option".to_owned())?
                .to_owned();
            let value = arguments
                .next()
                .ok_or_else(|| format!("{key} requires a value"))?;
            if values.insert(key.clone(), value).is_some() {
                return Err(format!("{key} may only be specified once"));
            }
        }
        let take = |values: &mut HashMap<String, OsString>, key: &str| {
            values
                .remove(key)
                .ok_or_else(|| format!("{key} is required"))
        };
        let parent_pid = take(&mut values, "--parent")?
            .to_str()
            .ok_or_else(|| "--parent must be valid UTF-8".to_owned())?
            .parse::<u32>()
            .map_err(|_| "--parent must be a positive process identifier".to_owned())?;
        if parent_pid == 0 {
            return Err("--parent must be a positive process identifier".to_owned());
        }
        let bundle = PathBuf::from(take(&mut values, "--bundle")?);
        let archive = PathBuf::from(take(&mut values, "--archive")?);
        let sha256 = take(&mut values, "--sha256")?
            .to_str()
            .filter(|value| {
                value.len() == 64
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
            .ok_or_else(|| "--sha256 must be a lowercase SHA-256 digest".to_owned())?
            .to_owned();
        let launcher = values.remove("--launcher").map(PathBuf::from);
        if !values.is_empty() {
            return Err(format!(
                "unknown apply-update option {}",
                values.keys().next().expect("map is not empty")
            ));
        }
        Ok(Self {
            parent_pid,
            bundle,
            archive,
            sha256,
            launcher,
        })
    }
}

/// Applies a verified portable bundle after the original host exits.
///
/// # Errors
///
/// Returns an error without replacing the current application when process
/// waiting, archive integrity, extraction, bundle verification, or atomic
/// swapping fails.
pub fn apply(options: ApplyOptions) -> Result<(), String> {
    let bundle = options
        .bundle
        .canonicalize()
        .map_err(|error| format!("cannot resolve installed bundle: {error}"))?;
    let archive = options
        .archive
        .canonicalize()
        .map_err(|error| format!("cannot resolve staged update archive: {error}"))?;
    verify_file_hash(&archive, &options.sha256)?;
    wait_for_parent(options.parent_pid)?;
    let stage = archive
        .parent()
        .ok_or_else(|| "staged update archive has no parent".to_owned())?;
    let expanded = stage.join("expanded");
    if expanded.exists() {
        fs::remove_dir_all(&expanded)
            .map_err(|error| format!("cannot reset update extraction directory: {error}"))?;
    }
    fs::create_dir(&expanded)
        .map_err(|error| format!("cannot create update extraction directory: {error}"))?;
    extract_archive(&archive, &expanded)?;
    let replacement = single_extracted_root(&expanded)?;
    verify_replacement(&replacement)?;

    let parent = bundle
        .parent()
        .ok_or_else(|| "installed bundle has no parent".to_owned())?;
    let name = bundle
        .file_name()
        .ok_or_else(|| "installed bundle has no filename".to_owned())?
        .to_string_lossy();
    let backup = parent.join(format!(".{name}.previous"));
    if backup.exists() {
        fs::remove_dir_all(&backup)
            .map_err(|error| format!("cannot remove the previous update backup: {error}"))?;
    }
    fs::rename(&bundle, &backup)
        .map_err(|error| format!("cannot move the installed bundle to backup: {error}"))?;
    if let Err(error) = fs::rename(&replacement, &bundle) {
        let rollback = fs::rename(&backup, &bundle);
        return Err(match rollback {
            Ok(()) => format!("cannot install the replacement bundle; rollback succeeded: {error}"),
            Err(rollback) => format!(
                "cannot install the replacement bundle ({error}) and rollback failed ({rollback})"
            ),
        });
    }
    verify_replacement(&bundle).map_err(|error| {
        let failed = parent.join(format!(".{name}.failed"));
        let _ = fs::rename(&bundle, &failed);
        let rollback = fs::rename(&backup, &bundle);
        match rollback {
            Ok(()) => format!("installed bundle verification failed; rollback succeeded: {error}"),
            Err(rollback) => format!(
                "installed bundle verification failed ({error}) and rollback failed ({rollback})"
            ),
        }
    })?;

    if let Some(launcher) = options.launcher {
        validate_relative_path(&launcher, "update relaunch path")?;
        let executable = bundle.join(launcher);
        Command::new(&executable)
            .current_dir(&bundle)
            .spawn()
            .map_err(|error| {
                format!("update installed but application relaunch failed: {error}")
            })?;
    }
    Ok(())
}

fn verify_replacement(root: &Path) -> Result<(), String> {
    if root.extension() == Some(OsStr::new("app")) {
        if !cfg!(target_os = "macos") {
            return Err(
                "macOS application updates cannot be installed on this platform".to_owned(),
            );
        }
        let status = Command::new("codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(root)
            .status()
            .map_err(|error| format!("cannot start macOS signature verification: {error}"))?;
        if !status.success() {
            return Err(format!(
                "macOS application signature verification failed with status {status}"
            ));
        }
        return verify_bundle(&root.join("Contents/Resources/runtime"));
    }
    verify_bundle(root)
}

fn wait_for_parent(parent_pid: u32) -> Result<(), String> {
    let started = Instant::now();
    while process_exists(parent_pid)? {
        if started.elapsed() > PARENT_EXIT_TIMEOUT {
            return Err("timed out waiting for the desktop host to exit".to_owned());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}

fn process_exists(parent_pid: u32) -> Result<bool, String> {
    if cfg!(windows) {
        let output = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {parent_pid}"), "/NH"])
            .output()
            .map_err(|error| format!("cannot inspect parent process with tasklist: {error}"))?;
        Ok(String::from_utf8_lossy(&output.stdout).contains(&parent_pid.to_string()))
    } else {
        let status = Command::new("ps")
            .args(["-p", &parent_pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| format!("cannot inspect parent process with ps: {error}"))?;
        Ok(status.success())
    }
}

fn extract_archive(archive: &Path, destination: &Path) -> Result<(), String> {
    let status = if cfg!(target_os = "linux") {
        Command::new("tar")
            .args(["-xzf"])
            .arg(archive)
            .arg("-C")
            .arg(destination)
            .arg("--no-same-owner")
            .arg("--no-same-permissions")
            .status()
    } else if cfg!(target_os = "macos") {
        Command::new("ditto")
            .args(["-x", "-k"])
            .arg(archive)
            .arg(destination)
            .status()
    } else if cfg!(windows) {
        Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Expand-Archive",
            ])
            .arg("-LiteralPath")
            .arg(archive)
            .arg("-DestinationPath")
            .arg(destination)
            .arg("-Force")
            .status()
    } else {
        return Err("portable update extraction is unsupported on this platform".to_owned());
    }
    .map_err(|error| format!("cannot start portable archive extraction: {error}"))?;
    if !status.success() {
        return Err(format!(
            "portable archive extraction failed with status {status}"
        ));
    }
    Ok(())
}

fn single_extracted_root(directory: &Path) -> Result<PathBuf, String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("cannot inspect extracted update: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot inspect extracted update: {error}"))?;
    if entries.len() != 1 {
        return Err("portable update must contain exactly one bundle directory".to_owned());
    }
    let entry = entries.pop().expect("one entry was required");
    if !entry
        .file_type()
        .map_err(|error| format!("cannot inspect extracted update root: {error}"))?
        .is_dir()
    {
        return Err("portable update root must be a directory".to_owned());
    }
    Ok(entry.path())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstalledBundleManifest {
    schema_version: u8,
    protocol_version: u16,
    application: ApplicationManifest,
    files: Vec<InstalledBundleFile>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstalledBundleFile {
    path: String,
    bytes: u64,
    sha256: String,
}

fn verify_bundle(root: &Path) -> Result<(), String> {
    let manifest_path = root.join("manifest.json");
    let mut source = File::open(&manifest_path)
        .map_err(|error| format!("cannot open bundle manifest: {error}"))?;
    if source
        .metadata()
        .map_err(|error| format!("cannot inspect bundle manifest: {error}"))?
        .len()
        > MAX_BUNDLE_MANIFEST_BYTES
    {
        return Err("bundle manifest exceeds the size limit".to_owned());
    }
    let mut bytes = Vec::new();
    source
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read bundle manifest: {error}"))?;
    let manifest: InstalledBundleManifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid bundle manifest: {error}"))?;
    if manifest.schema_version != 1 || manifest.protocol_version != PROTOCOL_VERSION {
        return Err("bundle manifest uses an unsupported schema or protocol".to_owned());
    }
    manifest.application.validate()?;

    let mut expected = HashMap::with_capacity(manifest.files.len());
    for file in manifest.files {
        let relative = PathBuf::from(&file.path);
        validate_relative_path(&relative, "bundle file")?;
        if expected
            .insert(relative, (file.bytes, file.sha256))
            .is_some()
        {
            return Err(format!(
                "bundle manifest path {:?} is duplicated",
                file.path
            ));
        }
    }
    let actual = bundle_files(root)?;
    if actual.len() != expected.len() {
        return Err("bundle contents do not match the signed artifact manifest".to_owned());
    }
    for (relative, path) in actual {
        let (bytes, sha256) = expected
            .remove(&relative)
            .ok_or_else(|| format!("bundle contains an undeclared file: {}", relative.display()))?;
        let metadata = path
            .metadata()
            .map_err(|error| format!("cannot inspect bundle file {}: {error}", path.display()))?;
        if metadata.len() != bytes {
            return Err(format!("bundle file size mismatch: {}", relative.display()));
        }
        verify_file_hash(&path, &sha256)?;
    }
    if !expected.is_empty() {
        return Err("bundle is missing files declared by its manifest".to_owned());
    }
    Ok(())
}

fn bundle_files(root: &Path) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    fn visit(
        root: &Path,
        directory: &Path,
        files: &mut Vec<(PathBuf, PathBuf)>,
    ) -> Result<(), String> {
        for entry in fs::read_dir(directory)
            .map_err(|error| format!("cannot inspect bundle directory: {error}"))?
        {
            let entry = entry.map_err(|error| format!("cannot inspect bundle entry: {error}"))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| format!("cannot inspect bundle entry type: {error}"))?;
            if file_type.is_symlink() {
                return Err(format!(
                    "bundle contains a symbolic link: {}",
                    path.display()
                ));
            }
            if file_type.is_dir() {
                visit(root, &path, files)?;
            } else if file_type.is_file() && path != root.join("manifest.json") {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| "bundle path escaped its root".to_owned())?
                    .to_path_buf();
                files.push((relative, path));
            } else if !file_type.is_file() {
                return Err(format!(
                    "bundle contains a special file: {}",
                    path.display()
                ));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    visit(root, root, &mut files)?;
    Ok(files)
}

fn verify_file_hash(path: &Path, expected: &str) -> Result<(), String> {
    let mut file =
        File::open(path).map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    let mut digest = Sha256::new();
    std::io::copy(&mut file, &mut HashWriter(&mut digest))
        .map_err(|error| format!("cannot hash {}: {error}", path.display()))?;
    let actual = format!("{:x}", digest.finalize());
    if actual != expected {
        return Err(format!("SHA-256 mismatch for {}", path.display()));
    }
    Ok(())
}

struct HashWriter<'a>(&'a mut Sha256);

impl Write for HashWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.update(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn validate_relative_path(path: &Path, label: &str) -> Result<(), String> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("{label} must be a normalized relative path"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::time::{SystemTime, UNIX_EPOCH};

    use ed25519_dalek::{Signer, SigningKey};
    use pam_desktop_protocol::{
        ApplicationCategory, SignedUpdateRelease, UPDATE_FEED_VERSION, UpdateArtifact,
        UpdateRelease,
    };

    use super::*;

    #[test]
    fn compares_release_versions_without_build_metadata() {
        assert!(version_is_newer("0.5.1", "0.5.0"));
        assert!(version_is_newer("1.0.0", "1.0.0-rc.2"));
        assert!(version_is_newer("1.0.0-rc.10", "1.0.0-rc.2"));
        assert!(!version_is_newer("1.0.0+build.2", "1.0.0+build.1"));
        assert!(!version_is_newer("0.4.9", "0.5.0"));
    }

    #[test]
    fn verifies_canonical_ed25519_release_payloads() {
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let release = UpdateRelease {
            schema_version: UPDATE_FEED_VERSION,
            application_id: "com.pushin.test".to_owned(),
            channel: "stable".to_owned(),
            version: "0.5.1".to_owned(),
            published_at: "2026-07-23T14:00:00Z".to_owned(),
            notes_url: None,
            artifacts: vec![UpdateArtifact {
                platform: UpdatePlatform::Linux,
                architecture: std::env::consts::ARCH.to_owned(),
                kind: UpdateArtifactKind::PortableArchive,
                url: "https://updates.pushin.dev/test.tar.gz".to_owned(),
                bytes: 42,
                sha256: "a".repeat(64),
            }],
        };
        let payload = serde_json::to_vec(&release).expect("release should serialize");
        let feed = SignedUpdateRelease {
            release,
            signature: encode_hex(&signing_key.sign(&payload).to_bytes()),
        };
        let public_key = encode_hex(signing_key.verifying_key().as_bytes());

        verify_signed_feed(&feed, &public_key).expect("signature should verify");
    }

    #[test]
    fn rejects_tampered_release_payloads() {
        let signing_key = SigningKey::from_bytes(&[9_u8; 32]);
        let mut release = UpdateRelease {
            schema_version: UPDATE_FEED_VERSION,
            application_id: "com.pushin.test".to_owned(),
            channel: "stable".to_owned(),
            version: "0.5.1".to_owned(),
            published_at: "2026-07-23T14:00:00Z".to_owned(),
            notes_url: None,
            artifacts: vec![UpdateArtifact {
                platform: UpdatePlatform::Linux,
                architecture: std::env::consts::ARCH.to_owned(),
                kind: UpdateArtifactKind::PortableArchive,
                url: "https://updates.pushin.dev/test.tar.gz".to_owned(),
                bytes: 42,
                sha256: "a".repeat(64),
            }],
        };
        let signature = encode_hex(
            &signing_key
                .sign(&serde_json::to_vec(&release).expect("release should serialize"))
                .to_bytes(),
        );
        release.version = "9.9.9".to_owned();
        let feed = SignedUpdateRelease { release, signature };
        let public_key = encode_hex(signing_key.verifying_key().as_bytes());

        assert!(verify_signed_feed(&feed, &public_key).is_err());
    }

    #[test]
    fn rejects_unsafe_helper_paths() {
        assert!(validate_relative_path(Path::new("bin/application"), "launcher").is_ok());
        assert!(validate_relative_path(Path::new("../outside"), "launcher").is_err());
        assert!(validate_relative_path(Path::new("/outside"), "launcher").is_err());
    }

    #[test]
    fn generates_keys_and_publishes_verifiable_multi_platform_feeds() {
        let root = temporary_test_directory("publish");
        let private_key = root.join("release.key");
        let public_key = generate_key(&private_key).expect("key should be generated");
        let linux = root.join("application.tar.gz");
        let windows = root.join("application.zip");
        fs::write(&linux, b"linux artifact").expect("Linux artifact should be written");
        fs::write(&windows, b"Windows artifact").expect("Windows artifact should be written");
        let output = root.join("latest.json");
        let manifest = fixture_application_manifest(public_key.clone());
        let options = PublishOptions {
            private_key,
            output: output.clone(),
            channel: "stable".to_owned(),
            published_at: "2026-07-23T14:00:00Z".to_owned(),
            notes_url: Some("https://updates.pushin.dev/releases/0.5.0".to_owned()),
            artifacts: vec![
                PublishArtifact {
                    platform: UpdatePlatform::Linux,
                    architecture: "x86_64".to_owned(),
                    kind: UpdateArtifactKind::PortableArchive,
                    path: linux,
                    url: "https://updates.pushin.dev/application.tar.gz".to_owned(),
                },
                PublishArtifact {
                    platform: UpdatePlatform::Windows,
                    architecture: "x86_64".to_owned(),
                    kind: UpdateArtifactKind::PortableArchive,
                    path: windows,
                    url: "https://updates.pushin.dev/application.zip".to_owned(),
                },
            ],
            force: false,
        };

        publish_feed(&manifest, options).expect("feed should be published");
        let feed: SignedUpdateRelease =
            serde_json::from_slice(&fs::read(&output).expect("feed should be readable"))
                .expect("feed should be valid JSON");
        verify_signed_feed(&feed, &public_key).expect("published feed should verify");
        assert_eq!(feed.release.artifacts.len(), 2);
        assert_eq!(feed.release.artifacts[0].platform as u8, 1);
        assert_eq!(feed.release.artifacts[1].platform as u8, 2);

        fs::remove_dir_all(root).expect("fixture should be removable");
    }

    #[test]
    fn verifies_every_file_in_staged_bundles() {
        let root = temporary_test_directory("bundle");
        write_fixture_bundle(&root, b"verified");

        verify_bundle(&root).expect("bundle should verify");
        fs::write(root.join("app/payload.txt"), b"tampered").expect("payload should be tampered");
        assert!(verify_bundle(&root).is_err());

        fs::remove_dir_all(root).expect("fixture should be removable");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn atomically_swaps_verified_bundles_and_keeps_a_rollback() {
        let root = temporary_test_directory("apply");
        let installed = root.join("application");
        let staged_root = root.join("staged");
        let replacement = staged_root.join("application-0.5.1-linux-x86_64");
        fs::create_dir(&staged_root).expect("staging root should be created");
        write_fixture_bundle(&installed, b"old");
        write_fixture_bundle(&replacement, b"new");
        let archive = root.join("update.tar.gz");
        let status = Command::new("tar")
            .args(["-czf"])
            .arg(&archive)
            .arg("-C")
            .arg(&staged_root)
            .arg(
                replacement
                    .file_name()
                    .expect("replacement should have a name"),
            )
            .status()
            .expect("tar should start");
        assert!(status.success());
        let (_, sha256) = file_size_and_hash(&archive).expect("archive should hash");

        apply(ApplyOptions {
            parent_pid: u32::MAX,
            bundle: installed.clone(),
            archive,
            sha256,
            launcher: None,
        })
        .expect("verified update should apply");

        assert_eq!(
            fs::read(installed.join("app/payload.txt")).expect("new payload should exist"),
            b"new"
        );
        assert_eq!(
            fs::read(root.join(".application.previous/app/payload.txt"))
                .expect("rollback payload should exist"),
            b"old"
        );

        fs::remove_dir_all(root).expect("fixture should be removable");
    }

    #[test]
    fn checks_and_downloads_a_bounded_signed_loopback_release() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("test update server should bind");
        let address = listener
            .local_addr()
            .expect("test update server should have an address");
        let artifact = b"verified portable update".to_vec();
        let signing_key = SigningKey::from_bytes(&[11_u8; 32]);
        let release = UpdateRelease {
            schema_version: UPDATE_FEED_VERSION,
            application_id: "com.pushin.test".to_owned(),
            channel: "stable".to_owned(),
            version: "0.5.1".to_owned(),
            published_at: "2026-07-23T14:00:00Z".to_owned(),
            notes_url: None,
            artifacts: vec![UpdateArtifact {
                platform: current_platform().expect("test platform should be supported"),
                architecture: std::env::consts::ARCH.to_owned(),
                kind: UpdateArtifactKind::PortableArchive,
                url: format!("http://{address}/artifact"),
                bytes: artifact.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&artifact)),
            }],
        };
        let signature = encode_hex(
            &signing_key
                .sign(&serde_json::to_vec(&release).expect("release should serialize"))
                .to_bytes(),
        );
        let feed = serde_json::to_vec(&SignedUpdateRelease { release, signature })
            .expect("feed should serialize");
        let server_artifact = artifact.clone();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("request should connect");
                let mut request = [0_u8; 2048];
                let read = stream
                    .read(&mut request)
                    .expect("request should be readable");
                let request = String::from_utf8_lossy(&request[..read]);
                let body = if request.starts_with("GET /feed ") {
                    feed.as_slice()
                } else if request.starts_with("GET /artifact ") {
                    server_artifact.as_slice()
                } else {
                    panic!("unexpected update request: {request}");
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .expect("response header should be written");
                stream
                    .write_all(body)
                    .expect("response body should be written");
            }
        });
        let public_key = encode_hex(signing_key.verifying_key().as_bytes());
        let mut manifest = fixture_application_manifest(public_key);
        manifest.version = "0.5.0".to_owned();
        manifest
            .updates
            .as_mut()
            .expect("updates should exist")
            .endpoint = format!("http://{address}/feed");
        let updater = Updater::prepare(&manifest);

        let available = updater.check().expect("signed release should be available");
        assert_eq!(available.state, UpdateState::Available);
        let ready = updater.download().expect("artifact should download");
        assert_eq!(ready.state, UpdateState::Ready);
        assert_eq!(ready.downloaded_bytes, artifact.len() as u64);
        let stage = updater
            .lock_state()
            .archive
            .clone()
            .and_then(|archive| archive.parent().map(Path::to_path_buf))
            .expect("staging directory should exist");

        server.join().expect("test server should stop");
        fs::remove_dir_all(stage).expect("staging directory should be removable");
    }

    fn fixture_application_manifest(public_key: String) -> ApplicationManifest {
        ApplicationManifest {
            identifier: "com.pushin.test".to_owned(),
            name: "Pam Test".to_owned(),
            version: "0.5.0".to_owned(),
            description: "A signed update fixture.".to_owned(),
            publisher: "Pushin".to_owned(),
            category: ApplicationCategory::Development,
            icon: "resources/icon.svg".to_owned(),
            bundle_excludes: Vec::new(),
            updates: Some(UpdateConfig {
                endpoint: "https://updates.pushin.dev/latest.json".to_owned(),
                channel: "stable".to_owned(),
                public_key,
                policy: UpdatePolicy::Manual,
            }),
        }
    }

    fn write_fixture_bundle(root: &Path, payload: &[u8]) {
        fs::create_dir_all(root.join("app")).expect("app directory should be created");
        fs::write(root.join("app/payload.txt"), payload).expect("payload should be written");
        let digest = format!("{:x}", Sha256::digest(payload));
        let mut manifest = fixture_application_manifest("a".repeat(64));
        manifest.updates = None;
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 1,
                "protocolVersion": PROTOCOL_VERSION,
                "application": manifest,
                "runtime": {"pamDesktop": "0.5.0", "pam": "0.1.1"},
                "target": {
                    "operatingSystem": "linux",
                    "architecture": "x86_64",
                    "abi": "glibc"
                },
                "sourceDateEpoch": 0,
                "files": [{
                    "path": "app/payload.txt",
                    "bytes": payload.len(),
                    "sha256": digest
                }]
            }))
            .expect("manifest should serialize"),
        )
        .expect("manifest should be written");
    }

    fn temporary_test_directory(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "pam-desktop-update-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("fixture directory should be created");
        root
    }
}
