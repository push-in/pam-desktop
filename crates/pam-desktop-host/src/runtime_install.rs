//! Installing a PHP runtime when none is found: consent, and on Linux and
//! macOS the PAM runtime itself, downloaded and verified here.
//!
//! Nothing is downloaded until the user agrees. In a terminal PAM Desktop
//! says what it will fetch, from where and into which folder, and asks;
//! scripts and CI agree beforehand with `--yes` or
//! `PAM_DESKTOP_AUTO_INSTALL=1`. Without a terminal and without that, the
//! installation is refused with the ways to install PHP by hand.
//!
//! The PAM runtime is not installed by running a downloaded script. Every
//! PAM release publishes, beside its archive, an update manifest signed with
//! Ed25519 (the one `pam self-update` trusts). PAM Desktop pins the SHA-256
//! of that signing key (`SIGNING_IDENTITIES`), checks the manifest's
//! signature, version, target and freshness, then downloads the archive and
//! accepts it only with the size and SHA-256 the manifest signs. The archive
//! is unpacked in Rust (no `curl`, `tar` or `sh` from `PATH`) into the same
//! layout as the official installer, so `pam self-update` keeps working.

use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, IsTerminal, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

/// Set by `--yes` on the command line.
static ASSUME_YES: AtomicBool = AtomicBool::new(false);

const AUTO_INSTALL_ENV: &str = "PAM_DESKTOP_AUTO_INSTALL";

/// Removes `--yes`/`-y` from `arguments`, remembering that it was given.
pub fn take_assume_yes(arguments: impl Iterator<Item = OsString>) -> Vec<OsString> {
    arguments
        .filter(|argument| {
            let yes = argument == "--yes" || argument == "-y";
            if yes {
                ASSUME_YES.store(true, Ordering::Relaxed);
            }
            !yes
        })
        .collect()
}

/// How the user agreed to an installation, when they did.
#[derive(Debug, PartialEq, Eq)]
enum Agreement {
    Given,
    Ask,
    Refused,
}

fn agreement(
    assume_yes: bool,
    environment: Option<&std::ffi::OsStr>,
    interactive: bool,
) -> Agreement {
    if assume_yes || environment.is_some_and(|value| value == "1") {
        Agreement::Given
    } else if interactive {
        Agreement::Ask
    } else {
        Agreement::Refused
    }
}

fn affirmative(answer: &str) -> bool {
    matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes" | "s" | "sim"
    )
}

/// Asks before anything is downloaded. `description` says what, from where
/// and into which folder.
pub fn consent(description: &str) -> Result<(), String> {
    let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    match agreement(
        ASSUME_YES.load(Ordering::Relaxed),
        std::env::var_os(AUTO_INSTALL_ENV).as_deref(),
        interactive,
    ) {
        Agreement::Given => {
            eprintln!("pam-desktop: {description}");
            Ok(())
        }
        Agreement::Ask => {
            eprint!("pam-desktop: {description}\nInstall it now? [y/N] ");
            let _ = std::io::stderr().flush();
            let mut answer = String::new();
            std::io::stdin()
                .lock()
                .read_line(&mut answer)
                .map_err(|error| format!("cannot read the answer: {error}"))?;
            if affirmative(&answer) {
                Ok(())
            } else {
                Err("the installation was declined".to_owned())
            }
        }
        Agreement::Refused => Err(format!(
            "{description}\nNothing was installed: there is no terminal to ask. Run again with \
             --yes or {AUTO_INSTALL_ENV}=1 to allow it, install PHP 8.5 (or PAM, \
             https://github.com/push-in/pam) yourself, or set PAM_DESKTOP_PHP to a PHP binary"
        )),
    }
}

// ---- the PAM runtime ------------------------------------------------------

/// SHA-256 of the Ed25519 public key that signs PAM releases (the
/// `signingIdentitySha256` of their update manifests, the identity
/// `pam self-update` pins). A second entry is added while the key rotates.
pub const SIGNING_IDENTITIES: &[&str] =
    &["ce476acae3184856032eda6d60f3be11db8b396204ddb0a3d1444306539e3bcd"];

