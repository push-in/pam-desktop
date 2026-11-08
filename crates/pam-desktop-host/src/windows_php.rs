//! PHP on Windows, where PAM has no release: the official build of
//! windows.php.net (PHP 8.5, non thread safe, x64), downloaded once into
//! `%LOCALAPPDATA%\PAM Desktop\php\<version>` and checked against the SHA-256
//! that windows.php.net publishes for it in `releases.json`, before anything
//! of it is extracted or run.
//!
//! `releases.json` comes from the same server as the archive, so its SHA-256
//! alone only proves the download was not damaged. PAM Desktop therefore also
//! carries the SHA-256 of every archive it supports (`PINNED`): an archive
//! must match both. When windows.php.net lists a newer build than this host
//! knows, the newest pinned build is installed from the server's `archives/`
//! folder instead; `PAM_DESKTOP_PHP_ALLOW_UNPINNED=1` accepts the unknown
//! build with only the published digest. The PHP binaries of windows.php.net
//! carry no Authenticode signature, so there is no publisher to check.
//! Nothing is downloaded without consent (`crate::runtime_install::consent`).
//!
//! The runtime gets a `php.ini` of its own that turns on the shared
//! extensions applications commonly need (`DEFAULT_EXTENSIONS`, those the
//! archive has); the extensions an application lists in `pam-desktop.json`
//! are turned on for it when it runs (`extension_arguments`), and
//! `pam-desktop build` ships what the application refers to.

use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The PHP branch applications are written for.
pub const BRANCH: &str = "8.5";
const RELEASES: &str = "https://windows.php.net/downloads/releases/releases.json";
const DOWNLOADS: &str = "https://windows.php.net/downloads/releases/";
/// A build of the branch for 64-bit Windows, non thread safe (the CLI).
const BUILD_PREFIX: &str = "nts-vs";
const BUILD_SUFFIX: &str = "-x64";
/// The archive is about 35 MB; anything far larger is not what was published.
const MAX_ARCHIVE_BYTES: u64 = 256 << 20;
const MAX_RELEASES_BYTES: u64 = 4 << 20;
/// Written last: a directory without it is an interrupted installation.
const VERIFIED_MARKER: &str = "pam-desktop-verified";

/// Shared extensions turned on in the runtime's `php.ini`, when the archive
/// has them. Applications list others in `pam-desktop.json`.
pub const DEFAULT_EXTENSIONS: [&str; 11] = [
    "curl",
    "exif",
    "fileinfo",
    "gd",
    "intl",
    "mbstring",
    "openssl",
    "pdo_sqlite",
    "sodium",
    "sqlite3",
    "zip",
];

/// Archives this host knows, newest first: `(version, file, sha256)`. Add the
/// new build of each PHP release here (and its digest from `releases.json`,
/// checked against a second source such as the `sha256sum.txt` of
/// windows.php.net) when PAM Desktop is released.
pub const PINNED: &[(&str, &str, &str)] = &[(
    "8.5.12-1",
    "php-8.5.12-1-nts-Win32-vs17-x64.zip",
    "2a36a2edcb7a5b9252f638fd5e4cf84d7f6e066092654220dc0b63e5cf4e9723",
)];

const ALLOW_UNPINNED_ENV: &str = "PAM_DESKTOP_PHP_ALLOW_UNPINNED";

/// What `install` downloads: the archive, where from, and the digest it must
/// have.
#[derive(Debug, PartialEq, Eq)]
pub struct Source {
    pub release: Release,
    pub url: String,
    pub pinned: bool,
}

/// Picks the archive to install for the newest build `releases.json` lists.
pub fn choose_source(listed: Release, allow_unpinned: bool) -> Result<Source, String> {
    if let Some((_, _, pinned)) = PINNED.iter().find(|(_, file, _)| *file == listed.file) {
        if *pinned != listed.sha256 {
            return Err(format!(
                "windows.php.net lists SHA-256 {} for {}, but PAM Desktop knows it as {pinned}; nothing was installed",
                listed.sha256, listed.file
            ));
        }
        return Ok(Source {
            url: format!("{DOWNLOADS}{}", listed.file),
            release: listed,
            pinned: true,
        });
    }
    if allow_unpinned {
        return Ok(Source {
            url: format!("{DOWNLOADS}{}", listed.file),
            release: listed,
            pinned: false,
        });
    }
    let (version, file, sha256) = PINNED
        .first()
        .ok_or("this PAM Desktop knows no PHP archive")?;
    Ok(Source {
        release: Release {
            version: (*version).to_owned(),
            file: (*file).to_owned(),
            sha256: (*sha256).to_owned(),
        },
        // windows.php.net keeps every earlier build under `archives/`.
        url: format!("{DOWNLOADS}archives/{file}"),
        pinned: true,
    })
}

