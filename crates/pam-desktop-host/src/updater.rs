//! Signed updates: keys, the release feed, and the running updater.
//!
//! A release is a JSON document signed with Ed25519. The application pins
//! the public key in `pam-desktop.json`; the updater accepts a release only
//! after the signature verifies, downloads the portable archive of this
//! system, checks its signed size and SHA-256, and hands it to a copy of the
//! host (`apply-update`) that swaps the installed bundle once the
//! application has exited.
//!
//! Freshness: a feed may carry a signed `expiresAtUnix` (`publish-update
//! --expires-in DAYS`); an expired feed is refused, so a mirror cannot keep
//! serving an old signed release forever. Independently of that, the updater
//! remembers the newest version a verified feed of each channel offered
//! (`.<bundle>.update-feed-seen` beside the bundle) and refuses a feed that
//! goes back to an older one. Feeds without `expiresAtUnix` verify exactly as
//! before; hosts older than this one refuse feeds that carry it, so publishers
//! add it once every installed application runs a host that knows it.
//!
//! Archives are unpacked without programs from `PATH`: in Rust on Linux,
//! with `/usr/bin/ditto` on macOS and the PowerShell of `System32` on Windows.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use pam_desktop_ui::modules::fetch::{HttpClient, HttpRequest, HttpResponse};
use pam_desktop_ui::modules::support::hex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::bundle::{self, Installed, hash_file, validate_relative_path, verify_file_hash};
use crate::manifest::{
    Application, UpdateConfig, UpdatePolicy, validate_channel, validate_identifier,
    validate_lower_hex, validate_text, validate_update_url, validate_version,
};

/// Version of the feed document; unchanged since PAM Desktop 1.x.
pub const FEED_VERSION: u16 = 1;

const MAX_FEED_BYTES: u64 = 1024 * 1024;
const NETWORK_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_REDIRECTS: usize = 3;
const PARENT_EXIT_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_RETAINED_UPDATE_STAGES: usize = 1;
const MAX_TRANSACTION_BYTES: u64 = 4 * 1024;
/// How long the helper keeps trying to move a bundle a process still holds.
const SWAP_ATTEMPTS: u32 = 20;

// ---- feed -----------------------------------------------------------------

/// An enumeration the feed carries as a small integer.
macro_rules! integer_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $value:literal => $keyword:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant = $value),+
        }

        impl $name {
            /// The word used on the command line.
            #[must_use]
            pub const fn keyword(self) -> &'static str {
                match self {
                    $(Self::$variant => $keyword),+
                }
            }

            fn parse(keyword: &str) -> Option<Self> {
                match keyword {
                    $($keyword => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_u8(*self as u8)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                match u8::deserialize(deserializer)? {
                    $($value => Ok(Self::$variant),)+
                    other => Err(serde::de::Error::custom(format!(concat!("unknown ", stringify!($name), " {}"), other))),
                }
            }
        }
    };
}

integer_enum! {
    /// The system an artifact runs on.
    Platform { Linux = 1 => "linux", Windows = 2 => "windows", MacOs = 3 => "macos" }
}

integer_enum! {
    /// What an artifact is: the updater installs portable archives only.
    ArtifactKind { PortableArchive = 1 => "portable", NativeInstaller = 2 => "installer" }
}

impl Platform {
    /// The platform of this host.
    #[must_use]
    pub fn current() -> Option<Self> {
        Self::parse(std::env::consts::OS)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Artifact {
    pub platform: Platform,
    pub architecture: String,
    pub kind: ArtifactKind,
    pub url: String,
    pub bytes: u64,
    pub sha256: String,
}

impl Artifact {
    fn validate(&self) -> Result<(), String> {
        validate_channel(&self.architecture, "update architecture")?;
        validate_update_url(&self.url, "update artifact URL")?;
        if self.bytes == 0 {
            return Err("update artifact size must be greater than zero".to_owned());
        }
        validate_lower_hex(&self.sha256, 64, "update artifact SHA-256")
    }
}

/// The signed part of a feed. Its compact JSON serialization, in this field
/// order, is the message the signature covers.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Release {
    pub schema_version: u16,
    pub application_id: String,
    pub channel: String,
    pub version: String,
    pub published_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_url: Option<String>,
    pub artifacts: Vec<Artifact>,
    /// Signed end of validity (Unix seconds). Last and omitted when absent,
    /// so feeds without it keep their exact signed payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_unix: Option<u64>,
}

impl Release {
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != FEED_VERSION {
            return Err(format!(
                "update feed schema {} is unsupported; expected {FEED_VERSION}",
                self.schema_version
            ));
        }
        validate_identifier(&self.application_id)?;
        validate_channel(&self.channel, "update channel")?;
        validate_version(&self.version)?;
        validate_text(
            &self.published_at,
            "update publication timestamp",
            64,
            false,
        )?;
        if let Some(notes_url) = &self.notes_url {
            validate_update_url(notes_url, "update notes URL")?;
        }
        if self.artifacts.is_empty() {
            return Err("update release must contain at least one artifact".to_owned());
        }
        if self.expires_at_unix == Some(0) {
            return Err("update feed expiry must be a positive Unix time".to_owned());
        }
        let mut targets = HashSet::with_capacity(self.artifacts.len());
        for artifact in &self.artifacts {
            artifact.validate()?;
            if !targets.insert((
                artifact.platform,
                artifact.architecture.as_str(),
                artifact.kind,
            )) {
                return Err(format!(
                    "update artifact target {}/{} ({}) is duplicated",
                    artifact.platform.keyword(),
                    artifact.architecture,
                    artifact.kind.keyword()
                ));
            }
        }
        Ok(())
    }

    fn signed_payload(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(self)
            .map_err(|error| format!("cannot serialize the update release: {error}"))
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignedRelease {
    pub release: Release,
    /// Ed25519 signature of the release, 128 lowercase hexadecimal characters.
    pub signature: String,
}

/// Checks the shape of a feed and its signature against the pinned key.
///
/// # Errors
///
/// When the feed is malformed or was not signed by the holder of the key.
pub fn verify_signed_feed(feed: &SignedRelease, public_key: &str) -> Result<(), String> {
    feed.release
        .validate()
        .map_err(|error| format!("The update feed is invalid: {error}"))?;
    let public_key = decode_hex::<32>(public_key, "update public key")?;
    let signature = decode_hex::<64>(&feed.signature, "update signature")?;
    let verifying_key = VerifyingKey::from_bytes(&public_key)
        .map_err(|_| "The configured Ed25519 update public key is invalid.".to_owned())?;
    let signature = Signature::from_bytes(&signature);
    verifying_key
        .verify_strict(&feed.release.signed_payload()?, &signature)
        .map_err(|_| "The update feed signature did not verify.".to_owned())
}

fn decode_hex<const N: usize>(value: &str, label: &str) -> Result<[u8; N], String> {
    if value.len() != N * 2 {
        return Err(format!("The {label} has an invalid length."));
    }
    let nibble = |byte: u8| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    };
    let mut decoded = [0_u8; N];
    for (index, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let (Some(high), Some(low)) = (nibble(pair[0]), nibble(pair[1])) else {
            return Err(format!("The {label} is not lowercase hexadecimal."));
        };
        decoded[index] = high << 4 | low;
    }
    Ok(decoded)
}

fn version_is_newer(candidate: &str, current: &str) -> bool {
    compare_versions(candidate, current) == Ordering::Greater
}

/// Orders versions like semantic versioning: numeric parts, then a release
/// above its pre-releases; build metadata is ignored.
fn compare_versions(left: &str, right: &str) -> Ordering {
    let split = |version: &'_ str| -> (Vec<String>, Option<String>) {
        let version = version
            .split_once('+')
            .map_or(version, |(version, _)| version);
        let (core, pre) = version
            .split_once('-')
            .map_or((version, None), |(core, pre)| (core, Some(pre.to_owned())));
        (core.split('.').map(str::to_owned).collect(), pre)
    };
    let (left_parts, left_pre) = split(left);
    let (right_parts, right_pre) = split(right);
    for index in 0..left_parts.len().max(right_parts.len()) {
        let left = left_parts.get(index).map_or("0", String::as_str);
        let right = right_parts.get(index).map_or("0", String::as_str);
        let order = match (left.parse::<u64>(), right.parse::<u64>()) {
            (Ok(left), Ok(right)) => left.cmp(&right),
            _ => left.cmp(right),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    match (left_pre, right_pre) {
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left), Some(right)) => compare_prerelease(&left, &right),
        (None, None) => Ordering::Equal,
    }
}

fn compare_prerelease(left: &str, right: &str) -> Ordering {
    let left: Vec<&str> = left.split('.').collect();
    let right: Vec<&str> = right.split('.').collect();
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

// ---- the running updater --------------------------------------------------

/// Where the updater stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The manifest declares no `updates`.
    Disabled,
    Idle,
    Checking,
    Available,
    Downloading,
    /// Downloaded and verified: `install` applies it.
    Ready,
    /// The helper is waiting for the application to exit.
    Applying,
    UpToDate,
    Failed,
}