const REPOSITORY: &str = "push-in/pam";
const VERSION_ENV: &str = "PAM_DESKTOP_PAM_VERSION";
const MAX_METADATA_BYTES: u64 = 1 << 20;
const MAX_MANIFEST_BYTES: u64 = 256 << 10;
const MAX_ARCHIVE_BYTES: u64 = 1 << 30;
const MAX_EXTRACTED_BYTES: u64 = 4 << 30;
const MAX_EXTRACTED_ENTRIES: usize = 100_000;
/// The longest freshness window a signed manifest may claim (as PAM).
const MAX_VALIDITY_SECONDS: u64 = 31 * 24 * 60 * 60;
const CLOCK_SKEW_SECONDS: u64 = 5 * 60;
const IDENTITY_TIMEOUT: Duration = Duration::from_secs(10);

/// The release target of this system: its name and the codes the manifest
/// signs for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub name: &'static str,
    platform_code: u64,
    architecture_code: u64,
}

impl Target {
    pub fn current() -> Option<Self> {
        let (name, platform_code, architecture_code) =
            match (std::env::consts::OS, std::env::consts::ARCH) {
                ("linux", "x86_64") => ("linux-x86_64", 1, 1),
                ("linux", "aarch64") => ("linux-aarch64", 1, 2),
                ("macos", "x86_64") => ("macos-x86_64", 2, 1),
                ("macos", "aarch64") => ("macos-arm64", 2, 2),
                _ => return None,
            };
        Some(Self {
            name,
            platform_code,
            architecture_code,
        })
    }
}

/// What a verified manifest authorizes.
#[derive(Debug, PartialEq, Eq)]
pub struct Authorization {
    pub sha256: String,
    pub bytes: u64,
}

/// A release tag as PAM publishes them: `v` and SemVer without build metadata.
pub fn valid_release(value: &str) -> bool {
    let Some(version) = value.strip_prefix('v') else {
        return false;
    };
    if value.len() > 64 || version.contains('+') {
        return false;
    }
    let (core, prerelease) = match version.split_once('-') {
        Some((core, prerelease)) => (core, Some(prerelease)),
        None => (version, None),
    };
    let number = |part: &str| {
        !part.is_empty()
            && part.bytes().all(|byte| byte.is_ascii_digit())
            && (part == "0" || !part.starts_with('0'))
    };
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| number(part))
        && prerelease.is_none_or(|prerelease| {
            prerelease.split('.').all(|identifier| {
                !identifier.is_empty()
                    && identifier
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                    && (!identifier.bytes().all(|byte| byte.is_ascii_digit()) || number(identifier))
            })
        })
}

/// The JSON a manifest signature covers: the document without
/// `manifestSignature`, compact, with object keys in byte order (how PAM
/// serializes it).
fn canonical_json(value: &serde_json::Value, output: &mut Vec<u8>) -> Result<(), String> {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            output.push(b'{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                serde_json::to_writer(&mut *output, key).map_err(|error| error.to_string())?;
                output.push(b':');
                canonical_json(&map[key], output)?;
            }
            output.push(b'}');
        }
        serde_json::Value::Array(items) => {
            output.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                canonical_json(item, output)?;
            }
            output.push(b']');
        }
        scalar => serde_json::to_writer(&mut *output, scalar).map_err(|error| error.to_string())?,
    }
    Ok(())
}