/// One archive of `releases.json`.
#[derive(Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub file: String,
    pub sha256: String,
}

/// The newest non thread safe x64 build of `branch` listed in `releases.json`.
pub fn parse_releases(json: &[u8], branch: &str) -> Result<Release, String> {
    let releases: serde_json::Value = serde_json::from_slice(json)
        .map_err(|error| format!("windows.php.net sent an unreadable release list: {error}"))?;
    let entry = releases
        .get(branch)
        .ok_or_else(|| format!("windows.php.net lists no PHP {branch} release"))?;
    let version = entry
        .get("version")
        .and_then(serde_json::Value::as_str)
        .filter(|version| valid_version(version, branch))
        .ok_or("the PHP release list has no valid version")?;
    // The newest compiler first (vs17 before vs16).
    let mut builds: Vec<(&String, &serde_json::Value)> = entry
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| key.starts_with(BUILD_PREFIX) && key.ends_with(BUILD_SUFFIX))
        .collect();
    builds.sort_by(|a, b| b.0.cmp(a.0));
    let (build, archive) = builds
        .into_iter()
        .find_map(|(key, value)| value.get("zip").map(|zip| (key, zip)))
        .ok_or_else(|| {
            format!("windows.php.net lists no non thread safe x64 build of PHP {version}")
        })?;
    let file = archive
        .get("path")
        .and_then(serde_json::Value::as_str)
        .ok_or("the PHP release list names no archive")?;
    let expected = format!(
        "php-{version}-nts-Win32-{}-x64.zip",
        build
            .strip_prefix("nts-")
            .and_then(|rest| rest.strip_suffix(BUILD_SUFFIX))
            .unwrap_or_default()
    );
    if file != expected {
        return Err(format!(
            "the PHP release list names an unexpected archive {file:?}"
        ));
    }
    let sha256 = archive
        .get("sha256")
        .and_then(serde_json::Value::as_str)
        .map(str::to_ascii_lowercase)
        .filter(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or("the PHP release list has no SHA-256 for the archive")?;
    Ok(Release {
        version: version.to_owned(),
        file: file.to_owned(),
        sha256,
    })
}

/// `8.5.12` or `8.5.12-1`: digits, dots and one build suffix, in the branch.
fn valid_version(version: &str, branch: &str) -> bool {
    version
        .strip_prefix(branch)
        .and_then(|rest| rest.strip_prefix('.'))
        .is_some_and(|rest| {
            !rest.is_empty()
                && rest.len() <= 16
                && rest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'.' || byte == b'-')
        })
}

/// Where runtimes are installed: `%LOCALAPPDATA%\PAM Desktop\php`.
fn runtimes_root() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .map(|base| Path::new(&base).join("PAM Desktop").join("php"))
}

/// The newest verified runtime of the branch already installed.
pub fn installed() -> Option<PathBuf> {
    installed_in(&runtimes_root()?)
}

fn installed_in(root: &Path) -> Option<PathBuf> {
    let mut versions: Vec<(Vec<u32>, PathBuf)> = fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            if !valid_version(&name, BRANCH) {
                return None;
            }
            let directory = entry.path();
            let php = directory.join("php.exe");
            (directory.join(VERIFIED_MARKER).is_file() && php.is_file()).then(|| {
                let order = name
                    .split(['.', '-'])
                    .map(|part| part.parse().unwrap_or(0))
                    .collect();
                (order, php)
            })
        })
        .collect();
    versions.sort();
    versions.pop().map(|(_, php)| php)
}

/// Whether `php` is a runtime this module installed.
pub fn is_managed(php: &Path) -> bool {
    runtimes_root().is_some_and(|root| php.starts_with(root))
}