impl Status {
    /// The word PHP receives.
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Idle => "idle",
            Self::Checking => "checking",
            Self::Available => "available",
            Self::Downloading => "downloading",
            Self::Ready => "ready",
            Self::Applying => "applying",
            Self::UpToDate => "up-to-date",
            Self::Failed => "failed",
        }
    }

    const fn busy(self) -> bool {
        matches!(self, Self::Checking | Self::Downloading | Self::Applying)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub status: Status,
    pub current_version: String,
    pub available_version: Option<String>,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub notes_url: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
struct Candidate {
    artifact: Artifact,
    version: String,
    notes_url: Option<String>,
}

struct InternalState {
    snapshot: Snapshot,
    candidate: Option<Candidate>,
    archive: Option<PathBuf>,
}

type Listener = Box<dyn Fn(&Snapshot) + Send + Sync>;

struct Inner {
    application_id: String,
    config: Option<UpdateConfig>,
    installed: Option<Installed>,
    client: Arc<dyn HttpClient>,
    state: Mutex<InternalState>,
    listener: Listener,
    /// The HTTP client wants a cancellation flag; updates are never cancelled.
    never_cancelled: AtomicBool,
}

/// Checks, downloads and installs releases of one application. Cheap to
/// clone; every operation blocks and belongs on a worker thread.
#[derive(Clone)]
pub struct Updater {
    inner: Arc<Inner>,
}

impl Updater {
    /// `installed` is the bundle the host runs from, when it runs from one;
    /// `listener` sees every change of the snapshot, from any thread.
    #[must_use]
    pub fn new(
        application_id: &str,
        current_version: &str,
        config: Option<UpdateConfig>,
        installed: Option<Installed>,
        client: Arc<dyn HttpClient>,
        listener: impl Fn(&Snapshot) + Send + Sync + 'static,
    ) -> Self {
        let status = if config.is_some() {
            Status::Idle
        } else {
            Status::Disabled
        };
        Self {
            inner: Arc::new(Inner {
                application_id: application_id.to_owned(),
                config,
                installed,
                client,
                state: Mutex::new(InternalState {
                    snapshot: Snapshot {
                        status,
                        current_version: current_version.to_owned(),
                        available_version: None,
                        downloaded_bytes: 0,
                        total_bytes: 0,
                        notes_url: None,
                        error: None,
                    },
                    candidate: None,
                    archive: None,
                }),
                listener: Box::new(listener),
                never_cancelled: AtomicBool::new(false),
            }),
        }
    }

    #[must_use]
    pub fn policy(&self) -> Option<UpdatePolicy> {
        self.inner.config.as_ref().map(|config| config.policy)
    }

    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        self.lock_state().snapshot.clone()
    }

    /// Fetches and verifies the release feed.
    ///
    /// # Errors
    ///
    /// When updates are disabled, another operation is running, the feed
    /// cannot be fetched, or it is not a release signed for this application
    /// and channel.
    pub fn check(&self) -> Result<Snapshot, String> {
        let config = self.config()?.clone();
        self.begin(Status::Checking)?;
        match self.check_inner(&config) {
            Ok(candidate) => Ok(self.change(|state| {
                state.archive = None;
                state.snapshot.error = None;
                state.snapshot.downloaded_bytes = 0;
                if let Some(candidate) = candidate {
                    state.snapshot.status = Status::Available;
                    state.snapshot.available_version = Some(candidate.version.clone());
                    state.snapshot.total_bytes = candidate.artifact.bytes;
                    state.snapshot.notes_url.clone_from(&candidate.notes_url);
                    state.candidate = Some(candidate);
                } else {
                    state.snapshot.status = Status::UpToDate;
                    state.snapshot.available_version = None;
                    state.snapshot.total_bytes = 0;
                    state.snapshot.notes_url = None;
                    state.candidate = None;
                }
            })),
            Err(error) => Err(self.fail(error)),
        }
    }

    /// Downloads the archive of the available release and verifies its
    /// signed byte length and SHA-256.
    ///
    /// # Errors
    ///
    /// When no release is available, the staging directory cannot be
    /// prepared, the transfer fails or the archive is not the signed one.
    pub fn download(&self) -> Result<Snapshot, String> {
        let candidate = self
            .lock_state()
            .candidate
            .clone()
            .ok_or("No update is available. Check for updates first.")?;
        self.begin(Status::Downloading)?;
        match self.download_inner(&candidate) {
            Ok(archive) => Ok(self.change(|state| {
                state.archive = Some(archive);
                state.snapshot.status = Status::Ready;
                state.snapshot.downloaded_bytes = candidate.artifact.bytes;
                state.snapshot.error = None;
            })),
            Err(error) => Err(self.fail(error)),
        }
    }

    /// Starts the helper that swaps the bundle. The host must exit right
    /// after this succeeds: the helper waits for it, and Windows cannot
    /// replace executables that are running.
    ///
    /// # Errors
    ///
    /// When no verified archive is staged or the host does not run from an
    /// installed bundle.
    pub fn install(&self) -> Result<Snapshot, String> {
        let (archive, candidate) = {
            let state = self.lock_state();
            let archive = state
                .archive
                .clone()
                .ok_or("No verified update has been downloaded.")?;
            let candidate = state
                .candidate
                .clone()
                .ok_or("The staged update has no matching release.")?;
            (archive, candidate)
        };
        let installed = self
            .inner
            .installed
            .clone()
            .ok_or("Updates can only be installed from a packaged PAM Desktop application.")?;
        self.begin(Status::Applying)?;
        match launch_apply_helper(
            &installed,
            &archive,
            &candidate.artifact.sha256,
            std::process::id(),
        ) {
            Ok(()) => Ok(self.snapshot()),
            Err(error) => Err(self.fail(error)),
        }
    }

    /// What the `notify` and `automatic` policies do at startup. Returns
    /// whether an installation started, in which case the host must exit.
    #[must_use]
    pub fn run_policy(&self) -> bool {
        let Some(policy) = self
            .policy()
            .filter(|policy| *policy != UpdatePolicy::Manual)
        else {
            return false;
        };
        let available = self
            .check()
            .is_ok_and(|snapshot| snapshot.status == Status::Available);
        policy == UpdatePolicy::Automatic
            && available
            && !self.rejected_here()
            && self.download().is_ok()
            && self.install().is_ok()
    }

    /// Whether the available release already failed to install over this
    /// bundle: the `automatic` policy leaves it to the application, which
    /// can still download and install it on request.
    fn rejected_here(&self) -> bool {
        let candidate = self.lock_state().candidate.clone();
        self.inner
            .installed
            .as_ref()
            .zip(candidate)
            .is_some_and(|(installed, candidate)| {
                rejected_release(&installed.root, &candidate.artifact.sha256)
            })
    }

    fn check_inner(&self, config: &UpdateConfig) -> Result<Option<Candidate>, String> {
        let response = self.get(&config.endpoint, "update feed", Some(NETWORK_TIMEOUT))?;
        let mut bytes = Vec::new();
        response
            .body
            .take(MAX_FEED_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("Cannot read the update feed: {error}"))?;
        if bytes.len() as u64 > MAX_FEED_BYTES {
            return Err("The update feed is larger than one MiB.".to_owned());
        }
        let feed: SignedRelease = serde_json::from_slice(&bytes)
            .map_err(|error| format!("The update feed is not valid: {error}"))?;
        verify_signed_feed(&feed, &config.public_key)?;
        if feed.release.application_id != self.inner.application_id {
            return Err("The signed release belongs to a different application.".to_owned());
        }
        if feed.release.channel != config.channel {
            return Err("The signed release belongs to a different update channel.".to_owned());
        }
        let watermark = self
            .inner
            .installed
            .as_ref()
            .and_then(|installed| feed_watermark_path(&installed.root).ok());
        check_feed_freshness(&feed.release, unix_now(), watermark.as_deref())?;
        let current_version = self.lock_state().snapshot.current_version.clone();
        if !version_is_newer(&feed.release.version, &current_version) {
            return Ok(None);
        }
        let platform =
            Platform::current().ok_or("This operating system has no PAM Desktop update target.")?;
        let artifact = feed
            .release
            .artifacts
            .into_iter()
            .find(|artifact| {
                artifact.platform == platform
                    && artifact.architecture == std::env::consts::ARCH
                    && artifact.kind == ArtifactKind::PortableArchive
            })
            .ok_or_else(|| {
                format!(
                    "Release {} has no portable artifact for {}/{}.",
                    feed.release.version,
                    std::env::consts::OS,
                    std::env::consts::ARCH
                )
            })?;
        Ok(Some(Candidate {
            artifact,
            version: feed.release.version,
            notes_url: feed.release.notes_url,
        }))
    }

    fn download_inner(&self, candidate: &Candidate) -> Result<PathBuf, String> {
        let parent = self
            .inner
            .installed
            .as_ref()
            .and_then(|installed| installed.root.parent().map(Path::to_path_buf))
            .unwrap_or_else(std::env::temp_dir);
        let stage = update_stage(&parent, &self.inner.application_id, &candidate.version)?;
        let archive = stage.join(archive_name());
        let expected = candidate.artifact.bytes;
        // No overall deadline: a large archive on a slow link is still progress.
        let mut response = self.get(&candidate.artifact.url, "update artifact", None)?;
        if response
            .content_length
            .is_some_and(|length| length != expected)
        {
            return Err("The update server returned an unexpected artifact size.".to_owned());
        }
        let mut output = File::create(&archive)
            .map_err(|error| format!("Cannot create the update staging file: {error}"))?;
        let mut digest = Sha256::new();
        let mut total = 0_u64;
        let mut reported = 0_u64;
        let mut buffer = vec![0_u8; 128 * 1024];
        loop {
            let read = match response.body.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(format!("The update download was interrupted: {error}")),
            };
            total = total.saturating_add(read as u64);
            if total > expected {
                return Err("The downloaded update is larger than its signed size.".to_owned());
            }
            output
                .write_all(&buffer[..read])
                .map_err(|error| format!("Cannot write the update staging file: {error}"))?;
            digest.update(&buffer[..read]);
            // One change per whole percent keeps listeners informed without flooding them.
            let percent = total.saturating_mul(100) / expected;
            if percent != reported {
                reported = percent;
                self.change(|state| state.snapshot.downloaded_bytes = total);
            }
        }
        output
            .sync_all()
            .map_err(|error| format!("Cannot flush the update staging file: {error}"))?;
        if total != expected || hex(&digest.finalize()) != candidate.artifact.sha256 {
            return Err(
                "The downloaded update does not match its signed size and SHA-256 digest."
                    .to_owned(),
            );
        }
        Ok(archive)
    }

    /// A GET that follows a few redirects itself, so that every hop obeys
    /// the same HTTPS rule as the URL the application pinned.
    fn get(
        &self,
        url: &str,
        label: &str,
        total_timeout: Option<Duration>,
    ) -> Result<HttpResponse, String> {
        let mut url = url.to_owned();
        for _ in 0..=MAX_REDIRECTS {
            validate_update_url(&url, label)
                .map_err(|error| format!("Cannot fetch the {label}: {error}"))?;
            let mut request = HttpRequest::get(url.as_str());
            request.connect_timeout = NETWORK_TIMEOUT;
            request.read_timeout = NETWORK_TIMEOUT;
            request.total_timeout = total_timeout;
            let response = self
                .inner
                .client
                .send(request, None, &self.inner.never_cancelled)
                .map_err(|error| format!("Cannot fetch the {label}: {error}"))?;
            match response.status {
                200 => return Ok(response),
                301 | 302 | 303 | 307 | 308 => {
                    let location = response.header("location").ok_or_else(|| {
                        format!("Cannot fetch the {label}: redirect without a location")
                    })?;
                    url = redirect_target(&url, location)
                        .ok_or_else(|| format!("Cannot fetch the {label}: unsupported redirect"))?;
                }
                status => {
                    return Err(format!(
                        "Cannot fetch the {label}: the server answered {status}"
                    ));
                }
            }
        }
        Err(format!("Cannot fetch the {label}: too many redirects"))
    }

    fn config(&self) -> Result<&UpdateConfig, String> {
        self.inner
            .config
            .as_ref()
            .ok_or_else(|| "Updates are not configured for this application.".to_owned())
    }

    fn begin(&self, next: Status) -> Result<(), String> {
        let mut busy = false;
        self.change(|state| {
            busy = state.snapshot.status.busy();
            if busy {
                return;
            }
            state.snapshot.status = next;
            state.snapshot.error = None;
            if next == Status::Downloading {
                state.snapshot.downloaded_bytes = 0;
            }
        });
        if busy {
            Err("Another update operation is already running.".to_owned())
        } else {
            Ok(())
        }
    }

    fn fail(&self, error: String) -> String {
        self.change(|state| {
            state.snapshot.status = Status::Failed;
            state.snapshot.error = Some(error.clone());
        });
        error
    }

    /// Applies `change` and tells the listener when the snapshot differs.
    fn change(&self, change: impl FnOnce(&mut InternalState)) -> Snapshot {
        let (before, after) = {
            let mut state = self.lock_state();
            let before = state.snapshot.clone();
            change(&mut state);
            (before, state.snapshot.clone())
        };
        if before != after {
            (self.inner.listener)(&after);
        }
        after
    }

    fn lock_state(&self) -> MutexGuard<'_, InternalState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Where a redirect leads: an absolute URL, or a path on the same origin.