fn lower_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Checks a PAM update manifest: Ed25519 signature by a pinned identity, the
/// requested release and target, a runtime archive, and, when `now` is given
/// (the version came from "latest"), a signed freshness window containing it.
pub fn authorize_manifest(
    source: &[u8],
    pinned: &[&str],
    release: &str,
    target: Target,
    now: Option<u64>,
) -> Result<Authorization, String> {
    if source.is_empty() || source.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("the PAM release manifest must hold at most 256 KiB".to_owned());
    }
    let mut document: serde_json::Value = serde_json::from_slice(source)
        .map_err(|error| format!("the PAM release manifest is not JSON: {error}"))?;
    let object = document
        .as_object_mut()
        .ok_or("the PAM release manifest is not an object")?;
    let signature = object
        .remove("manifestSignature")
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or("the PAM release manifest is not signed")?;
    let text = |name: &str| {
        object
            .get(name)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("the PAM release manifest has no {name}"))
    };
    let engine = base64::engine::general_purpose::STANDARD;
    let public_key = text("signingPublicKey")?;
    let key_bytes = engine
        .decode(public_key)
        .ok()
        .filter(|bytes| engine.encode(bytes) == public_key)
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .ok_or("the PAM release manifest has a malformed signing key")?;
    let identity = lower_hex(&Sha256::digest(key_bytes));
    if text("signingIdentitySha256")? != identity {
        return Err("the PAM release manifest names another signing identity".to_owned());
    }
    if !pinned.contains(&identity.as_str()) {
        return Err(format!(
            "the PAM release manifest was signed by {identity}, not by the PAM release key this PAM Desktop pins"
        ));
    }
    let signature = engine
        .decode(&signature)
        .ok()
        .filter(|bytes| engine.encode(bytes) == signature)
        .and_then(|bytes| Signature::from_slice(&bytes).ok())
        .ok_or("the PAM release manifest has a malformed signature")?;
    let mut payload = Vec::with_capacity(source.len());
    canonical_json(&document, &mut payload)?;
    VerifyingKey::from_bytes(&key_bytes)
        .and_then(|key| key.verify_strict(&payload, &signature))
        .map_err(|_| "the signature of the PAM release manifest did not verify".to_owned())?;

    // Only signed fields are read from here on.
    let object = document.as_object().ok_or("unreachable")?;
    let text = |name: &str| object.get(name).and_then(serde_json::Value::as_str);
    let number = |name: &str| object.get(name).and_then(serde_json::Value::as_u64);
    if text("releaseVersion") != Some(release) {
        return Err(format!(
            "the signed PAM manifest is not for release {release}"
        ));
    }
    if number("surfaceCode") != Some(1)
        || number("packageCode") != Some(1)
        || number("platformCode") != Some(target.platform_code)
        || number("architectureCode") != Some(target.architecture_code)
    {
        return Err(format!(
            "the signed PAM manifest does not authorize a runtime archive for {}",
            target.name
        ));
    }
    if let Some(now) = now {
        let issued = number("issuedAtUnix").ok_or("the signed PAM manifest has no issuedAtUnix")?;
        let expires =
            number("expiresAtUnix").ok_or("the signed PAM manifest has no expiresAtUnix")?;
        if issued == 0 || expires <= issued || expires - issued > MAX_VALIDITY_SECONDS {
            return Err("the signed PAM manifest has an invalid freshness window".to_owned());
        }
        if issued > now.saturating_add(CLOCK_SKEW_SECONDS) {
            return Err("the signed PAM manifest was issued in the future".to_owned());
        }
        if expires <= now {
            return Err(format!(
                "the signed PAM manifest of {release} has expired; set {VERSION_ENV} to install a chosen release"
            ));
        }
    }
    let artifact = object
        .get("artifact")
        .and_then(serde_json::Value::as_object)
        .ok_or("the signed PAM manifest has no artifact")?;
    let sha256 = artifact
        .get("sha256")
        .and_then(serde_json::Value::as_str)
        .filter(|value| is_sha256(value))
        .ok_or("the signed PAM manifest has no artifact SHA-256")?;
    let bytes = artifact
        .get("bytes")
        .and_then(serde_json::Value::as_u64)
        .filter(|bytes| (1..=MAX_ARCHIVE_BYTES).contains(bytes))
        .ok_or("the signed PAM manifest has an invalid artifact size")?;
    if artifact.get("path").and_then(serde_json::Value::as_str) != Some("files/candidate.tar.gz") {
        return Err("the signed PAM manifest does not describe the release archive".to_owned());
    }
    Ok(Authorization {
        sha256: sha256.to_owned(),
        bytes,
    })
}

/// Where the official installer puts releases and links `pam`.
struct Layout {
    root: PathBuf,
}

impl Layout {
    fn from_environment() -> Option<Self> {
        let root = std::env::var_os("PAM_INSTALL_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("XDG_DATA_HOME")
                    .map(PathBuf::from)
                    .or_else(|| {
                        std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/share"))
                    })
                    .map(|data| data.join("pam"))
            })?;
        root.is_absolute().then_some(Self { root })
    }

    fn release(&self, release: &str, target: Target) -> PathBuf {
        self.root.join(format!("{release}-{}", target.name))
    }
}