/// Why `php` cannot start on this machine: the Visual C++ runtime the
/// official build needs (`vcruntime140.dll`) is neither beside it nor
/// installed. `None` when nothing is missing.
pub fn missing_runtime(php: &Path) -> Option<String> {
    let directory = php.parent()?;
    let system = std::env::var_os("SystemRoot")
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
        .join("System32");
    let missing: Vec<String> = crate::pe::missing_vc_runtime(directory, &[php])
        .into_iter()
        .filter(|name| !system.join(name).is_file())
        .collect();
    if !missing.is_empty() {
        return Some(format!(
            "{} needs {}, from the Visual C++ Redistributable 2015-2022 (x64): install it from {VC_REDIST}",
            php.display(),
            missing.join(", ")
        ));
    }
    // PHP also refuses a Visual C++ runtime older than the one it was built
    // with ("… is not compatible with this PHP build"): ask it.
    let output = crate::console::hidden(&mut std::process::Command::new(php))
        .args(["-n", "-r", "echo PHP_VERSION;"])
        .output()
        .ok()?;
    if output.status.success() {
        return None;
    }
    let said = String::from_utf8_lossy(&output.stderr)
        .lines()
        .chain(String::from_utf8_lossy(&output.stdout).lines())
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_owned())
        .unwrap_or_default();
    Some(format!(
        "{} does not start ({said}); install or update the Visual C++ Redistributable 2015-2022 (x64) from {VC_REDIST}",
        php.display()
    ))
}

const VC_REDIST: &str = "https://aka.ms/vs/17/release/vc_redist.x64.exe";

/// `-d extension=…` for the extensions an application lists that the
/// runtime's `php.ini` does not turn on (`auto` and `all` are not names).
pub fn extension_arguments(php: &Path, listed: &[String]) -> Vec<OsString> {
    let Some(ext) = php.parent().map(|directory| directory.join("ext")) else {
        return Vec::new();
    };
    listed
        .iter()
        .filter(|name| !matches!(name.as_str(), "auto" | "all"))
        .filter(|name| !DEFAULT_EXTENSIONS.contains(&name.as_str()))
        .filter(|name| ext.join(format!("php_{name}.dll")).is_file())
        .flat_map(|name| {
            [
                OsString::from("-d"),
                OsString::from(format!("extension={name}")),
            ]
        })
        .collect()
}

/// The `php.ini` of an installed runtime.
pub fn runtime_ini(directory: &Path) -> String {
    let ext = directory.join("ext");
    let mut ini = format!(
        "; Written by PAM Desktop. Applications in a bundle use the bundle's php.ini.\n\
         extension_dir=\"{}\"\n\
         memory_limit=512M\n\
         display_errors=stderr\n\
         log_errors=On\n\
         default_charset=\"UTF-8\"\n",
        ext.display()
    );
    for name in DEFAULT_EXTENSIONS {
        if ext.join(format!("php_{name}.dll")).is_file() {
            ini.push_str(&format!("extension={name}\n"));
        }
    }
    ini
}