fn redirect_target(current: &str, location: &str) -> Option<String> {
    if location.contains("://") {
        return Some(location.to_owned());
    }
    if !location.starts_with('/') || location.starts_with("//") {
        return None;
    }
    let (scheme, rest) = current.split_once("://")?;
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    Some(format!("{scheme}://{authority}{location}"))
}

/// A fresh staging directory beside the bundle (same file system, so the
/// swap is a rename), keeping at most one older stage of the application.
fn update_stage(parent: &Path, application_id: &str, version: &str) -> Result<PathBuf, String> {
    prune_update_stages(parent, application_id)?;
    let mut entropy = [0_u8; 8];
    getrandom::fill(&mut entropy)
        .map_err(|error| format!("Cannot create the update staging name: {error}"))?;
    let stage = parent.join(format!(
        ".pam-update-{}-{}-{}",
        safe_path_segment(application_id),
        safe_path_segment(version),
        hex(&entropy)
    ));
    fs::create_dir(&stage)
        .map_err(|error| format!("Cannot create the update staging directory: {error}"))?;
    Ok(stage)
}

fn prune_update_stages(parent: &Path, application_id: &str) -> Result<(), String> {
    let prefix = format!(".pam-update-{}-", safe_path_segment(application_id));
    let mut stages: Vec<_> = fs::read_dir(parent)
        .map_err(|error| format!("Cannot inspect the update staging parent: {error}"))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            if !name.starts_with(&prefix) {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            if !metadata.is_dir() || entry.file_type().ok()?.is_symlink() {
                return None;
            }
            Some((metadata.modified().ok(), name, entry.path()))
        })
        .collect();
    stages.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    for (_, _, stage) in stages.into_iter().skip(MAX_RETAINED_UPDATE_STAGES) {
        fs::remove_dir_all(&stage).map_err(|error| {
            format!(
                "Cannot remove the stale update stage {}: {error}",
                stage.display()
            )
        })?;
    }
    Ok(())
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

const fn archive_name() -> &'static str {
    if cfg!(target_os = "linux") {
        "update.tar.gz"
    } else {
        "update.zip"
    }
}

fn launch_apply_helper(
    installed: &Installed,
    archive: &Path,
    sha256: &str,
    parent_pid: u32,
) -> Result<(), String> {
    let current_exe = std::env::current_exe()
        .map_err(|error| format!("Cannot locate the PAM Desktop host: {error}"))?;
    let stage = archive
        .parent()
        .ok_or("The update staging path has no parent.")?;
    // The helper runs from a copy: the bundle it replaces holds the original.
    let helper = stage.join(format!(
        "pam-desktop-update{}",
        std::env::consts::EXE_SUFFIX
    ));
    fs::copy(&current_exe, &helper)
        .map_err(|error| format!("Cannot prepare the update helper: {error}"))?;
    let mut command = Command::new(&helper);
    command
        .arg("apply-update")
        .arg("--parent")
        .arg(parent_pid.to_string())
        .arg("--bundle")
        .arg(&installed.root)
        .arg("--archive")
        .arg(archive)
        .arg("--sha256")
        .arg(sha256)
        .current_dir(installed.root.parent().unwrap_or(&installed.root))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(launcher) = &installed.launcher {
        command.arg("--launcher").arg(launcher);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Cannot start the update helper: {error}"))
}

// ---- keys and feed publication --------------------------------------------

pub struct PublishOptions {
    private_key: PathBuf,
    output: PathBuf,
    channel: String,
    published_at: String,
    notes_url: Option<String>,
    artifacts: Vec<PublishArtifact>,
    expires_in_days: Option<u64>,
    force: bool,
}

struct PublishArtifact {
    platform: Platform,
    architecture: String,
    kind: ArtifactKind,
    path: PathBuf,
    url: String,
}

impl PublishOptions {
    /// Parses the options of `publish-update`.
    ///
    /// # Errors
    ///
    /// For missing or repeated options, malformed artifact tuples and
    /// unknown arguments.
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut private_key = None;
        let mut output = None;
        let mut channel = None;
        let mut published_at = None;
        let mut notes_url = None;
        let mut artifacts = Vec::new();
        let mut expires_in_days = None;
        let mut force = false;
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some("--key") => set_once(
                    &mut private_key,
                    arguments.next().map(PathBuf::from),
                    "--key",
                    "a private key file",
                )?,
                Some("--output" | "-o") => set_once(
                    &mut output,
                    arguments.next().map(PathBuf::from),
                    "--output",
                    "a feed path",
                )?,
                Some("--channel") => set_once(
                    &mut channel,
                    text_value(arguments.next()),
                    "--channel",
                    "a value",
                )?,
                Some("--published-at") => set_once(
                    &mut published_at,
                    text_value(arguments.next()),
                    "--published-at",
                    "an RFC 3339 timestamp",
                )?,
                Some("--notes-url") => set_once(
                    &mut notes_url,
                    text_value(arguments.next()),
                    "--notes-url",
                    "an HTTPS URL",
                )?,
                Some("--artifact") => {
                    let value = arguments
                        .next()
                        .ok_or("--artifact requires a target tuple")?;
                    artifacts.push(parse_publish_artifact(&value)?);
                }
                Some("--expires-in") => set_once(
                    &mut expires_in_days,
                    text_value(arguments.next())
                        .and_then(|days| days.parse::<u64>().ok())
                        .filter(|days| (1..=366).contains(days)),
                    "--expires-in",
                    "a number of days from 1 to 366",
                )?,
                Some("--force") => force = true,
                Some(value) => return Err(format!("unknown publish-update option {value:?}")),
                None => return Err("publish-update options must be valid UTF-8".to_owned()),
            }
        }
        if artifacts.is_empty() {
            return Err("publish-update requires at least one --artifact tuple".to_owned());
        }
        Ok(Self {
            private_key: private_key.ok_or("--key is required")?,
            output: output.ok_or("--output is required")?,
            channel: channel.unwrap_or_else(|| "stable".to_owned()),
            published_at: published_at.ok_or("--published-at is required")?,
            notes_url,
            artifacts,
            expires_in_days,
            force,
        })
    }
}

fn text_value(value: Option<OsString>) -> Option<String> {
    value.and_then(|value| value.into_string().ok())
}

fn set_once<T>(
    target: &mut Option<T>,
    value: Option<T>,
    option: &str,
    expected: &str,
) -> Result<(), String> {
    let value = value.ok_or_else(|| format!("{option} requires {expected}"))?;
    if target.replace(value).is_some() {
        return Err(format!("{option} may only be specified once"));
    }
    Ok(())
}

fn parse_publish_artifact(value: &OsStr) -> Result<PublishArtifact, String> {
    let value = value.to_str().ok_or("artifact tuple must be valid UTF-8")?;
    let fields: Vec<&str> = value.splitn(5, ',').collect();
    if fields.len() != 5 || fields.iter().any(|field| field.is_empty()) {
        return Err("artifact tuple must be platform,architecture,kind,path,url (kind: portable or installer)".to_owned());
    }
    let platform = Platform::parse(fields[0])
        .ok_or_else(|| format!("unknown update artifact platform {:?}", fields[0]))?;
    let kind = match fields[2] {
        "native" => ArtifactKind::NativeInstaller,
        keyword => ArtifactKind::parse(keyword)
            .ok_or_else(|| format!("unknown update artifact kind {keyword:?}"))?,
    };
    Ok(PublishArtifact {
        platform,
        architecture: fields[1].to_owned(),
        kind,
        path: PathBuf::from(fields[3]),
        url: fields[4].to_owned(),
    })
}

/// Creates an Ed25519 seed file and returns the public key. The private key
/// is never printed.
///
/// # Errors
///
/// When the file exists, its directory does not, or it cannot be written.
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
    getrandom::fill(&mut seed)
        .map_err(|error| format!("cannot generate the update signing key: {error}"))?;
    let signing_key = SigningKey::from_bytes(&seed);
    let encoded = format!("{}\n", hex(&seed));
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
        .map_err(|error| format!("cannot persist the update private key: {error}"))?;
    Ok(hex(signing_key.verifying_key().as_bytes()))
}