/// Installs the PAM runtime and links it at `link`; returns `link`.
pub fn install_pam(link: &Path) -> Result<PathBuf, String> {
    let target = Target::current().ok_or("PAM publishes no runtime for this system")?;
    let layout = Layout::from_environment().ok_or("HOME is not set")?;
    let agent = crate::windows_php::agent();
    let (release, now) = match std::env::var(VERSION_ENV) {
        Ok(release) => (release, None),
        Err(_) => (latest_release(&agent)?, Some(unix_now()?)),
    };
    if !valid_release(&release) {
        return Err(format!(
            "{release:?} is not a PAM release tag (vMAJOR.MINOR.PATCH)"
        ));
    }
    let base = format!("https://github.com/{REPOSITORY}/releases/download/{release}");
    let manifest = crate::windows_php::get(
        &agent,
        &format!("{base}/pam-{release}-{}.update.json", target.name),
        MAX_MANIFEST_BYTES,
        None,
    )?;
    let authorization = authorize_manifest(&manifest, SIGNING_IDENTITIES, &release, target, now)?;
    let archive_url = format!("{base}/pam-{release}-{}.tar.gz", target.name);
    let destination = layout.release(&release, target);
    consent(&format!(
        "No PHP was found. PAM Desktop can install the PAM runtime {release} for {} ({} MB):\n  \
         from      {archive_url}\n  \
         verified  Ed25519-signed release manifest, signing identity {}\n  \
         into      {}\n  \
         linked as {}",
        target.name,
        authorization.bytes.div_ceil(1 << 20),
        SIGNING_IDENTITIES[0],
        destination.display(),
        link.display()
    ))?;
    fs::create_dir_all(&layout.root)
        .map_err(|error| format!("cannot create {}: {error}", layout.root.display()))?;
    if !runnable(&destination) {
        let stage = layout.root.join(format!(
            ".{release}-{}.installing-{}",
            target.name,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&stage);
        create_private_directory(&stage)?;
        let result = (|| {
            let archive = stage.join("pam.tar.gz");
            eprintln!("pam-desktop: downloading {archive_url}");
            crate::windows_php::get(&agent, &archive_url, authorization.bytes, Some(&archive))?;
            verify_archive(&archive, &authorization)?;
            let root = format!("pam-{release}-{}", target.name);
            extract(&archive, &stage, &root)?;
            let extracted = stage.join(&root);
            check_identity(&extracted, &release)?;
            if destination.exists() || destination.is_symlink() {
                fs::remove_dir_all(&destination).map_err(|error| {
                    format!(
                        "cannot replace the incomplete {}: {error}",
                        destination.display()
                    )
                })?;
            }
            fs::rename(&extracted, &destination).map_err(|error| {
                format!("cannot install PAM into {}: {error}", destination.display())
            })
        })();
        let _ = fs::remove_dir_all(&stage);
        result?;
    }
    activate(&destination, link)?;
    eprintln!(
        "pam-desktop: installed the PAM runtime {release} at {}",
        destination.display()
    );
    Ok(link.to_path_buf())
}

fn unix_now() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .map_err(|_| "the system clock is before 1970".to_owned())
}

fn latest_release(agent: &ureq::Agent) -> Result<String, String> {
    let metadata = crate::windows_php::get(
        agent,
        &format!("https://api.github.com/repos/{REPOSITORY}/releases/latest"),
        MAX_METADATA_BYTES,
        None,
    )?;
    serde_json::from_slice::<serde_json::Value>(&metadata)
        .ok()
        .and_then(|value| value.get("tag_name")?.as_str().map(str::to_owned))
        .ok_or_else(|| "GitHub sent no latest PAM release".to_owned())
}

fn runnable(release: &Path) -> bool {
    !release.is_symlink() && release.is_dir() && release.join("bin/pam-run").is_file()
}

fn create_private_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder
    };
    #[cfg(not(unix))]
    let builder = fs::DirBuilder::new();
    builder
        .create(path)
        .map_err(|error| format!("cannot create {}: {error}", path.display()))
}

fn verify_archive(archive: &Path, authorization: &Authorization) -> Result<(), String> {
    let mut file =
        fs::File::open(archive).map_err(|error| format!("cannot read the PAM archive: {error}"))?;
    let mut digest = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = vec![0_u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        total += read as u64;
        digest.update(&buffer[..read]);
    }
    let actual = lower_hex(&digest.finalize());
    if total != authorization.bytes || actual != authorization.sha256 {
        return Err(format!(
            "the PAM archive does not match its signed manifest (expected {} bytes with SHA-256 {}, \
             got {total} bytes with {actual}); nothing was installed",
            authorization.bytes, authorization.sha256
        ));
    }
    Ok(())
}