/// Downloads, verifies and installs the newest PHP of the branch; returns
/// its `php.exe`.
pub fn install() -> Result<PathBuf, String> {
    let root = runtimes_root().ok_or("LOCALAPPDATA is not set")?;
    let agent = agent();
    let releases = get(&agent, RELEASES, MAX_RELEASES_BYTES, None)?;
    let listed = parse_releases(&releases, BRANCH)?;
    let allow_unpinned = std::env::var_os(ALLOW_UNPINNED_ENV).is_some_and(|value| value == "1");
    let source = choose_source(listed, allow_unpinned)?;
    let release = &source.release;
    let target = root.join(&release.version);
    if target.join(VERIFIED_MARKER).is_file() && target.join("php.exe").is_file() {
        return Ok(target.join("php.exe"));
    }
    let check = if source.pinned {
        "its SHA-256 is pinned in PAM Desktop and published by windows.php.net"
    } else {
        "WARNING: this build is not pinned in PAM Desktop; only the SHA-256 windows.php.net publishes is checked"
    };
    crate::runtime_install::consent(&format!(
        "No PHP was found. PAM Desktop can install the official PHP {} (NTS x64):\n  \
         from      {}\n  \
         verified  {check}\n  \
         into      {}",
        release.version,
        source.url,
        target.display()
    ))?;
    fs::create_dir_all(&root)
        .map_err(|error| format!("cannot create {}: {error}", root.display()))?;
    eprintln!(
        "pam-desktop: downloading PHP {} from {}",
        release.version, source.url
    );
    let stage = root.join(format!(
        ".{}.partial-{}",
        release.version,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&stage);
    fs::create_dir_all(&stage)
        .map_err(|error| format!("cannot create {}: {error}", stage.display()))?;
    let result = (|| {
        let archive = stage.join(&release.file);
        let bytes = get(&agent, &source.url, MAX_ARCHIVE_BYTES, Some(&archive))?;
        drop(bytes);
        verify(&archive, &release.sha256)?;
        let extracted = stage.join("php");
        fs::create_dir_all(&extracted).map_err(|error| error.to_string())?;
        extract(&archive, &extracted)?;
        let _ = fs::remove_file(&archive);
        let php = extracted.join("php.exe");
        if !php.is_file() {
            return Err("the PHP archive has no php.exe".to_owned());
        }
        fs::write(extracted.join("php.ini"), runtime_ini(&target))
            .map_err(|error| format!("cannot write the PHP configuration: {error}"))?;
        fs::write(
            extracted.join(VERIFIED_MARKER),
            format!("{}\nsha256 {}\n", release.file, release.sha256),
        )
        .map_err(|error| error.to_string())?;
        let _ = fs::remove_dir_all(&target);
        fs::rename(&extracted, &target)
            .map_err(|error| format!("cannot install PHP into {}: {error}", target.display()))?;
        Ok(target.join("php.exe"))
    })();
    let _ = fs::remove_dir_all(&stage);
    let php = result?;
    eprintln!(
        "pam-desktop: installed PHP {} at {}",
        release.version,
        php.display()
    );
    Ok(php)
}

pub(crate) fn agent() -> ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
    let provider = if cfg!(any(target_os = "macos", target_os = "windows")) {
        TlsProvider::NativeTls
    } else {
        TlsProvider::Rustls
    };
    let tls = TlsConfig::builder()
        .provider(provider)
        .root_certs(RootCerts::PlatformVerifier)
        .build();
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .tls_config(tls)
            .https_only(true)
            .max_redirects(3)
            .user_agent(concat!("pam-desktop/", env!("CARGO_PKG_VERSION")))
            .build(),
    )
}

/// GETs `url` into memory, or into `file` when given (then nothing is kept
/// in memory), refusing more than `limit` bytes.
pub(crate) fn get(
    agent: &ureq::Agent,
    url: &str,
    limit: u64,
    file: Option<&Path>,
) -> Result<Vec<u8>, String> {
    let response = agent
        .get(url)
        .call()
        .map_err(|error| format!("cannot download {url}: {error}"))?;
    let mut body = response.into_body().into_reader().take(limit + 1);
    let mut memory = Vec::new();
    let written = match file {
        Some(path) => {
            let mut output = fs::File::create(path)
                .map_err(|error| format!("cannot create {}: {error}", path.display()))?;
            let written = std::io::copy(&mut body, &mut output)
                .map_err(|error| format!("the download of {url} was interrupted: {error}"))?;
            output.flush().map_err(|error| error.to_string())?;
            written
        }
        None => body
            .read_to_end(&mut memory)
            .map_err(|error| format!("the download of {url} was interrupted: {error}"))?
            as u64,
    };
    if written > limit {
        return Err(format!("{url} is larger than expected"));
    }
    Ok(memory)
}

/// Compares the SHA-256 of `archive` with the published one.
pub fn verify(archive: &Path, expected: &str) -> Result<(), String> {
    let mut file = fs::File::open(archive)
        .map_err(|error| format!("cannot read {}: {error}", archive.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let actual: String = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if actual != expected.to_ascii_lowercase() {
        return Err(format!(
            "the PHP archive does not match the SHA-256 windows.php.net published \
             (expected {expected}, got {actual}); nothing was installed"
        ));
    }
    Ok(())
}

/// Extracts the verified archive with the `tar` of Windows (bsdtar, which
/// refuses paths outside the destination), or PowerShell before 1803.
fn extract(archive: &Path, destination: &Path) -> Result<(), String> {
    let system = std::env::var_os("SystemRoot")
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
        .join("System32");
    let tar = system.join("tar.exe");
    if tar.is_file() {
        let status = crate::console::hidden(&mut std::process::Command::new(&tar))
            .arg("-xf")
            .arg(archive)
            .arg("-C")
            .arg(destination)
            .status()
            .map_err(|error| format!("cannot start tar: {error}"))?;
        if status.success() {
            return Ok(());
        }
    }
    let status = crate::console::hidden(&mut std::process::Command::new(
        system.join(r"WindowsPowerShell\v1.0\powershell.exe"),
    ))
    .args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "Expand-Archive -LiteralPath $env:PAM_PHP_ARCHIVE -DestinationPath $env:PAM_PHP_DESTINATION -Force",
    ])
    .env("PAM_PHP_ARCHIVE", archive)
    .env("PAM_PHP_DESTINATION", destination)
    .status()
    .map_err(|error| format!("cannot start PowerShell to extract PHP: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "extracting the PHP archive failed with status {status}"
        ))
    }
}