/// Builds and signs the feed of the application's current version.
///
/// # Errors
///
/// When the key file is readable by others or does not match the public key
/// of the manifest, an artifact is missing or invalid, or the feed cannot be
/// written.
pub fn publish_feed(application: &Application, options: PublishOptions) -> Result<PathBuf, String> {
    let signing_key = read_signing_key(&options.private_key)?;
    let public_key = hex(signing_key.verifying_key().as_bytes());
    if application
        .updates
        .as_ref()
        .is_some_and(|config| config.public_key != public_key)
    {
        return Err(
            "the update signing key does not match updates.publicKey of the application".to_owned(),
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
        let (bytes, sha256) = hash_file(&path)?;
        artifacts.push(Artifact {
            platform: input.platform,
            architecture: input.architecture,
            kind: input.kind,
            url: input.url,
            bytes,
            sha256,
        });
    }
    let release = Release {
        schema_version: FEED_VERSION,
        application_id: application.identifier.clone(),
        channel: options.channel,
        version: application.version.clone(),
        published_at: options.published_at,
        notes_url: options.notes_url,
        artifacts,
        expires_at_unix: options
            .expires_in_days
            .map(|days| unix_now() + days * 24 * 60 * 60),
    };
    release.validate()?;
    let signature = hex(&signing_key.sign(&release.signed_payload()?).to_bytes());
    let mut encoded = serde_json::to_vec_pretty(&SignedRelease { release, signature })
        .map_err(|error| format!("cannot serialize the signed update feed: {error}"))?;
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
        .map_err(|error| format!("cannot read the update private key: {error}"))?;
    Ok(SigningKey::from_bytes(&decode_hex::<32>(
        source.trim(),
        "update private key",
    )?))
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
    getrandom::fill(&mut entropy)
        .map_err(|error| format!("cannot create the feed staging path: {error}"))?;
    let temporary = parent.join(format!(".pam-feed-{}", hex(&entropy)));
    fs::write(&temporary, contents)
        .map_err(|error| format!("cannot write the update feed staging file: {error}"))?;
    if !path.exists() || cfg!(unix) {
        return fs::rename(&temporary, path)
            .map_err(|error| format!("cannot publish the update feed atomically: {error}"));
    }
    // Windows does not rename over an existing file.
    let backup = parent.join(format!(".pam-feed-backup-{}", hex(&entropy)));
    fs::rename(path, &backup)
        .map_err(|error| format!("cannot stage the existing update feed: {error}"))?;
    if let Err(error) = fs::rename(&temporary, path) {
        return Err(match fs::rename(&backup, path) {
            Ok(()) => format!("cannot publish the update feed; rollback succeeded: {error}"),
            Err(rollback) => {
                format!("cannot publish the update feed ({error}) and rollback failed ({rollback})")
            }
        });
    }
    fs::remove_file(&backup)
        .map_err(|error| format!("feed published but the old copy could not be removed: {error}"))
}

// ---- applying an update ---------------------------------------------------

pub struct ApplyOptions {
    parent_pid: u32,
    bundle: PathBuf,
    archive: PathBuf,
    sha256: String,
    launcher: Option<PathBuf>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateTransaction {
    schema_version: u8,
    archive_sha256: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecoveryOutcome {
    Ready,
    AlreadyInstalled,
}

impl ApplyOptions {
    /// Parses the private `apply-update` command.
    ///
    /// # Errors
    ///
    /// For missing, repeated or malformed arguments.
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut values = HashMap::<String, OsString>::new();
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            let key = argument
                .to_str()
                .filter(|value| value.starts_with("--"))
                .ok_or("invalid apply-update option")?
                .to_owned();
            let value = arguments
                .next()
                .ok_or_else(|| format!("{key} requires a value"))?;
            if values.insert(key.clone(), value).is_some() {
                return Err(format!("{key} may only be specified once"));
            }
        }
        let mut take = |key: &str| {
            values
                .remove(key)
                .ok_or_else(|| format!("{key} is required"))
        };
        let parent_pid = take("--parent")?
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|pid| *pid != 0)
            .ok_or("--parent must be a positive process identifier")?;
        let bundle = PathBuf::from(take("--bundle")?);
        let archive = PathBuf::from(take("--archive")?);
        let sha256 = take("--sha256")?
            .to_str()
            .filter(|value| validate_lower_hex(value, 64, "--sha256").is_ok())
            .ok_or("--sha256 must be a lowercase SHA-256 digest")?
            .to_owned();
        let launcher = values.remove("--launcher").map(PathBuf::from);
        if let Some(unknown) = values.keys().next() {
            return Err(format!("unknown apply-update option {unknown}"));
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

/// Replaces the installed bundle with the verified archive once the host
/// that started the helper has exited, then starts the application again.
///
/// # Errors
///
/// Without replacing the application when waiting, archive integrity,
/// extraction, bundle verification or the swap fails; a failed swap is
/// rolled back.
pub fn apply(options: ApplyOptions) -> Result<(), String> {
    let (parent_pid, configured, archive, sha256, launcher) = (
        options.parent_pid,
        options.bundle.clone(),
        options.archive.clone(),
        options.sha256.clone(),
        options.launcher.clone(),
    );
    apply_release(options).inspect_err(|_| {
        recover_from_failed_apply(
            parent_pid,
            &configured,
            &archive,
            &sha256,
            launcher.as_deref(),
        );
    })
}

/// The application closed for an update that did not install: what is in
/// place (the installed bundle, or the one a rollback restored) stays, so it
/// starts again. The release is remembered as rejected, so that the
/// `automatic` policy does not download it and close the application at
/// every start, and the staged archive and extraction are removed.
fn recover_from_failed_apply(
    parent_pid: u32,
    configured: &Path,
    archive: &Path,
    sha256: &str,
    launcher: Option<&Path>,
) {
    let Ok(bundle) = resolve_bundle_target(configured) else {
        return;
    };
    if let Ok(marker) = rejected_release_path(&bundle) {
        let _ = fs::write(marker, sha256);
    }
    let _ = cleanup_update_stage(archive);
    // Started while the application still runs, the launch would only hand
    // itself to it.
    if bundle.is_dir() && wait_for_parent(parent_pid).is_ok() {
        let _ = relaunch(&bundle, launcher);
    }
}

/// Beside the bundle: the SHA-256 of the last release its installation
/// refused.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn feed_watermark_path(bundle: &Path) -> Result<PathBuf, String> {
    let parent = bundle
        .parent()
        .ok_or("the installed bundle has no parent")?;
    let name = bundle
        .file_name()
        .ok_or("the installed bundle has no file name")?
        .to_string_lossy();
    Ok(parent.join(format!(".{name}.update-feed-seen")))
}

/// Refuses an expired feed and, with a `watermark` file, a feed whose
/// version is older than one a verified feed of the same channel already
/// offered (a replayed or rolled back feed); then raises the watermark.
fn check_feed_freshness(
    release: &Release,
    now: u64,
    watermark: Option<&Path>,
) -> Result<(), String> {
    if let Some(expires) = release.expires_at_unix
        && expires <= now
    {
        return Err(format!(
            "The update feed expired (expiresAtUnix {expires}); the server is serving an outdated feed."
        ));
    }
    let Some(watermark) = watermark else {
        return Ok(());
    };
    let recorded = fs::read_to_string(watermark).ok();
    let seen = recorded.as_deref().and_then(|text| {
        let (channel, version) = text.trim_end().split_once('\n')?;
        (channel == release.channel && validate_version(version).is_ok()).then_some(version)
    });
    if let Some(seen) = seen {
        match compare_versions(&release.version, seen) {
            Ordering::Less => {
                return Err(format!(
                    "The update feed offers {}, older than {seen} offered before; refusing a replayed feed.",
                    release.version
                ));
            }
            Ordering::Equal => return Ok(()),
            Ordering::Greater => {}
        }
    }
    // Best effort: a read-only installation still gets the expiry check.
    let _ = atomic_write(
        watermark,
        format!("{}\n{}\n", release.channel, release.version).as_bytes(),
        true,
    );
    Ok(())
}

fn rejected_release_path(bundle: &Path) -> Result<PathBuf, String> {
    let parent = bundle
        .parent()
        .ok_or("the installed bundle has no parent")?;
    let name = bundle
        .file_name()
        .ok_or("the installed bundle has no file name")?
        .to_string_lossy();
    Ok(parent.join(format!(".{name}.rejected-update")))
}

/// Whether the installation of this exact archive already failed here.
fn rejected_release(bundle: &Path, sha256: &str) -> bool {
    rejected_release_path(bundle)
        .and_then(|path| fs::read(path).map_err(|error| error.to_string()))
        .is_ok_and(|recorded| recorded == sha256.as_bytes())
}

fn apply_release(options: ApplyOptions) -> Result<(), String> {
    let ApplyOptions {
        parent_pid,
        bundle: configured_bundle,
        archive: configured_archive,
        sha256,
        launcher,
    } = options;
    let bundle = resolve_bundle_target(&configured_bundle)?;
    let archive = configured_archive
        .canonicalize()
        .map_err(|error| format!("cannot resolve the staged update archive: {error}"))?;
    verify_file_hash(&archive, &sha256)?;
    wait_for_parent(parent_pid)?;
    if reconcile_interrupted_update(&bundle, &sha256)? == RecoveryOutcome::AlreadyInstalled {
        cleanup_update_stage(&archive)?;
        return relaunch(&bundle, launcher.as_deref());
    }
    let stage = archive
        .parent()
        .ok_or("the staged update archive has no parent")?;
    let expanded = stage.join("expanded");
    if expanded.exists() {
        fs::remove_dir_all(&expanded)
            .map_err(|error| format!("cannot reset the update extraction directory: {error}"))?;
    }
    fs::create_dir(&expanded)
        .map_err(|error| format!("cannot create the update extraction directory: {error}"))?;
    extract_archive(&archive, &expanded)?;
    let replacement = single_extracted_root(&expanded)?;
    let replacing = verify_replacement(&replacement)?;
    // A release of another application must never take this one's place,
    // whatever the feed said.
    if let Ok(current) = bundle::read_manifest(&Installed::runtime_directory(&bundle))
        && current.application.identifier != replacing.application.identifier
    {
        return Err("the update archive holds a different application".to_owned());
    }

    let (backup, failed, transaction) = transaction_paths(&bundle)?;
    if backup.exists() {
        remove_scoped_directory(&backup, "previous update backup")?;
    }
    if failed.exists() {
        remove_scoped_directory(&failed, "failed update bundle")?;
    }
    write_update_transaction(&transaction, &sha256)?;
    if let Err(error) = rename_when_released(&bundle, &backup) {
        let _ = fs::remove_file(&transaction);
        return Err(format!(
            "cannot move the installed bundle to backup: {error}"
        ));
    }
    if let Err(error) = fs::rename(&replacement, &bundle) {
        return Err(match fs::rename(&backup, &bundle) {
            Ok(()) => {
                let _ = fs::remove_file(&transaction);
                format!("cannot install the replacement bundle; rollback succeeded: {error}")
            }
            Err(rollback) => format!(
                "cannot install the replacement bundle ({error}) and rollback failed ({rollback})"
            ),
        });
    }
    verify_replacement(&bundle).map_err(|error| {
        let _ = fs::rename(&bundle, &failed);
        match fs::rename(&backup, &bundle) {
            Ok(()) => {
                let _ = fs::remove_file(&transaction);
                format!("installed bundle verification failed; rollback succeeded: {error}")
            }
            Err(rollback) => format!(
                "installed bundle verification failed ({error}) and rollback failed ({rollback})"
            ),
        }
    })?;
    fs::remove_file(&transaction).map_err(|error| {
        format!("update installed but the transaction could not be removed: {error}")
    })?;
    if let Ok(marker) = rejected_release_path(&bundle) {
        let _ = fs::remove_file(marker);
    }
    cleanup_update_stage(&archive)?;
    relaunch(&bundle, launcher.as_deref())
}

/// Moves the installed bundle aside. The PHP worker of the old host may
/// still be exiting, and Windows refuses to move a directory with running
/// executables, so a refusal is retried for a few seconds.
fn rename_when_released(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut attempt = 0;
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error)
                if attempt + 1 < SWAP_ATTEMPTS
                    && error.kind() == std::io::ErrorKind::PermissionDenied
                    && cfg!(windows) =>
            {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(error) => return Err(error),
        }
    }
}

fn resolve_bundle_target(configured: &Path) -> Result<PathBuf, String> {
    if !configured.is_absolute() {
        return Err("the installed bundle path must be absolute".to_owned());
    }
    let parent = configured
        .parent()
        .ok_or("the installed bundle has no parent")?
        .canonicalize()
        .map_err(|error| format!("cannot resolve the parent of the installed bundle: {error}"))?;
    let name = configured
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or("the installed bundle has no file name")?;
    let bundle = parent.join(name);
    if let Ok(metadata) = bundle.symlink_metadata()
        && (metadata.file_type().is_symlink() || !metadata.is_dir())
    {
        return Err("the installed bundle must be a regular directory, never a symlink".to_owned());
    }
    Ok(bundle)
}

/// The backup, quarantine and journal paths beside a bundle.
fn transaction_paths(bundle: &Path) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let parent = bundle
        .parent()
        .ok_or("the installed bundle has no parent")?;
    let name = bundle
        .file_name()
        .ok_or("the installed bundle has no file name")?
        .to_string_lossy();
    Ok((
        parent.join(format!(".{name}.previous")),
        parent.join(format!(".{name}.failed")),
        parent.join(format!(".{name}.update-transaction.json")),
    ))
}

fn write_update_transaction(path: &Path, sha256: &str) -> Result<(), String> {
    let transaction = UpdateTransaction {
        schema_version: 1,
        archive_sha256: sha256.to_owned(),
    };
    let bytes = serde_json::to_vec(&transaction)
        .map_err(|error| format!("cannot encode the update transaction: {error}"))?;
    let temporary = path.with_extension(format!("json.tmp.{}", std::process::id()));
    if temporary.exists() {
        fs::remove_file(&temporary).map_err(|error| {
            format!("cannot reset the update transaction staging file: {error}")
        })?;
    }
    let result = (|| {
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| format!("cannot create the update transaction: {error}"))?;
        output
            .write_all(&bytes)
            .and_then(|()| output.sync_all())
            .map_err(|error| format!("cannot persist the update transaction: {error}"))?;
        fs::rename(&temporary, path)
            .map_err(|error| format!("cannot publish the update transaction: {error}"))
    })();
    let _ = fs::remove_file(&temporary);
    result
}