/// Unpacks a verified release archive: only regular files and directories,
/// all under `root`, with bounded count and size; never owners, setuid bits
/// or extended attributes.
pub fn extract(archive: &Path, destination: &Path, root: &str) -> Result<(), String> {
    let file =
        fs::File::open(archive).map_err(|error| format!("cannot read the PAM archive: {error}"))?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    archive.set_preserve_permissions(false);
    archive.set_preserve_ownerships(false);
    archive.set_unpack_xattrs(false);
    archive.set_overwrite(false);
    let mut entries = 0_usize;
    let mut bytes = 0_u64;
    for entry in archive
        .entries()
        .map_err(|error| format!("the PAM archive is unreadable: {error}"))?
    {
        let mut entry = entry.map_err(|error| format!("the PAM archive is unreadable: {error}"))?;
        entries += 1;
        if entries > MAX_EXTRACTED_ENTRIES {
            return Err("the PAM archive has too many entries".to_owned());
        }
        let path = entry
            .path()
            .map_err(|error| format!("the PAM archive has an unreadable path: {error}"))?
            .into_owned();
        let mut components = path.components();
        let inside = components.next() == Some(Component::Normal(root.as_ref()))
            && components.all(|component| matches!(component, Component::Normal(_)));
        if !inside {
            return Err(format!(
                "the PAM archive has a path outside {root}: {}",
                path.display()
            ));
        }
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() {
            return Err(format!(
                "the PAM archive has an unexpected special entry: {}",
                path.display()
            ));
        }
        bytes = bytes.saturating_add(entry.size());
        if bytes > MAX_EXTRACTED_BYTES {
            return Err("the PAM archive expands beyond 4 GiB".to_owned());
        }
        if !entry
            .unpack_in(destination)
            .map_err(|error| format!("cannot unpack {}: {error}", path.display()))?
        {
            return Err(format!(
                "the PAM archive escapes its folder: {}",
                path.display()
            ));
        }
    }
    if !destination.join(root).is_dir() {
        return Err(format!("the PAM archive has no {root} folder"));
    }
    Ok(())
}

/// Runs `bin/pam-run --version` of the unpacked release, which must name the
/// release that was verified.
fn check_identity(release_directory: &Path, release: &str) -> Result<(), String> {
    let mut child = std::process::Command::new(release_directory.join("bin/pam-run"))
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("cannot start the unpacked PAM runtime: {error}"))?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        if started.elapsed() > IDENTITY_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err("the unpacked PAM runtime did not report its version".to_owned());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut output = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.by_ref().take(4096).read_to_string(&mut output);
    }
    let expected = format!("pam {}", release.trim_start_matches('v'));
    if !status.success() || output.lines().next().map(str::trim) != Some(expected.as_str()) {
        return Err(format!(
            "the unpacked PAM runtime reports {:?}, expected {expected:?}",
            output.lines().next().unwrap_or_default()
        ));
    }
    Ok(())
}