/// The certificate authorities PHP trusts on Windows, where its OpenSSL and
/// curl have no store of their own: the roots of the Windows store (the
/// enterprise ones included) and Mozilla's roots (`webpki-root-certs`,
/// updated with PAM Desktop), which cover the roots Windows only fetches the
/// first time a site needs them. Written as PEM to `directory` when it
/// changed; `None` when it cannot be written.
pub fn ca_bundle(directory: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    let (system, mozilla): (Vec<Vec<u8>>, Vec<&[u8]>) = (
        system_roots(),
        webpki_root_certs::TLS_SERVER_ROOT_CERTS
            .iter()
            .map(|certificate| certificate.as_ref())
            .collect(),
    );
    #[cfg(not(windows))]
    let (system, mozilla): (Vec<Vec<u8>>, Vec<&[u8]>) = (Vec::new(), Vec::new());
    let pem = ca_pem(system.iter().map(Vec::as_slice).chain(mozilla));
    if pem.is_empty() {
        return None;
    }
    let path = directory.join("php-ca-bundle.pem");
    if fs::read(&path).ok().as_deref() != Some(pem.as_bytes()) {
        fs::create_dir_all(directory).ok()?;
        let partial = directory.join(format!("php-ca-bundle.pem.{}", std::process::id()));
        fs::write(&partial, pem.as_bytes()).ok()?;
        if fs::rename(&partial, &path).is_err() {
            let _ = fs::remove_file(&partial);
            // Another instance wrote it first.
            return path.is_file().then_some(path);
        }
    }
    Some(path)
}

/// The `-d` options that point PHP's curl and OpenSSL at `bundle`.
pub fn ca_arguments(bundle: &Path) -> Vec<OsString> {
    let path = bundle.display();
    ["curl.cainfo", "openssl.cafile"]
        .into_iter()
        .flat_map(|setting| {
            [
                OsString::from("-d"),
                OsString::from(format!("{setting}={path}")),
            ]
        })
        .collect()
}

/// DER certificates as one PEM text, each once, in order.
pub fn ca_pem<'a>(certificates: impl Iterator<Item = &'a [u8]>) -> String {
    use base64::Engine as _;
    let mut seen = std::collections::HashSet::new();
    let mut pem = String::new();
    for der in certificates {
        if der.is_empty() || !seen.insert(Sha256::digest(der)) {
            continue;
        }
        let encoded = base64::engine::general_purpose::STANDARD.encode(der);
        pem.push_str("-----BEGIN CERTIFICATE-----\n");
        for line in encoded.as_bytes().chunks(64) {
            pem.push_str(std::str::from_utf8(line).unwrap_or_default());
            pem.push('\n');
        }
        pem.push_str("-----END CERTIFICATE-----\n");
    }
    pem
}