/// Whether a journal of this very archive exists.
fn read_update_transaction(path: &Path, expected_sha256: &str) -> Result<bool, String> {
    let metadata = match path.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("cannot inspect the update transaction: {error}")),
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_TRANSACTION_BYTES
    {
        return Err("the update transaction is not a bounded regular file".to_owned());
    }
    let bytes =
        fs::read(path).map_err(|error| format!("cannot read the update transaction: {error}"))?;
    let transaction: UpdateTransaction = serde_json::from_slice(&bytes)
        .map_err(|error| format!("cannot decode the update transaction: {error}"))?;
    if transaction.schema_version != 1 || transaction.archive_sha256 != expected_sha256 {
        return Err("the update transaction does not match the staged archive".to_owned());
    }
    Ok(true)
}

/// Brings the file system back to a known state after a helper that stopped
/// in the middle of a swap, instead of repeating the swap blindly.
fn reconcile_interrupted_update(
    bundle: &Path,
    expected_sha256: &str,
) -> Result<RecoveryOutcome, String> {
    let (backup, failed, transaction) = transaction_paths(bundle)?;
    if !read_update_transaction(&transaction, expected_sha256)? {
        if !bundle.is_dir() {
            return Err(
                "the installed bundle is missing and no recovery transaction exists".to_owned(),
            );
        }
        return Ok(RecoveryOutcome::Ready);
    }
    let finish = |label: &str| {
        fs::remove_file(&transaction)
            .map_err(|error| format!("cannot finish the {label} transaction: {error}"))
    };
    let bundle_exists = scoped_directory_exists(bundle, "installed bundle")?;
    let backup_exists = scoped_directory_exists(&backup, "update recovery backup")?;
    match (bundle_exists, backup_exists) {
        (true, true) if verify_replacement(bundle).is_ok() => {
            finish("recovered update")?;
            Ok(RecoveryOutcome::AlreadyInstalled)
        }
        (true, true) => {
            verify_replacement(&backup).map_err(|error| format!("the update backup cannot be recovered: {error}"))?;
            if failed.exists() {
                remove_scoped_directory(&failed, "failed update bundle")?;
            }
            fs::rename(bundle, &failed).map_err(|error| format!("cannot quarantine the interrupted replacement: {error}"))?;
            fs::rename(&backup, bundle).map_err(|error| format!("cannot restore the interrupted update backup: {error}"))?;
            finish("update rollback")?;
            Ok(RecoveryOutcome::Ready)
        }
        (false, true) => {
            verify_replacement(&backup).map_err(|error| format!("the update backup cannot be recovered: {error}"))?;
            fs::rename(&backup, bundle).map_err(|error| format!("cannot restore the interrupted update backup: {error}"))?;
            finish("update recovery")?;
            Ok(RecoveryOutcome::Ready)
        }
        (true, false) => {
            verify_replacement(bundle).map_err(|error| format!("the installed bundle cannot resume the update: {error}"))?;
            finish("unstarted update")?;
            Ok(RecoveryOutcome::Ready)
        }
        (false, false) => Err("the installed bundle and its recovery backup are both missing; refusing destructive recovery".to_owned()),
    }
}

fn remove_scoped_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata = path
        .symlink_metadata()
        .map_err(|error| format!("cannot inspect the {label}: {error}"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(format!("the {label} must be a regular directory"));
    }
    fs::remove_dir_all(path).map_err(|error| format!("cannot remove the {label}: {error}"))
}