/// Points `link` at the release's `bin/pam-run`, replacing only a symlink.
fn activate(release_directory: &Path, link: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        let parent = link.parent().ok_or("the pam link has no folder")?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        if link.exists() && !link.is_symlink() {
            return Err(format!(
                "refusing to replace {}, which is not a symlink",
                link.display()
            ));
        }
        let next = parent.join(format!(".pam.next.{}.tmp", std::process::id()));
        let _ = fs::remove_file(&next);
        std::os::unix::fs::symlink(release_directory.join("bin/pam-run"), &next)
            .map_err(|error| format!("cannot link {}: {error}", link.display()))?;
        fs::rename(&next, link).map_err(|error| {
            let _ = fs::remove_file(&next);
            format!("cannot link {}: {error}", link.display())
        })
    }
    #[cfg(not(unix))]
    {
        let _ = (release_directory, link);
        Err("PAM has no runtime for this system".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The signed manifest PAM published for v2.2.9 on Linux x86_64.
    const MANIFEST: &[u8] = include_bytes!("../tests/fixtures/pam-v2.2.9-linux-x86_64.update.json");
    const LINUX_X86_64: Target = Target {
        name: "linux-x86_64",
        platform_code: 1,
        architecture_code: 1,
    };
    const ISSUED: u64 = 1_793_159_360;

    #[test]
    fn consent_is_explicit() {
        assert_eq!(agreement(true, None, false), Agreement::Given);
        assert_eq!(
            agreement(false, Some("1".as_ref()), false),
            Agreement::Given
        );
        assert_eq!(
            agreement(false, Some("0".as_ref()), false),
            Agreement::Refused
        );
        assert_eq!(
            agreement(false, Some("yes".as_ref()), false),
            Agreement::Refused
        );
        assert_eq!(agreement(false, None, true), Agreement::Ask);
        assert_eq!(agreement(false, None, false), Agreement::Refused);
        assert!(affirmative(" y\n") && affirmative("Yes") && affirmative("sim"));
        assert!(!affirmative("") && !affirmative("n") && !affirmative("yess"));
        let rest = take_assume_yes(["run", "--yes", "app"].into_iter().map(OsString::from));
        assert_eq!(rest, vec![OsString::from("run"), OsString::from("app")]);
        assert!(ASSUME_YES.load(Ordering::Relaxed));
    }

    #[test]
    fn accepts_the_genuine_signed_manifest_of_a_pam_release() {
        let authorization =
            authorize_manifest(MANIFEST, SIGNING_IDENTITIES, "v2.2.9", LINUX_X86_64, None).unwrap();
        assert_eq!(
            authorization,
            Authorization {
                sha256: "b436824b8bcd1becc7bbc2e55f8e742f142addabacbdc8c78e9b93aee17b8736".into(),
                bytes: 80_370_682,
            }
        );
        // Inside its signed freshness window, and refused once it expired.
        assert!(
            authorize_manifest(
                MANIFEST,
                SIGNING_IDENTITIES,
                "v2.2.9",
                LINUX_X86_64,
                Some(ISSUED + 60)
            )
            .is_ok()
        );
        let expired = ISSUED + MAX_VALIDITY_SECONDS + 1;
        assert!(
            authorize_manifest(
                MANIFEST,
                SIGNING_IDENTITIES,
                "v2.2.9",
                LINUX_X86_64,
                Some(expired)
            )
            .unwrap_err()
            .contains("expired")
        );
        assert!(
            authorize_manifest(
                MANIFEST,
                SIGNING_IDENTITIES,
                "v2.2.9",
                LINUX_X86_64,
                Some(ISSUED - 3_600)
            )
            .is_err()
        );
    }

    #[test]
    fn refuses_manifests_for_other_releases_targets_keys_or_contents() {
        let pinned = SIGNING_IDENTITIES;
        assert!(authorize_manifest(MANIFEST, pinned, "v2.2.8", LINUX_X86_64, None).is_err());
        let arm = Target {
            name: "linux-aarch64",
            platform_code: 1,
            architecture_code: 2,
        };
        assert!(authorize_manifest(MANIFEST, pinned, "v2.2.9", arm, None).is_err());
        let other = "0".repeat(64);
        assert!(
            authorize_manifest(MANIFEST, &[other.as_str()], "v2.2.9", LINUX_X86_64, None)
                .unwrap_err()
                .contains("not by the PAM release key")
        );
        // Any change to a signed field breaks the signature.
        let text = std::str::from_utf8(MANIFEST).unwrap();
        let tampered = text.replace(
            "b436824b8bcd1becc7bbc2e55f8e742f142addabacbdc8c78e9b93aee17b8736",
            "c436824b8bcd1becc7bbc2e55f8e742f142addabacbdc8c78e9b93aee17b8736",
        );
        assert_ne!(tampered, text);
        assert!(
            authorize_manifest(tampered.as_bytes(), pinned, "v2.2.9", LINUX_X86_64, None)
                .unwrap_err()
                .contains("did not verify")
        );
        // A key the attacker controls, with its own matching identity, is
        // still not the pinned one.
        let mut forged: serde_json::Value = serde_json::from_slice(MANIFEST).unwrap();
        forged["signingPublicKey"] = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into();
        let forged = serde_json::to_vec(&forged).unwrap();
        assert!(authorize_manifest(&forged, pinned, "v2.2.9", LINUX_X86_64, None).is_err());
        assert!(authorize_manifest(b"{}", pinned, "v2.2.9", LINUX_X86_64, None).is_err());
        assert!(authorize_manifest(b"", pinned, "v2.2.9", LINUX_X86_64, None).is_err());
    }

    #[test]
    fn release_tags_are_strict() {
        for valid in ["v2.2.9", "v0.0.1", "v10.20.30-rc.1", "v1.0.0-alpha-2"] {
            assert!(valid_release(valid), "{valid}");
        }
        for invalid in [
            "2.2.9",
            "v2.2",
            "v02.2.9",
            "v2.2.9+build",
            "v2.2.9-",
            "v2.2.9-01",
            "v2.2.9/../x",
            "",
        ] {
            assert!(!valid_release(invalid), "{invalid}");
        }
    }

    fn tar_gz(entries: &[(&str, tar::EntryType, &str)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        for (path, kind, contents) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(*kind);
            header.set_mode(0o755);
            let contents = contents.as_bytes();
            header.set_size(contents.len() as u64);
            if kind.is_symlink() || *kind == tar::EntryType::Link {
                header.set_size(0);
                header.set_link_name("/etc/passwd").unwrap();
            }
            // Bypasses the builder's own path checks to write hostile names.
            let name = path.as_bytes();
            header.as_old_mut().name[..name.len()].copy_from_slice(name);
            header.set_cksum();
            builder.append(&header, contents).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn extracts_only_files_and_folders_under_the_release_root() {
        let directory =
            std::env::temp_dir().join(format!("pam-runtime-extract-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let archive = directory.join("pam.tar.gz");
        let good = tar_gz(&[
            ("pam-v1.0.0-linux-x86_64/", tar::EntryType::Directory, ""),
            (
                "pam-v1.0.0-linux-x86_64/bin/",
                tar::EntryType::Directory,
                "",
            ),
            (
                "pam-v1.0.0-linux-x86_64/bin/pam-run",
                tar::EntryType::Regular,
                "#!/bin/sh\n",
            ),
        ]);
        fs::write(&archive, good).unwrap();
        let out = directory.join("ok");
        fs::create_dir(&out).unwrap();
        extract(&archive, &out, "pam-v1.0.0-linux-x86_64").unwrap();
        let binary = out.join("pam-v1.0.0-linux-x86_64/bin/pam-run");
        assert_eq!(fs::read(&binary).unwrap(), b"#!/bin/sh\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&binary).unwrap().permissions().mode() & 0o111,
                0o111
            );
        }

        for (index, hostile) in [
            tar_gz(&[(
                "pam-v1.0.0-linux-x86_64/../../evil",
                tar::EntryType::Regular,
                "x",
            )]),
            tar_gz(&[("/tmp/evil", tar::EntryType::Regular, "x")]),
            tar_gz(&[("other/file", tar::EntryType::Regular, "x")]),
            tar_gz(&[("pam-v1.0.0-linux-x86_64/link", tar::EntryType::Symlink, "")]),
            tar_gz(&[("pam-v1.0.0-linux-x86_64/hard", tar::EntryType::Link, "")]),
            tar_gz(&[("pam-v1.0.0-linux-x86_64/fifo", tar::EntryType::Fifo, "")]),
        ]
        .into_iter()
        .enumerate()
        {
            fs::write(&archive, hostile).unwrap();
            let out = directory.join(format!("bad-{index}"));
            fs::create_dir(&out).unwrap();
            assert!(
                extract(&archive, &out, "pam-v1.0.0-linux-x86_64").is_err(),
                "{index}"
            );
        }
        assert!(!directory.join("evil").exists());
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn the_archive_must_have_the_signed_size_and_digest() {
        let directory =
            std::env::temp_dir().join(format!("pam-runtime-digest-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let archive = directory.join("pam.tar.gz");
        fs::write(&archive, b"abc").unwrap();
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let signed = |sha256: &str, bytes| Authorization {
            sha256: sha256.into(),
            bytes,
        };
        assert!(verify_archive(&archive, &signed(abc, 3)).is_ok());
        assert!(verify_archive(&archive, &signed(abc, 4)).is_err());
        assert!(verify_archive(&archive, &signed(&"0".repeat(64), 3)).is_err());
        fs::remove_dir_all(&directory).unwrap();
    }
}