/// The certificates of the `ROOT` system store (machine and user roots,
/// third-party and enterprise roots included).
#[cfg(windows)]
fn system_roots() -> Vec<Vec<u8>> {
    use windows_sys::Win32::Security::Cryptography::{
        CertCloseStore, CertEnumCertificatesInStore, CertOpenSystemStoreW,
    };
    let name: Vec<u16> = "ROOT".encode_utf16().chain(std::iter::once(0)).collect();
    let mut roots = Vec::new();
    // SAFETY: the store is opened, enumerated and closed here; every
    // context is read before the next call frees it.
    unsafe {
        let store = CertOpenSystemStoreW(0, name.as_ptr());
        if store.is_null() {
            return roots;
        }
        let mut context = CertEnumCertificatesInStore(store, std::ptr::null());
        while !context.is_null() {
            let certificate = &*context;
            if !certificate.pbCertEncoded.is_null() && certificate.cbCertEncoded > 0 {
                roots.push(
                    std::slice::from_raw_parts(
                        certificate.pbCertEncoded,
                        certificate.cbCertEncoded as usize,
                    )
                    .to_vec(),
                );
            }
            context = CertEnumCertificatesInStore(store, context);
        }
        CertCloseStore(store, 0);
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASES_SAMPLE: &str = r#"{
        "8.4": {"version": "8.4.20", "nts-vs17-x64": {"zip": {"path": "php-8.4.20-nts-Win32-vs17-x64.zip", "sha256": "00"}}},
        "8.5": {
            "version": "8.5.12-1",
            "ts-vs17-x64": {"zip": {"path": "php-8.5.12-1-Win32-vs17-x64.zip", "sha256": "0dacd6484b092b5e9ae9fa3d7f96a937caf9d874eee2d980b68ed3129741144a"}},
            "nts-vs17-x86": {"zip": {"path": "php-8.5.12-1-nts-Win32-vs17-x86.zip", "sha256": "1111111111111111111111111111111111111111111111111111111111111111"}},
            "nts-vs17-x64": {"mtime": "2026-10-26T13:05:05+00:00", "zip": {"path": "php-8.5.12-1-nts-Win32-vs17-x64.zip", "size": "34.63MB", "sha256": "2A36A2EDCB7A5B9252F638FD5E4CF84D7F6E066092654220DC0B63E5CF4E9723"}}
        }
    }"#;

    #[test]
    fn picks_the_non_thread_safe_x64_archive_and_its_published_digest() {
        let release = parse_releases(RELEASES_SAMPLE.as_bytes(), "8.5").unwrap();
        assert_eq!(
            release,
            Release {
                version: "8.5.12-1".into(),
                file: "php-8.5.12-1-nts-Win32-vs17-x64.zip".into(),
                sha256: "2a36a2edcb7a5b9252f638fd5e4cf84d7f6e066092654220dc0b63e5cf4e9723".into(),
            }
        );
        assert!(parse_releases(RELEASES_SAMPLE.as_bytes(), "9.0").is_err());
    }

    #[test]
    fn refuses_unexpected_names_versions_and_digests() {
        for sample in [
            r#"{"8.5": {"version": "8.5.1", "nts-vs17-x64": {"zip": {"path": "../evil.zip", "sha256": "2a36a2edcb7a5b9252f638fd5e4cf84d7f6e066092654220dc0b63e5cf4e9723"}}}}"#,
            r#"{"8.5": {"version": "8.5.1", "nts-vs17-x64": {"zip": {"path": "php-8.5.1-nts-Win32-vs17-x64.zip"}}}}"#,
            r#"{"8.5": {"version": "8.5.1", "nts-vs17-x64": {"zip": {"path": "php-8.5.1-nts-Win32-vs17-x64.zip", "sha256": "xyz"}}}}"#,
            r#"{"8.5": {"version": "8.5.1/../x", "nts-vs17-x64": {"zip": {"path": "php-8.5.1/../x-nts-Win32-vs17-x64.zip", "sha256": "2a36a2edcb7a5b9252f638fd5e4cf84d7f6e066092654220dc0b63e5cf4e9723"}}}}"#,
            r#"{"8.5": {"version": "8.4.1", "nts-vs17-x64": {"zip": {"path": "php-8.4.1-nts-Win32-vs17-x64.zip", "sha256": "2a36a2edcb7a5b9252f638fd5e4cf84d7f6e066092654220dc0b63e5cf4e9723"}}}}"#,
            r#"{"8.5": {"version": "8.5.1", "ts-vs17-x64": {"zip": {"path": "php-8.5.1-Win32-vs17-x64.zip", "sha256": "2a36a2edcb7a5b9252f638fd5e4cf84d7f6e066092654220dc0b63e5cf4e9723"}}}}"#,
            "not json",
        ] {
            assert!(
                parse_releases(sample.as_bytes(), "8.5").is_err(),
                "{sample}"
            );
        }
    }

    #[test]
    fn archives_must_match_the_pinned_digest_or_fall_back_to_a_pinned_build() {
        let (version, file, sha256) = PINNED[0];
        let listed = |file: &str, sha256: &str| Release {
            version: "x".into(),
            file: file.into(),
            sha256: sha256.into(),
        };
        let source = choose_source(listed(file, sha256), false).unwrap();
        assert!(source.pinned);
        assert_eq!(source.url, format!("{DOWNLOADS}{file}"));
        // The same server cannot swap the digest of a known archive.
        assert!(choose_source(listed(file, &"1".repeat(64)), false).is_err());
        assert!(choose_source(listed(file, &"1".repeat(64)), true).is_err());
        // An unknown, newer build falls back to the newest pinned one...
        let unknown = "php-8.5.99-nts-Win32-vs17-x64.zip";
        let source = choose_source(listed(unknown, &"2".repeat(64)), false).unwrap();
        assert!(source.pinned);
        assert_eq!(source.release.version, version);
        assert_eq!(source.release.sha256, sha256);
        assert_eq!(source.url, format!("{DOWNLOADS}archives/{file}"));
        // ...unless the user accepts unpinned builds.
        let source = choose_source(listed(unknown, &"2".repeat(64)), true).unwrap();
        assert!(!source.pinned);
        assert_eq!(source.release.file, unknown);
        for (version, file, sha256) in PINNED {
            assert!(valid_version(version, BRANCH), "{version}");
            assert!(
                file.starts_with(&format!("php-{version}-nts-Win32-vs")),
                "{file}"
            );
            assert!(
                sha256.len() == 64
                    && sha256
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            );
        }
    }

    #[test]
    fn verifies_the_archive_digest() {
        let directory =
            std::env::temp_dir().join(format!("pam-windows-php-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let archive = directory.join("php.zip");
        fs::write(&archive, b"abc").unwrap();
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(verify(&archive, abc).is_ok());
        assert!(verify(&archive, &abc.to_ascii_uppercase()).is_ok());
        assert!(verify(&archive, &"0".repeat(64)).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn finds_the_newest_verified_runtime_and_the_listed_extensions() {
        let root =
            std::env::temp_dir().join(format!("pam-windows-php-root-{}", std::process::id()));
        for (version, verified) in [
            ("8.5.9", true),
            ("8.5.12-1", true),
            ("8.5.13", false),
            ("8.4.30", true),
        ] {
            let directory = root.join(version);
            fs::create_dir_all(directory.join("ext")).unwrap();
            fs::write(directory.join("php.exe"), b"").unwrap();
            if verified {
                fs::write(directory.join(VERIFIED_MARKER), b"").unwrap();
            }
        }
        let php = installed_in(&root).unwrap();
        assert_eq!(php, root.join("8.5.12-1").join("php.exe"));
        let ext = root.join("8.5.12-1").join("ext");
        for name in ["gmp", "mbstring", "intl"] {
            fs::write(ext.join(format!("php_{name}.dll")), b"").unwrap();
        }
        let listed: Vec<String> = ["auto", "gmp", "mbstring", "ldap"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            extension_arguments(&php, &listed),
            vec![OsString::from("-d"), OsString::from("extension=gmp")]
        );
        let ini = runtime_ini(&root.join("8.5.12-1"));
        assert!(ini.contains("extension=intl\n") && ini.contains("extension=mbstring\n"));
        assert!(!ini.contains("extension=curl"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ca_bundles_are_pem_without_repeats() {
        let pem = ca_pem([&b"abc"[..], &b""[..], &b"abc"[..], &[0_u8; 50][..]].into_iter());
        assert_eq!(pem.matches("-----BEGIN CERTIFICATE-----\n").count(), 2);
        assert!(pem.starts_with("-----BEGIN CERTIFICATE-----\nYWJj\n-----END CERTIFICATE-----\n"));
        // 50 bytes are 68 base64 characters: a full line of 64, then 4.
        assert!(pem.contains(&format!("\n{}\n{}\n-----END", "A".repeat(64), "AAA=")));
        let arguments = ca_arguments(Path::new(r"C:\Users\Ana Maria\cache\php-ca-bundle.pem"));
        assert_eq!(arguments.len(), 4);
        assert_eq!(
            arguments[1],
            OsString::from(r"curl.cainfo=C:\Users\Ana Maria\cache\php-ca-bundle.pem")
        );
        assert!(ca_bundle(&std::env::temp_dir()).is_none() || cfg!(windows));
    }
}