fn scoped_directory_exists(path: &Path, label: &str) -> Result<bool, String> {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(format!("the {label} must be a regular directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("cannot inspect the {label}: {error}")),
    }
}

fn cleanup_update_stage(archive: &Path) -> Result<(), String> {
    let stage = archive
        .parent()
        .ok_or("the staged update archive has no parent")?;
    let expanded = stage.join("expanded");
    if expanded.exists() {
        remove_scoped_directory(&expanded, "update extraction directory")?;
    }
    if archive.exists() {
        fs::remove_file(archive)
            .map_err(|error| format!("cannot remove the installed update archive: {error}"))?;
    }
    Ok(())
}

fn relaunch(bundle: &Path, launcher: Option<&Path>) -> Result<(), String> {
    let Some(launcher) = launcher else {
        return Ok(());
    };
    validate_relative_path(launcher, "update relaunch path")?;
    Command::new(bundle.join(launcher))
        .current_dir(bundle)
        .spawn()
        .map(|_| ())
        .map_err(|error| {
            format!("update installed but the application could not be started again: {error}")
        })
}

/// Verifies a bundle directory, or on macOS the signature of an `.app` and
/// the bundle inside it.
fn verify_replacement(root: &Path) -> Result<bundle::BundleManifest, String> {
    if root.extension() == Some(OsStr::new("app")) {
        if !cfg!(target_os = "macos") {
            return Err(
                "macOS application updates cannot be installed on this platform".to_owned(),
            );
        }
        let status = Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(root)
            .status()
            .map_err(|error| format!("cannot start the macOS signature verification: {error}"))?;
        if !status.success() {
            return Err(format!(
                "macOS application signature verification failed with status {status}"
            ));
        }
    }
    bundle::verify(&Installed::runtime_directory(root))
}

fn wait_for_parent(parent_pid: u32) -> Result<(), String> {
    let started = Instant::now();
    while process_exists(parent_pid)? {
        if started.elapsed() > PARENT_EXIT_TIMEOUT {
            return Err("timed out waiting for the application to exit".to_owned());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}

fn process_exists(pid: u32) -> Result<bool, String> {
    if cfg!(windows) {
        let output = crate::console::hidden(&mut Command::new(system32().join("tasklist.exe")))
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .map_err(|error| format!("cannot inspect the parent process with tasklist: {error}"))?;
        Ok(String::from_utf8_lossy(&output.stdout).contains(&pid.to_string()))
    } else {
        let status = Command::new("/bin/ps")
            .args(["-p", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| format!("cannot inspect the parent process with ps: {error}"))?;
        Ok(status.success())
    }
}

/// `%SystemRoot%\\System32`, where Windows keeps the programs used here.
fn system32() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
        .join("System32")
}

fn extract_archive(archive: &Path, destination: &Path) -> Result<(), String> {
    if cfg!(target_os = "linux") {
        return extract_tar_gz(archive, destination);
    }
    let status = if cfg!(target_os = "macos") {
        Command::new("/usr/bin/ditto")
            .args(["-x", "-k"])
            .arg(archive)
            .arg(destination)
            .status()
    } else if cfg!(windows) {
        crate::console::hidden(&mut Command::new(
            system32().join(r"WindowsPowerShell\v1.0\powershell.exe"),
        ))
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Expand-Archive -LiteralPath $env:PAM_UPDATE_ARCHIVE -DestinationPath $env:PAM_UPDATE_DESTINATION -Force",
        ])
        .env("PAM_UPDATE_ARCHIVE", archive)
        .env("PAM_UPDATE_DESTINATION", destination)
        .status()
    } else {
        return Err("portable update extraction is unsupported on this platform".to_owned());
    }
    .map_err(|error| format!("cannot start the portable archive extraction: {error}"))?;
    if !status.success() {
        return Err(format!(
            "portable archive extraction failed with status {status}"
        ));
    }
    Ok(())
}

/// Unpacks a portable Linux archive in Rust: regular files, folders and
/// relative symlinks that stay inside the archive; never owners, setuid
/// bits, extended attributes, hard links or device files.
fn extract_tar_gz(archive: &Path, destination: &Path) -> Result<(), String> {
    use std::path::Component;
    let file =
        File::open(archive).map_err(|error| format!("cannot read the update archive: {error}"))?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    archive.set_preserve_permissions(false);
    archive.set_preserve_ownerships(false);
    archive.set_unpack_xattrs(false);
    archive.set_overwrite(false);
    let entries = archive
        .entries()
        .map_err(|error| format!("the update archive is unreadable: {error}"))?;
    for entry in entries {
        let mut entry =
            entry.map_err(|error| format!("the update archive is unreadable: {error}"))?;
        let path = entry
            .path()
            .map_err(|error| format!("the update archive has an unreadable path: {error}"))?
            .into_owned();
        if path
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
        {
            return Err(format!(
                "the update archive has a path outside its folder: {}",
                path.display()
            ));
        }
        let kind = entry.header().entry_type();
        if kind.is_symlink() {
            let target = entry
                .link_name()
                .ok()
                .flatten()
                .ok_or("the update archive has a symlink without a target")?;
            // Depth of the link's folder; `..` may climb at most that far.
            let mut depth = path.components().count().saturating_sub(1) as isize;
            for component in target.components() {
                match component {
                    Component::Normal(_) => depth += 1,
                    Component::CurDir => {}
                    Component::ParentDir => depth -= 1,
                    _ => depth = -1,
                }
                if depth < 0 {
                    return Err(format!(
                        "the update archive has a symlink leaving it: {}",
                        path.display()
                    ));
                }
            }
        } else if !kind.is_file() && !kind.is_dir() {
            return Err(format!(
                "the update archive has an unexpected special entry: {}",
                path.display()
            ));
        }
        if !entry
            .unpack_in(destination)
            .map_err(|error| format!("cannot unpack {}: {error}", path.display()))?
        {
            return Err(format!(
                "the update archive escapes its folder: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn single_extracted_root(directory: &Path) -> Result<PathBuf, String> {
    let mut entries = fs::read_dir(directory)
        .and_then(Iterator::collect::<Result<Vec<_>, _>>)
        .map_err(|error| format!("cannot inspect the extracted update: {error}"))?;
    let (Some(entry), true) = (entries.pop(), entries.is_empty()) else {
        return Err("a portable update must contain exactly one bundle directory".to_owned());
    };
    if !entry
        .file_type()
        .map_err(|error| format!("cannot inspect the extracted update root: {error}"))?
        .is_dir()
    {
        return Err("the root of a portable update must be a directory".to_owned());
    }
    Ok(entry.path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::test_support::write_bundle;
    use crate::http_client::UreqClient;
    use pam_desktop_ui::modules::test_support::{Served, TestServer, temporary_directory};

    const APPLICATION: &str = "com.pushin.test";

    fn release(version: &str, url: &str, artifact: &[u8]) -> Release {
        Release {
            schema_version: FEED_VERSION,
            application_id: APPLICATION.to_owned(),
            channel: "stable".to_owned(),
            version: version.to_owned(),
            published_at: "2026-07-23T14:00:00Z".to_owned(),
            notes_url: None,
            artifacts: vec![Artifact {
                platform: Platform::current().expect("supported test platform"),
                architecture: std::env::consts::ARCH.to_owned(),
                kind: ArtifactKind::PortableArchive,
                url: url.to_owned(),
                bytes: artifact.len() as u64,
                sha256: hex(&Sha256::digest(artifact)),
            }],
            expires_at_unix: None,
        }
    }

    fn sign(signing_key: &SigningKey, release: Release) -> SignedRelease {
        let signature = hex(&signing_key
            .sign(&release.signed_payload().unwrap())
            .to_bytes());
        SignedRelease { release, signature }
    }

    fn public_key(signing_key: &SigningKey) -> String {
        hex(signing_key.verifying_key().as_bytes())
    }

    fn application(version: &str, public_key: Option<String>) -> Application {
        Application {
            identifier: APPLICATION.to_owned(),
            name: "Pam Test".to_owned(),
            version: version.to_owned(),
            build: String::new(),
            entry: "index.php".to_owned(),
            description: String::new(),
            publisher: "Pushin".to_owned(),
            category: crate::manifest::Category::Development,
            icon: None,
            exclude: Vec::new(),
            extensions: None,
            url_schemes: Vec::new(),
            updates: public_key.map(|public_key| UpdateConfig {
                endpoint: "https://updates.pushin.dev/latest.json".to_owned(),
                public_key,
                channel: "stable".to_owned(),
                policy: UpdatePolicy::Manual,
            }),
        }
    }

    fn updater(
        endpoint: String,
        public_key: String,
        policy: UpdatePolicy,
        installed: Option<Installed>,
    ) -> (Updater, Arc<Mutex<Vec<Snapshot>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let config = UpdateConfig {
            endpoint,
            public_key,
            channel: "stable".to_owned(),
            policy,
        };
        let updater = Updater::new(
            APPLICATION,
            "0.5.0",
            Some(config),
            installed,
            Arc::new(UreqClient::new()),
            move |snapshot| {
                sink.lock().unwrap().push(snapshot.clone());
            },
        );
        (updater, seen)
    }

    #[test]
    fn compares_release_versions_without_build_metadata() {
        assert!(version_is_newer("0.5.1", "0.5.0"));
        assert!(version_is_newer("1.0.0", "1.0.0-rc.2"));
        assert!(version_is_newer("1.0.0-rc.10", "1.0.0-rc.2"));
        assert!(version_is_newer("2", "1.9.9"));
        assert!(!version_is_newer("1.0.0+build.2", "1.0.0+build.1"));
        assert!(!version_is_newer("0.4.9", "0.5.0"));
        assert!(!version_is_newer("1.0.0-beta", "1.0.0"));
    }

    #[test]
    fn verifies_canonical_ed25519_release_payloads() {
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let feed = sign(
            &signing_key,
            release(
                "0.5.1",
                "https://updates.pushin.dev/test.tar.gz",
                b"artifact",
            ),
        );
        verify_signed_feed(&feed, &public_key(&signing_key)).expect("signature should verify");
        // The feed survives a round trip through its JSON document.
        let document = serde_json::to_string_pretty(&feed).unwrap();
        let parsed: SignedRelease = serde_json::from_str(&document).unwrap();
        assert_eq!(parsed, feed);
        verify_signed_feed(&parsed, &public_key(&signing_key)).expect("parsed feed should verify");
        // Platforms and kinds travel as the integers 1.x feeds use.
        assert!(document.contains("\"schemaVersion\": 1"));
        assert!(document.contains(&format!(
            "\"platform\": {}",
            Platform::current().unwrap() as u8
        )));
        assert!(document.contains("\"kind\": 1"));
    }

    #[test]
    fn rejects_tampered_and_foreign_release_payloads() {
        let signing_key = SigningKey::from_bytes(&[9_u8; 32]);
        let mut feed = sign(
            &signing_key,
            release(
                "0.5.1",
                "https://updates.pushin.dev/test.tar.gz",
                b"artifact",
            ),
        );
        let other_key = public_key(&SigningKey::from_bytes(&[10_u8; 32]));
        assert!(
            verify_signed_feed(&feed, &other_key)
                .unwrap_err()
                .contains("did not verify")
        );
        feed.release.version = "9.9.9".to_owned();
        assert!(
            verify_signed_feed(&feed, &public_key(&signing_key))
                .unwrap_err()
                .contains("did not verify")
        );

        let mut insecure = release(
            "0.5.1",
            "http://updates.pushin.dev/test.tar.gz",
            b"artifact",
        );
        assert!(insecure.validate().is_err());
        insecure.artifacts[0].url = "https://updates.pushin.dev/test.tar.gz".to_owned();
        insecure.artifacts.push(insecure.artifacts[0].clone());
        assert!(insecure.validate().unwrap_err().contains("duplicated"));
        let unknown = r#"{"release":{"schemaVersion":1,"applicationId":"a.b","channel":"stable","version":"1","publishedAt":"x","artifacts":[],"extra":1},"signature":""}"#;
        assert!(serde_json::from_str::<SignedRelease>(unknown).is_err());
        let platform = r#"{"platform":9,"architecture":"x86_64","kind":1,"url":"https://a.b/c","bytes":1,"sha256":""}"#;
        assert!(serde_json::from_str::<Artifact>(platform).is_err());
    }

    #[test]
    fn generates_keys_and_publishes_verifiable_multi_platform_feeds() {
        let root = temporary_directory("update-publish");
        let private_key = root.join("release.key");
        let public = generate_key(&private_key).expect("key should be generated");
        assert!(
            generate_key(&private_key)
                .unwrap_err()
                .contains("already exists")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&private_key).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let linux = root.join("application.tar.gz");
        let windows = root.join("application.zip");
        fs::write(&linux, b"linux artifact").unwrap();
        fs::write(&windows, b"Windows artifact").unwrap();
        let output = root.join("latest.json");
        let arguments = |output: &Path| {
            [
                "--key".into(),
                private_key.clone().into_os_string(),
                "--output".into(),
                output.as_os_str().to_owned(),
                "--published-at".into(),
                "2026-07-23T14:00:00Z".into(),
                "--notes-url".into(),
                "https://updates.pushin.dev/releases/0.5.0".into(),
                "--artifact".into(),
                OsString::from(format!(
                    "linux,x86_64,portable,{},https://updates.pushin.dev/application.tar.gz",
                    linux.display()
                )),
                "--artifact".into(),
                OsString::from(format!(
                    "windows,x86_64,portable,{},https://updates.pushin.dev/application.zip",
                    windows.display()
                )),
            ]
        };
        let manifest = application("0.5.0", Some(public.clone()));
        publish_feed(
            &manifest,
            PublishOptions::parse(arguments(&output)).unwrap(),
        )
        .expect("feed should be published");
        let feed: SignedRelease =
            serde_json::from_slice(&fs::read(&output).unwrap()).expect("feed should be valid JSON");
        verify_signed_feed(&feed, &public).expect("published feed should verify");
        assert_eq!(feed.release.application_id, APPLICATION);
        assert_eq!(feed.release.version, "0.5.0");
        assert_eq!(feed.release.channel, "stable");
        assert_eq!(feed.release.artifacts.len(), 2);
        assert_eq!(feed.release.artifacts[0].platform, Platform::Linux);
        assert_eq!(feed.release.artifacts[0].bytes, 14);
        assert_eq!(
            feed.release.artifacts[0].sha256,
            hex(&Sha256::digest(b"linux artifact"))
        );
        assert_eq!(feed.release.artifacts[1].platform, Platform::Windows);

        // An existing feed needs --force; a foreign key is refused.
        assert!(
            publish_feed(
                &manifest,
                PublishOptions::parse(arguments(&output)).unwrap()
            )
            .unwrap_err()
            .contains("--force")
        );
        let mut forced: Vec<OsString> = arguments(&output).into();
        forced.push("--force".into());
        publish_feed(&manifest, PublishOptions::parse(forced).unwrap())
            .expect("forced publication");
        let foreign = application("0.5.0", Some("ab".repeat(32)));
        let error = publish_feed(
            &foreign,
            PublishOptions::parse(arguments(&root.join("other.json"))).unwrap(),
        )
        .unwrap_err();
        assert!(error.contains("does not match"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&private_key, fs::Permissions::from_mode(0o644)).unwrap();
            let error = publish_feed(
                &manifest,
                PublishOptions::parse(arguments(&root.join("other.json"))).unwrap(),
            )
            .unwrap_err();
            assert!(error.contains("too broad"));
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn publish_options_require_their_fields() {
        let parse =
            |arguments: &[&str]| PublishOptions::parse(arguments.iter().map(OsString::from)).err();
        assert!(
            parse(&["--key", "k", "--output", "o", "--published-at", "t"])
                .unwrap()
                .contains("--artifact")
        );
        assert!(
            parse(&[
                "--output",
                "o",
                "--published-at",
                "t",
                "--artifact",
                "linux,x86_64,portable,a,b"
            ])
            .unwrap()
            .contains("--key")
        );
        assert!(
            parse(&["--key", "k", "--key", "k"])
                .unwrap()
                .contains("once")
        );
        assert!(
            parse(&["--artifact", "plan9,x86_64,portable,a,b"])
                .unwrap()
                .contains("platform")
        );
        assert!(
            parse(&["--artifact", "linux,x86_64,floppy,a,b"])
                .unwrap()
                .contains("kind")
        );
        assert!(
            parse(&["--artifact", "linux,x86_64,portable,a"])
                .unwrap()
                .contains("tuple")
        );
        assert!(parse(&["--verbose"]).unwrap().contains("unknown"));
    }

    #[test]
    fn apply_options_are_strict() {
        let parse = |arguments: &[&str]| ApplyOptions::parse(arguments.iter().map(OsString::from));
        let digest = "c".repeat(64);
        let options = parse(&[
            "--parent",
            "42",
            "--bundle",
            "/opt/app",
            "--archive",
            "/tmp/a",
            "--sha256",
            &digest,
            "--launcher",
            "bin/app",
        ])
        .unwrap();
        assert_eq!(
            (options.parent_pid, options.launcher.as_deref()),
            (42, Some(Path::new("bin/app")))
        );
        assert!(
            parse(&[
                "--parent",
                "0",
                "--bundle",
                "/b",
                "--archive",
                "/a",
                "--sha256",
                &digest
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "--parent",
                "1",
                "--bundle",
                "/b",
                "--archive",
                "/a",
                "--sha256",
                "ABC"
            ])
            .is_err()
        );
        assert!(parse(&["--parent", "1", "--bundle", "/b", "--archive", "/a"]).is_err());
        assert!(
            parse(&[
                "--parent",
                "1",
                "--bundle",
                "/b",
                "--archive",
                "/a",
                "--sha256",
                &digest,
                "--extra",
                "x"
            ])
            .is_err()
        );
    }

    #[cfg(target_os = "linux")]
    fn archive_of(staged_root: &Path, name: &str, archive: &Path) -> String {
        let status = Command::new("tar")
            .arg("-czf")
            .arg(archive)
            .arg("-C")
            .arg(staged_root)
            .arg(name)
            .status()
            .expect("tar should start");
        assert!(status.success());
        hash_file(archive).expect("archive should hash").1
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn swaps_verified_bundles_and_keeps_a_recoverable_rollback() {
        let root = temporary_directory("update-apply");
        let installed = root.join("application");
        let staged_root = root.join("staged");
        fs::create_dir(&staged_root).unwrap();
        write_bundle(&installed, APPLICATION, "0.5.0", b"old");
        write_bundle(
            &staged_root.join("application-0.5.1-linux-x86_64"),
            APPLICATION,
            "0.5.1",
            b"new",
        );
        let archive = root.join("update.tar.gz");
        let sha256 = archive_of(&staged_root, "application-0.5.1-linux-x86_64", &archive);

        apply(ApplyOptions {
            parent_pid: u32::MAX,
            bundle: installed.clone(),
            archive,
            sha256,
            launcher: None,
        })
        .expect("verified update should apply");

        assert_eq!(fs::read(installed.join("app/payload.txt")).unwrap(), b"new");
        assert_eq!(
            bundle::verify(&installed).unwrap().application.version,
            "0.5.1"
        );
        assert_eq!(
            fs::read(root.join(".application.previous/app/payload.txt")).unwrap(),
            b"old"
        );
        assert!(!root.join("update.tar.gz").exists());
        assert!(!root.join("expanded").exists());
        assert!(!root.join(".application.update-transaction.json").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn refuses_archives_that_are_not_the_signed_bundle_of_this_application() {
        let root = temporary_directory("update-refuse");
        let installed = root.join("application");
        write_bundle(&installed, APPLICATION, "0.5.0", b"old");
        let apply_archive = |name: &str, prepare: &dyn Fn(&Path), sha256: Option<String>| {
            let staged_root = root.join(format!("staged-{name}"));
            fs::create_dir_all(&staged_root).unwrap();
            prepare(&staged_root.join("replacement"));
            let stage = root.join(format!("stage-{name}"));
            fs::create_dir_all(&stage).unwrap();
            let archive = stage.join("update.tar.gz");
            let actual = archive_of(&staged_root, "replacement", &archive);
            apply(ApplyOptions {
                parent_pid: u32::MAX,
                bundle: installed.clone(),
                archive,
                sha256: sha256.unwrap_or(actual),
                launcher: None,
            })
        };

        let error = apply_archive(
            "digest",
            &|path| write_bundle(path, APPLICATION, "0.5.1", b"new"),
            Some("d".repeat(64)),
        )
        .unwrap_err();
        assert!(error.contains("SHA-256 mismatch"), "{error}");
        let error = apply_archive(
            "tampered",
            &|path| {
                write_bundle(path, APPLICATION, "0.5.1", b"new");
                fs::write(path.join("app/payload.txt"), b"evil").unwrap();
            },
            None,
        )
        .unwrap_err();
        assert!(error.contains("mismatch"), "{error}");
        let error = apply_archive(
            "foreign",
            &|path| write_bundle(path, "com.pushin.other", "0.5.1", b"new"),
            None,
        )
        .unwrap_err();
        assert!(error.contains("different application"), "{error}");

        // Nothing was swapped, journaled or left behind as a backup.
        assert_eq!(fs::read(installed.join("app/payload.txt")).unwrap(), b"old");
        assert!(!root.join(".application.previous").exists());
        assert!(!root.join(".application.update-transaction.json").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_refused_update_restarts_the_installed_bundle_and_is_not_retried_automatically() {
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
        let root = temporary_directory("update-refused-relaunch");
        let installed = root.join("application");
        write_bundle(&installed, APPLICATION, "0.5.0", b"old");
        fs::create_dir_all(installed.join("bin")).unwrap();
        let launcher = installed.join("bin/app");
        fs::write(&launcher, "#!/bin/sh\necho started > \"$0.started\"\n").unwrap();
        fs::set_permissions(
            &launcher,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        let staged_root = root.join("staged");
        write_bundle(
            &staged_root.join("replacement"),
            APPLICATION,
            "0.5.1",
            b"new",
        );
        fs::write(staged_root.join("replacement/app/payload.txt"), b"evil").unwrap();
        let stage = root.join(".pam-update-stage");
        fs::create_dir(&stage).unwrap();
        let archive = stage.join("update.tar.gz");
        let sha256 = archive_of(&staged_root, "replacement", &archive);

        let error = apply(ApplyOptions {
            parent_pid: u32::MAX,
            bundle: installed.clone(),
            archive: archive.clone(),
            sha256: sha256.clone(),
            launcher: Some(PathBuf::from("bin/app")),
        })
        .unwrap_err();
        assert!(error.contains("mismatch"), "{error}");
        // The application that closed for the update starts again, unchanged.
        let started = installed.join("bin/app.started");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !started.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(started.exists(), "the installed bundle was not relaunched");
        assert_eq!(fs::read(installed.join("app/payload.txt")).unwrap(), b"old");
        assert!(!archive.exists() && !stage.join("expanded").exists());
        assert!(rejected_release(&installed, &sha256));
        assert!(!rejected_release(&installed, &"e".repeat(64)));

        // The automatic policy finds the same release and leaves it alone.
        let artifact = b"refused release".to_vec();
        let signing_key = SigningKey::from_bytes(&[13_u8; 32]);
        let served = artifact.clone();
        let downloads = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&downloads);
        let feed = Arc::new(Mutex::new(Vec::new()));
        let served_feed = Arc::clone(&feed);
        let server = TestServer::start(move |request| match request.path.as_str() {
            "/feed" => Served::ok(served_feed.lock().unwrap().clone()),
            "/artifact" => {
                counted.fetch_add(1, AtomicOrdering::SeqCst);
                Served::ok(served.clone())
            }
            _ => Served::status(404, ""),
        });
        *feed.lock().unwrap() = serde_json::to_vec(&sign(
            &signing_key,
            release("0.5.1", &server.url("/artifact"), &artifact),
        ))
        .unwrap();
        fs::write(
            rejected_release_path(&installed).unwrap(),
            hex(&Sha256::digest(&artifact)),
        )
        .unwrap();
        let (updater, _) = updater(
            server.url("/feed"),
            public_key(&signing_key),
            UpdatePolicy::Automatic,
            Some(Installed {
                root: installed.clone(),
                launcher: Some(PathBuf::from("bin/app")),
            }),
        );
        assert!(!updater.run_policy());
        assert_eq!(updater.snapshot().status, Status::Available);
        assert_eq!(downloads.load(AtomicOrdering::SeqCst), 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn recovers_every_interrupted_swap_state_idempotently() {
        let digest = "b".repeat(64);

        let missing_root = temporary_directory("update-recover-missing");
        let missing_bundle = missing_root.join("application");
        write_bundle(&missing_bundle, APPLICATION, "0.5.0", b"old");
        let (missing_backup, _, missing_transaction) = transaction_paths(&missing_bundle).unwrap();
        write_update_transaction(&missing_transaction, &digest).unwrap();
        fs::rename(&missing_bundle, &missing_backup).unwrap();
        assert_eq!(
            reconcile_interrupted_update(&missing_bundle, &digest),
            Ok(RecoveryOutcome::Ready)
        );
        assert_eq!(
            fs::read(missing_bundle.join("app/payload.txt")).unwrap(),
            b"old"
        );
        assert!(!missing_transaction.exists());

        let installed_root = temporary_directory("update-recover-installed");
        let installed_bundle = installed_root.join("application");
        write_bundle(&installed_bundle, APPLICATION, "0.5.1", b"new");
        let (installed_backup, _, installed_transaction) =
            transaction_paths(&installed_bundle).unwrap();
        write_bundle(&installed_backup, APPLICATION, "0.5.0", b"old");
        write_update_transaction(&installed_transaction, &digest).unwrap();
        assert_eq!(
            reconcile_interrupted_update(&installed_bundle, &digest),
            Ok(RecoveryOutcome::AlreadyInstalled)
        );
        assert_eq!(
            fs::read(installed_backup.join("app/payload.txt")).unwrap(),
            b"old"
        );
        assert!(!installed_transaction.exists());

        let invalid_root = temporary_directory("update-recover-invalid");
        let invalid_bundle = invalid_root.join("application");
        write_bundle(&invalid_bundle, APPLICATION, "0.5.1", b"tampered");
        fs::write(invalid_bundle.join("app/payload.txt"), b"invalid").unwrap();
        let (invalid_backup, invalid_failed, invalid_transaction) =
            transaction_paths(&invalid_bundle).unwrap();
        write_bundle(&invalid_backup, APPLICATION, "0.5.0", b"old");
        write_update_transaction(&invalid_transaction, &digest).unwrap();
        assert_eq!(
            reconcile_interrupted_update(&invalid_bundle, &digest),
            Ok(RecoveryOutcome::Ready)
        );
        assert_eq!(
            fs::read(invalid_bundle.join("app/payload.txt")).unwrap(),
            b"old"
        );
        assert!(invalid_failed.is_dir());
        assert!(!invalid_transaction.exists());

        let gone_root = temporary_directory("update-recover-gone");
        let gone_bundle = gone_root.join("application");
        let (_, _, gone_transaction) = transaction_paths(&gone_bundle).unwrap();
        write_update_transaction(&gone_transaction, &digest).unwrap();
        assert!(
            reconcile_interrupted_update(&gone_bundle, &digest)
                .unwrap_err()
                .contains("both missing")
        );
        assert!(gone_transaction.exists());

        for root in [missing_root, installed_root, invalid_root, gone_root] {
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn rejects_forged_transactions_and_bounds_staging_directories() {
        let root = temporary_directory("update-recover-forged");
        let bundle = root.join("application");
        write_bundle(&bundle, APPLICATION, "0.5.0", b"current");
        let (_, _, transaction) = transaction_paths(&bundle).unwrap();
        write_update_transaction(&transaction, &"a".repeat(64)).unwrap();
        assert!(reconcile_interrupted_update(&bundle, &"b".repeat(64)).is_err());

        for suffix in ["one", "two", "three"] {
            fs::create_dir(root.join(format!(".pam-update-com.example.app-1.0.0-{suffix}")))
                .unwrap();
        }
        fs::create_dir(root.join(".pam-update-com.example.other-1.0.0-kept")).unwrap();
        let stage = update_stage(&root, "com.example.app", "1.0.1").expect("a new stage");
        assert!(stage.is_dir());
        let names: Vec<String> = fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        let retained = names
            .iter()
            .filter(|name| name.starts_with(".pam-update-com.example.app-"))
            .count();
        assert_eq!(retained, MAX_RETAINED_UPDATE_STAGES + 1);
        assert!(
            names
                .iter()
                .any(|name| name == ".pam-update-com.example.other-1.0.0-kept")
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn checks_and_downloads_a_bounded_signed_loopback_release() {
        let artifact = b"verified portable update".to_vec();
        let signing_key = SigningKey::from_bytes(&[11_u8; 32]);
        let served_artifact = artifact.clone();
        let feed = Arc::new(Mutex::new(Vec::new()));
        let served_feed = Arc::clone(&feed);
        let server = TestServer::start(move |request| match request.path.as_str() {
            "/feed" => Served::ok(served_feed.lock().unwrap().clone()),
            // A redirect on the same origin is followed.
            "/artifact" => Served::status(302, "").header("Location", "/files/artifact"),
            "/files/artifact" => Served::ok(served_artifact.clone()),
            _ => Served::status(404, ""),
        });
        *feed.lock().unwrap() = serde_json::to_vec(&sign(
            &signing_key,
            release("0.5.1", &server.url("/artifact"), &artifact),
        ))
        .unwrap();
        let (updater, seen) = updater(
            server.url("/feed"),
            public_key(&signing_key),
            UpdatePolicy::Manual,
            None,
        );
        assert_eq!(updater.snapshot().status, Status::Idle);
        assert!(
            updater
                .download()
                .unwrap_err()
                .contains("Check for updates first")
        );

        let available = updater.check().expect("signed release should be available");
        assert_eq!(available.status, Status::Available);
        assert_eq!(available.available_version.as_deref(), Some("0.5.1"));
        assert_eq!(available.total_bytes, artifact.len() as u64);
        let ready = updater.download().expect("artifact should download");
        assert_eq!(ready.status, Status::Ready);
        assert_eq!(ready.downloaded_bytes, artifact.len() as u64);
        let archive = updater
            .lock_state()
            .archive
            .clone()
            .expect("a staged archive");
        assert_eq!(fs::read(&archive).unwrap(), artifact);
        // Outside an installed bundle there is nothing to replace.
        assert!(updater.install().unwrap_err().contains("packaged"));

        let statuses: Vec<Status> = seen
            .lock()
            .unwrap()
            .iter()
            .map(|snapshot| snapshot.status)
            .collect();
        assert_eq!(statuses.first(), Some(&Status::Checking));
        assert!(
            statuses.contains(&Status::Available)
                && statuses.contains(&Status::Downloading)
                && statuses.contains(&Status::Ready)
        );
        fs::remove_dir_all(archive.parent().unwrap())
            .expect("staging directory should be removable");
    }

    #[test]
    fn reports_up_to_date_and_refuses_untrusted_feeds() {
        let signing_key = SigningKey::from_bytes(&[12_u8; 32]);
        let artifact = b"artifact".to_vec();
        let feed = Arc::new(Mutex::new(Vec::new()));
        let served_feed = Arc::clone(&feed);
        let server = TestServer::start(move |request| match request.path.as_str() {
            "/feed" => Served::ok(served_feed.lock().unwrap().clone()),
            "/short" => Served::ok(b"artif".to_vec()),
            "/wrong" => Served::ok(b"ARTIFACT".to_vec()),
            "/away" => Served::status(302, "").header("Location", "http://example.com/artifact"),
            _ => Served::status(404, ""),
        });
        let publish = |release: Release, key: &SigningKey| {
            *feed.lock().unwrap() = serde_json::to_vec(&sign(key, release)).unwrap()
        };
        let (updater, _) = updater(
            server.url("/feed"),
            public_key(&signing_key),
            UpdatePolicy::Notify,
            None,
        );

        publish(
            release("0.5.0", &server.url("/artifact"), &artifact),
            &signing_key,
        );
        assert_eq!(updater.check().unwrap().status, Status::UpToDate);
        // The notify policy checks and never installs.
        assert!(!updater.run_policy());

        publish(
            release("0.5.1", &server.url("/artifact"), &artifact),
            &SigningKey::from_bytes(&[13_u8; 32]),
        );
        assert!(updater.check().unwrap_err().contains("did not verify"));
        let failed = updater.snapshot();
        assert_eq!(failed.status, Status::Failed);
        assert!(failed.error.unwrap().contains("did not verify"));

        let mut foreign = release("0.5.1", &server.url("/artifact"), &artifact);
        foreign.application_id = "com.pushin.other".to_owned();
        publish(foreign, &signing_key);
        assert!(
            updater
                .check()
                .unwrap_err()
                .contains("different application")
        );
        let mut beta = release("0.5.1", &server.url("/artifact"), &artifact);
        beta.channel = "beta".to_owned();
        publish(beta, &signing_key);
        assert!(
            updater
                .check()
                .unwrap_err()
                .contains("different update channel")
        );
        let mut elsewhere = release("0.5.1", &server.url("/artifact"), &artifact);
        elsewhere.artifacts[0].architecture = "riscv64".to_owned();
        publish(elsewhere, &signing_key);
        assert!(
            updater
                .check()
                .unwrap_err()
                .contains("no portable artifact")
        );
        *feed.lock().unwrap() = vec![b' '; MAX_FEED_BYTES as usize + 1];
        assert!(updater.check().unwrap_err().contains("larger than one MiB"));

        for (path, message) in [
            ("/short", "unexpected artifact size"),
            ("/wrong", "does not match"),
            ("/missing", "answered 404"),
            ("/away", "HTTPS"),
        ] {
            publish(release("0.5.1", &server.url(path), &artifact), &signing_key);
            assert_eq!(updater.check().unwrap().status, Status::Available);
            let error = updater.download().unwrap_err();
            assert!(error.contains(message), "{path}: {error}");
            assert_eq!(updater.snapshot().status, Status::Failed);
        }
        let prefix = format!(".pam-update-{APPLICATION}-");
        for entry in fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(Result::ok)
        {
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }

    #[test]
    fn updates_are_disabled_without_configuration() {
        let updater = Updater::new(
            APPLICATION,
            "1.0.0",
            None,
            None,
            Arc::new(UreqClient::new()),
            |_| {},
        );
        assert_eq!(updater.snapshot().status, Status::Disabled);
        assert_eq!(updater.policy(), None);
        assert!(updater.check().unwrap_err().contains("not configured"));
        assert!(!updater.run_policy());
        assert_eq!(updater.snapshot().status, Status::Disabled);
    }

    #[test]
    fn redirects_stay_on_absolute_or_same_origin_targets() {
        assert_eq!(
            redirect_target("https://a.example/feed?x=1", "/v2/feed").as_deref(),
            Some("https://a.example/v2/feed")
        );
        assert_eq!(
            redirect_target("https://a.example/feed", "https://cdn.example/feed").as_deref(),
            Some("https://cdn.example/feed")
        );
        assert_eq!(redirect_target("https://a.example/feed", "feed2"), None);
        assert_eq!(
            redirect_target("https://a.example/feed", "//evil.example/feed"),
            None
        );
    }

    #[test]
    fn feeds_without_an_expiry_keep_their_signed_payload_and_new_ones_sign_it() {
        let signing_key = SigningKey::from_bytes(&[21_u8; 32]);
        let key = public_key(&signing_key);
        let plain = release("1.0.0", "https://example.com/a", b"a");
        let payload = String::from_utf8(plain.signed_payload().unwrap()).unwrap();
        assert!(!payload.contains("expiresAtUnix"));
        assert!(payload.ends_with("}]}"), "{payload}");
        let legacy = format!(
            r#"{{"release":{payload},"signature":"{}"}}"#,
            hex(&signing_key.sign(payload.as_bytes()).to_bytes())
        );
        let legacy: SignedRelease = serde_json::from_str(&legacy).unwrap();
        assert!(verify_signed_feed(&legacy, &key).is_ok());

        let mut expiring = plain.clone();
        expiring.expires_at_unix = Some(2_000_000_000);
        let mut signed = sign(&signing_key, expiring);
        assert!(verify_signed_feed(&signed, &key).is_ok());
        signed.release.expires_at_unix = Some(4_000_000_000);
        assert!(
            verify_signed_feed(&signed, &key).is_err(),
            "the expiry is signed"
        );
        signed.release.expires_at_unix = Some(0);
        assert!(verify_signed_feed(&signed, &key).is_err());
    }

    #[test]
    fn refuses_expired_and_rolled_back_feeds() {
        let directory = std::env::temp_dir().join(format!("pam-feed-seen-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let watermark = directory.join(".app.update-feed-seen");
        let at = |version: &str, expires: Option<u64>| {
            let mut release = release(version, "https://example.com/a", b"a");
            release.expires_at_unix = expires;
            release
        };
        assert!(
            check_feed_freshness(&at("1.0.0", Some(100)), 100, None)
                .unwrap_err()
                .contains("expired")
        );
        assert!(check_feed_freshness(&at("1.0.0", Some(101)), 100, None).is_ok());
        assert!(check_feed_freshness(&at("1.0.0", None), u64::MAX, None).is_ok());

        assert!(check_feed_freshness(&at("1.2.0", None), 0, Some(&watermark)).is_ok());
        assert_eq!(fs::read_to_string(&watermark).unwrap(), "stable\n1.2.0\n");
        assert!(check_feed_freshness(&at("1.2.0", None), 0, Some(&watermark)).is_ok());
        assert!(
            check_feed_freshness(&at("1.1.9", None), 0, Some(&watermark))
                .unwrap_err()
                .contains("replayed")
        );
        assert!(check_feed_freshness(&at("1.3.0", None), 0, Some(&watermark)).is_ok());
        assert!(check_feed_freshness(&at("1.2.0", None), 0, Some(&watermark)).is_err());
        // Another channel starts its own watermark.
        let mut beta = at("0.9.0", None);
        beta.channel = "beta".to_owned();
        assert!(check_feed_freshness(&beta, 0, Some(&watermark)).is_ok());
        // A damaged watermark is ignored, not trusted.
        fs::write(&watermark, "stable\nnot a version\n").unwrap();
        assert!(check_feed_freshness(&at("0.1.0", None), 0, Some(&watermark)).is_ok());
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn publish_accepts_a_bounded_expiry() {
        let parse = |days: &str| {
            PublishOptions::parse(
                [
                    "--key",
                    "k",
                    "--output",
                    "o",
                    "--published-at",
                    "t",
                    "--expires-in",
                    days,
                    "--artifact",
                    "linux,x86_64,portable,a,https://example.com/a",
                ]
                .iter()
                .map(OsString::from),
            )
        };
        assert_eq!(parse("30").unwrap().expires_in_days, Some(30));
        for invalid in ["0", "367", "-1", "x"] {
            assert!(parse(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn linux_archives_unpack_in_rust_and_refuse_escapes() {
        let directory =
            std::env::temp_dir().join(format!("pam-update-untar-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let archive = directory.join("update.tar.gz");
        let write = |entries: &[(&str, tar::EntryType, &str)]| {
            let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
                Vec::new(),
                flate2::Compression::fast(),
            ));
            for (path, kind, value) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(*kind);
                header.set_mode(0o755);
                let contents: &[u8] = if kind.is_file() {
                    value.as_bytes()
                } else {
                    &[]
                };
                header.set_size(contents.len() as u64);
                if kind.is_symlink() || *kind == tar::EntryType::Link {
                    header.set_link_name(value).unwrap();
                }
                let name = path.as_bytes();
                header.as_old_mut().name[..name.len()].copy_from_slice(name);
                header.set_cksum();
                builder.append(&header, contents).unwrap();
            }
            fs::write(&archive, builder.into_inner().unwrap().finish().unwrap()).unwrap();
        };
        write(&[
            ("App/", tar::EntryType::Directory, ""),
            ("App/lib/", tar::EntryType::Directory, ""),
            ("App/lib/libx.so.1", tar::EntryType::Regular, "x"),
            ("App/lib/libx.so", tar::EntryType::Symlink, "libx.so.1"),
            ("App/up", tar::EntryType::Symlink, "lib/../lib/libx.so.1"),
        ]);
        let out = directory.join("ok");
        fs::create_dir(&out).unwrap();
        extract_tar_gz(&archive, &out).unwrap();
        assert_eq!(
            fs::read_to_string(out.join("App/lib/libx.so")).unwrap(),
            "x"
        );
        for (index, entries) in [
            vec![("App/../../evil", tar::EntryType::Regular, "x")],
            vec![("/abs", tar::EntryType::Regular, "x")],
            vec![("App/l", tar::EntryType::Symlink, "/etc")],
            vec![("App/l", tar::EntryType::Symlink, "../../etc")],
            vec![("App/h", tar::EntryType::Link, "App/x")],
            vec![("App/f", tar::EntryType::Fifo, "")],
        ]
        .into_iter()
        .enumerate()
        {
            write(&entries);
            let out = directory.join(format!("bad-{index}"));
            fs::create_dir(&out).unwrap();
            assert!(extract_tar_gz(&archive, &out).is_err(), "{index}");
        }
        assert!(!directory.join("evil").exists());
        fs::remove_dir_all(&directory).unwrap();
    }
}
