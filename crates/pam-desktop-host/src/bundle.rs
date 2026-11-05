//! The layout of a packaged application and its integrity manifest.
//!
//! ```text
//! <bundle>/
//!   manifest.json            identity, target and SHA-256 of every file
//!   bin/pam-desktop[.exe]    this host
//!   bin/<name>[.exe]         launcher (a script; on Windows a copy of the host)
//!   bin/pam-desktop-php      starts the bundled PHP (Unix)
//!   runtime/php/bin|lib|ext  the PHP runtime, its libraries and extensions
//!   lib/                     libraries of the host the system does not provide
//!   etc/php.ini
//!   app/                     the application
//! ```
//!
//! The packager writes this layout, the updater verifies it before and after
//! swapping bundles, and the launcher of a Windows bundle reads it to start
//! the application.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitCode};

use pam_desktop_ui::modules::support::hex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::manifest::{validate_identifier, validate_version};

/// The layout described above. Bundles of PAM Desktop 1.x are schema 1 and
/// are neither produced nor accepted.
pub const SCHEMA_VERSION: u8 = 2;
pub const MANIFEST: &str = "manifest.json";
const MAX_MANIFEST_BYTES: u64 = 10 * 1024 * 1024;

/// Directory of the real PHP binary; on Windows its DLLs sit beside it.
pub const PHP_BIN: &str = "runtime/php/bin";
pub const PHP_LIB: &str = "runtime/php/lib";
pub const PHP_EXTENSIONS: &str = "runtime/php/ext";
pub const PHP_INI: &str = "etc/php.ini";
/// The script in `bin` that starts the bundled PHP with its own libraries
/// and configuration (Unix; Windows finds DLLs beside `php.exe`).
pub const PHP_WRAPPER: &str = "pam-desktop-php";
/// Read by the bundled `php.ini`: a bundle does not know where it is installed.
pub const PHP_EXTENSION_DIR_VARIABLE: &str = "PAM_DESKTOP_PHP_EXTENSION_DIR";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleManifest {
    pub schema_version: u8,
    pub application: BundleApplication,
    /// The project directory the launcher runs, relative to the bundle.
    pub project: String,
    /// The script the host starts in it, relative to the bundle.
    pub entry: String,
    /// The launcher, relative to the bundle.
    pub launcher: String,
    pub runtime: BundleRuntime,
    pub target: BundleTarget,
    pub source_date_epoch: u64,
    pub files: Vec<BundleFile>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleApplication {
    pub identifier: String,
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub build: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub publisher: String,
    #[serde(default)]
    pub category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updates: Option<BundleUpdates>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleUpdates {
    pub endpoint: String,
    pub public_key: String,
    pub channel: String,
    pub policy: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleRuntime {
    pub pam_desktop: String,
    /// Version of the bundled PHP, `unknown` when it could not be asked.
    pub php: String,
    /// File name of the PHP binary inside `runtime/php/bin`.
    pub php_binary: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleTarget {
    pub operating_system: String,
    pub architecture: String,
    pub abi: String,
}

impl BundleTarget {
    /// The system this host was built for.
    #[must_use]
    pub fn current() -> Self {
        let abi = if cfg!(all(target_os = "windows", target_env = "msvc")) {
            "msvc"
        } else if cfg!(target_os = "windows") {
            "gnu"
        } else if cfg!(target_os = "macos") {
            "darwin"
        } else if cfg!(target_env = "musl") {
            "musl"
        } else {
            "glibc"
        };
        Self {
            operating_system: std::env::consts::OS.to_owned(),
            architecture: std::env::consts::ARCH.to_owned(),
            abi: abi.to_owned(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleFile {
    /// Relative to the bundle, with forward slashes.
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

/// Byte length and lowercase hexadecimal SHA-256 of a file.
///
/// # Errors
///
/// When the file cannot be read.
pub fn hash_file(path: &Path) -> Result<(u64, String), String> {
    let mut file =
        File::open(path).map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 128 * 1024];
    let mut bytes = 0_u64;
    loop {
        let count = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("cannot hash {}: {error}", path.display())),
        };
        digest.update(&buffer[..count]);
        bytes += count as u64;
    }
    Ok((bytes, hex(&digest.finalize())))
}

/// Fails unless the file has exactly this SHA-256.
///
/// # Errors
///
/// When the file cannot be read or its digest differs.
pub fn verify_file_hash(path: &Path, expected: &str) -> Result<(), String> {
    let (_, actual) = hash_file(path)?;
    if actual != expected {
        return Err(format!("SHA-256 mismatch for {}", path.display()));
    }
    Ok(())
}

/// A relative path made only of plain names.
///
/// # Errors
///
/// When the path is empty, absolute or has `.`/`..` components.
pub fn validate_relative_path(path: &Path, label: &str) -> Result<(), String> {
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

/// Every file of the bundle except its manifest, as `(relative path, path)`
/// sorted by relative path. A bundle holds only directories and regular
/// files: anything else could point outside of what the manifest vouches for.
///
/// # Errors
///
/// When the tree cannot be read or holds a symbolic link or a special file.
pub fn files(root: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    fn visit(
        root: &Path,
        directory: &Path,
        files: &mut Vec<(String, PathBuf)>,
    ) -> Result<(), String> {
        for entry in fs::read_dir(directory).map_err(|error| {
            format!(
                "cannot read bundle directory {}: {error}",
                directory.display()
            )
        })? {
            let entry = entry.map_err(|error| format!("cannot read bundle entry: {error}"))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
            if file_type.is_symlink() {
                return Err(format!(
                    "bundle contains a symbolic link: {}",
                    path.display()
                ));
            }
            if file_type.is_dir() {
                visit(root, &path, files)?;
            } else if file_type.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| "bundle path escaped its root".to_owned())?;
                if relative == Path::new(MANIFEST) {
                    continue;
                }
                let relative = relative
                    .to_str()
                    .ok_or_else(|| format!("bundle path is not valid UTF-8: {}", path.display()))?
                    .replace('\\', "/");
                files.push((relative, path));
            } else {
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
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

/// Writes `manifest.json` with the digest of every file now in `root`.
///
/// # Errors
///
/// When a file cannot be hashed or the manifest cannot be written.
pub fn write_manifest(root: &Path, mut manifest: BundleManifest) -> Result<(), String> {
    manifest.files = files(root)?
        .into_iter()
        .map(|(relative, path)| {
            hash_file(&path).map(|(bytes, sha256)| BundleFile {
                path: relative,
                bytes,
                sha256,
            })
        })
        .collect::<Result<_, _>>()?;
    let mut encoded = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("cannot serialize the bundle manifest: {error}"))?;
    encoded.push(b'\n');
    fs::write(root.join(MANIFEST), encoded)
        .map_err(|error| format!("cannot write the bundle manifest: {error}"))
}

/// Reads and validates the shape of `manifest.json`.
///
/// # Errors
///
/// When the manifest is missing, oversized, of another schema or names an
/// invalid application.
pub fn read_manifest(root: &Path) -> Result<BundleManifest, String> {
    let path = root.join(MANIFEST);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("cannot open the bundle manifest: {error}"))?;
    if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES {
        return Err("the bundle manifest is not a regular file within the size limit".to_owned());
    }
    let bytes =
        fs::read(&path).map_err(|error| format!("cannot read the bundle manifest: {error}"))?;
    let manifest: BundleManifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid bundle manifest: {error}"))?;
    if manifest.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "bundle manifest schema {} is unsupported; expected {SCHEMA_VERSION}",
            manifest.schema_version
        ));
    }
    validate_identifier(&manifest.application.identifier)?;
    validate_version(&manifest.application.version)?;
    validate_relative_path(Path::new(&manifest.project), "bundle project")?;
    validate_relative_path(Path::new(&manifest.entry), "bundle entry")?;
    validate_relative_path(Path::new(&manifest.launcher), "bundle launcher")?;
    Ok(manifest)
}

/// Checks that `root` holds exactly the files its manifest declares, with
/// the declared sizes and digests.
///
/// # Errors
///
/// When the manifest is invalid or any file is missing, extra or different.
pub fn verify(root: &Path) -> Result<BundleManifest, String> {
    let manifest = read_manifest(root)?;
    let mut expected = std::collections::HashMap::with_capacity(manifest.files.len());
    for file in &manifest.files {
        validate_relative_path(Path::new(&file.path), "bundle file")?;
        if expected
            .insert(file.path.as_str(), (file.bytes, file.sha256.as_str()))
            .is_some()
        {
            return Err(format!(
                "bundle manifest path {:?} is duplicated",
                file.path
            ));
        }
    }
    let actual = files(root)?;
    if actual.len() != expected.len() {
        return Err("bundle contents do not match its manifest".to_owned());
    }
    for (relative, path) in actual {
        let (bytes, sha256) = expected
            .remove(relative.as_str())
            .ok_or_else(|| format!("bundle contains an undeclared file: {relative}"))?;
        let length = path
            .metadata()
            .map_err(|error| format!("cannot inspect bundle file {relative}: {error}"))?
            .len();
        if length != bytes {
            return Err(format!("bundle file size mismatch: {relative}"));
        }
        verify_file_hash(&path, sha256)?;
    }
    if !expected.is_empty() {
        return Err("bundle is missing files declared by its manifest".to_owned());
    }
    Ok(manifest)
}

/// The installed bundle the running host belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    /// What an update replaces: the bundle, or the `.app` around it on macOS.
    pub root: PathBuf,
    /// The launcher relative to `root`, when the application was started by it.
    pub launcher: Option<PathBuf>,
}

impl Installed {
    /// Finds the bundle of `application_id`: the one the launcher announced,
    /// otherwise the one this executable sits in.
    ///
    /// A manifest that names another application is not ours to replace: the
    /// host may be running from a shared cache or a development checkout.
    #[must_use]
    pub fn discover(application_id: &str) -> Option<Self> {
        let variable = |name: &str| {
            std::env::var_os(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        };
        let configured =
            variable("PAM_DESKTOP_UPDATE_ROOT").or_else(|| variable("PAM_DESKTOP_BUNDLE_ROOT"));
        let inferred = || {
            std::env::current_exe()
                .ok()
                .and_then(|executable| executable.parent()?.parent().map(Path::to_path_buf))
        };
        let root = configured.or_else(inferred)?.canonicalize().ok()?;
        Self::at(
            root,
            application_id,
            variable("PAM_DESKTOP_LAUNCHER").as_deref(),
        )
    }

    fn at(root: PathBuf, application_id: &str, launcher: Option<&Path>) -> Option<Self> {
        let runtime = Self::runtime_directory(&root);
        let manifest = read_manifest(&runtime).ok()?;
        if manifest.application.identifier != application_id {
            return None;
        }
        let launcher = launcher
            .and_then(|path| path.canonicalize().ok())
            .and_then(|path| path.strip_prefix(&root).ok().map(Path::to_path_buf));
        Some(Self { root, launcher })
    }

    /// Where the bundle layout starts inside `root`.
    #[must_use]
    pub fn runtime_directory(root: &Path) -> PathBuf {
        if root.extension().is_some_and(|extension| extension == "app") {
            root.join("Contents/Resources/runtime")
        } else {
            root.to_path_buf()
        }
    }
}

/// The environment and command a launcher starts the application with.
#[derive(Debug, PartialEq, Eq)]
pub struct Launch {
    pub program: PathBuf,
    pub arguments: Vec<OsString>,
    pub directory: PathBuf,
    pub environment: Vec<(&'static str, OsString)>,
}

impl Launch {
    /// The bundle `executable` is the launcher of. The launcher of a Windows
    /// bundle is this very host under the application's name, because a
    /// script cannot be the executable of a desktop application there.
    #[must_use]
    pub fn for_executable(executable: &Path, arguments: &[OsString]) -> Option<Self> {
        if executable
            .file_stem()
            .is_some_and(|stem| stem == "pam-desktop")
        {
            return None;
        }
        let bin = executable.parent()?;
        let root = bin.parent()?;
        if bin.file_name()? != "bin" {
            return None;
        }
        let manifest = read_manifest(root).ok()?;
        let relative = executable
            .strip_prefix(root)
            .ok()?
            .to_str()?
            .replace('\\', "/");
        (manifest.launcher == relative).then(|| Self::new(root, &manifest, executable, arguments))
    }

    fn new(
        root: &Path,
        manifest: &BundleManifest,
        launcher: &Path,
        arguments: &[OsString],
    ) -> Self {
        let project = root.join(&manifest.project);
        let wrapper = root.join("bin").join(PHP_WRAPPER);
        let php = if wrapper.is_file() {
            wrapper
        } else {
            root.join(PHP_BIN).join(&manifest.runtime.php_binary)
        };
        let mut environment = vec![
            ("PAM_DESKTOP_BUNDLE", OsString::from("1")),
            ("PAM_DESKTOP_BUNDLE_ROOT", root.as_os_str().to_owned()),
            ("PAM_DESKTOP_LAUNCHER", launcher.as_os_str().to_owned()),
            ("PAM_DESKTOP_PHP", php.into_os_string()),
            ("PHPRC", root.join(PHP_INI).into_os_string()),
            ("PHP_INI_SCAN_DIR", OsString::new()),
            (
                PHP_EXTENSION_DIR_VARIABLE,
                root.join(PHP_EXTENSIONS).into_os_string(),
            ),
        ];
        // A link or file the system opened the application with.
        if let Some(url) = arguments
            .first()
            .filter(|_| std::env::var_os("PAM_DESKTOP_LAUNCH_URL").is_none())
        {
            environment.push(("PAM_DESKTOP_LAUNCH_URL", url.clone()));
        }
        Self {
            program: root
                .join("bin")
                .join(format!("pam-desktop{}", std::env::consts::EXE_SUFFIX)),
            arguments: vec![OsString::from("run"), project.clone().into_os_string()],
            directory: project,
            environment,
        }
    }

    /// Runs the application and returns its exit status.
    #[must_use]
    pub fn run(self) -> ExitCode {
        let mut command = Command::new(&self.program);
        command.args(&self.arguments).current_dir(&self.directory);
        for (name, value) in &self.environment {
            command.env(name, value);
        }
        match command.status() {
            Ok(status) => status
                .code()
                .and_then(|code| u8::try_from(code).ok())
                .map_or(ExitCode::FAILURE, ExitCode::from),
            Err(error) => {
                eprintln!(
                    "pam-desktop: cannot start {}: {error}",
                    self.program.display()
                );
                ExitCode::FAILURE
            }
        }
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;

    /// A manifest for `root` as the packager would write it, without files.
    #[must_use]
    pub fn manifest(identifier: &str, version: &str) -> BundleManifest {
        BundleManifest {
            schema_version: SCHEMA_VERSION,
            application: BundleApplication {
                identifier: identifier.to_owned(),
                name: String::from("Fixture"),
                version: version.to_owned(),
                build: String::new(),
                description: String::new(),
                publisher: String::from("Fixture"),
                category: String::from("utility"),
                updates: None,
            },
            project: String::from("app"),
            entry: String::from("app/index.php"),
            launcher: String::from("bin/fixture"),
            runtime: BundleRuntime {
                pam_desktop: env!("CARGO_PKG_VERSION").to_owned(),
                php: String::from("8.5.0"),
                php_binary: String::from("php"),
            },
            target: BundleTarget::current(),
            source_date_epoch: 0,
            files: Vec::new(),
        }
    }

    /// A minimal valid bundle whose payload file holds `payload`.
    pub fn write_bundle(root: &Path, identifier: &str, version: &str, payload: &[u8]) {
        fs::create_dir_all(root.join("app")).expect("app directory");
        fs::write(root.join("app/payload.txt"), payload).expect("payload");
        write_manifest(root, manifest(identifier, version)).expect("manifest");
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{manifest, write_bundle};
    use super::*;
    use pam_desktop_ui::modules::test_support::temporary_directory;

    #[test]
    fn verifies_every_file_of_a_bundle() {
        let root = temporary_directory("bundle-verify");
        write_bundle(&root, "com.pushin.test", "1.0.0", b"verified");
        // A manifest deeper in the tree is an ordinary file.
        fs::write(root.join("app/manifest.json"), b"{}").unwrap();
        write_manifest(&root, manifest("com.pushin.test", "1.0.0")).unwrap();
        let verified = verify(&root).expect("bundle should verify");
        let paths: Vec<&str> = verified
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect();
        assert_eq!(paths, ["app/manifest.json", "app/payload.txt"]);
        assert_eq!(verified.files[1].bytes, 8);
        assert_eq!(
            verified.files[1].sha256,
            pam_desktop_ui::modules::support::sha256_hex(b"verified")
        );

        fs::write(root.join("app/payload.txt"), b"tampered").unwrap();
        assert!(verify(&root).unwrap_err().contains("SHA-256 mismatch"));
        fs::write(root.join("app/payload.txt"), b"longer than before").unwrap();
        assert!(verify(&root).unwrap_err().contains("size mismatch"));
        fs::write(root.join("app/payload.txt"), b"verified").unwrap();
        fs::write(root.join("app/extra.txt"), b"x").unwrap();
        assert!(verify(&root).unwrap_err().contains("do not match"));
        fs::remove_file(root.join("app/extra.txt")).unwrap();
        fs::rename(root.join("app/payload.txt"), root.join("app/renamed.txt")).unwrap();
        assert!(verify(&root).unwrap_err().contains("undeclared"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_other_schemas_and_unsafe_manifests() {
        let root = temporary_directory("bundle-schema");
        write_bundle(&root, "com.pushin.test", "1.0.0", b"x");
        let rewrite = |change: &dyn Fn(&mut serde_json::Value)| {
            let mut document: serde_json::Value =
                serde_json::from_slice(&fs::read(root.join(MANIFEST)).unwrap()).unwrap();
            change(&mut document);
            fs::write(root.join(MANIFEST), serde_json::to_vec(&document).unwrap()).unwrap();
        };
        rewrite(&|document| document["schemaVersion"] = 1.into());
        assert!(verify(&root).unwrap_err().contains("unsupported"));
        rewrite(&|document| {
            document["schemaVersion"] = SCHEMA_VERSION.into();
            document["files"][0]["path"] = "../outside".into();
        });
        assert!(
            verify(&root)
                .unwrap_err()
                .contains("normalized relative path")
        );
        rewrite(&|document| {
            document["files"][0]["path"] = "app/payload.txt".into();
            document["application"]["identifier"] = "Not An Id".into();
        });
        assert!(verify(&root).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symbolic_links() {
        let root = temporary_directory("bundle-symlink");
        write_bundle(&root, "com.pushin.test", "1.0.0", b"x");
        std::os::unix::fs::symlink("/etc/hostname", root.join("app/link")).unwrap();
        assert!(verify(&root).unwrap_err().contains("symbolic link"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_unsafe_relative_paths() {
        assert!(validate_relative_path(Path::new("bin/application"), "launcher").is_ok());
        assert!(validate_relative_path(Path::new("../outside"), "launcher").is_err());
        assert!(validate_relative_path(Path::new("/outside"), "launcher").is_err());
        assert!(validate_relative_path(Path::new(""), "launcher").is_err());
    }

    #[test]
    fn an_installed_bundle_belongs_to_its_application() {
        let directory = temporary_directory("bundle-installed")
            .canonicalize()
            .unwrap();
        let root = directory.join("fixture");
        write_bundle(&root, "com.pushin.test", "1.0.0", b"x");
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::write(root.join("bin/fixture"), b"#!/bin/sh\n").unwrap();
        let installed = Installed::at(
            root.clone(),
            "com.pushin.test",
            Some(&root.join("bin/fixture")),
        )
        .expect("the bundle is ours");
        assert_eq!(installed.root, root);
        assert_eq!(
            installed.launcher.as_deref(),
            Some(Path::new("bin/fixture"))
        );
        // A launcher outside the bundle is not relaunched.
        assert_eq!(
            Installed::at(root.clone(), "com.pushin.test", Some(&directory))
                .unwrap()
                .launcher,
            None
        );
        assert_eq!(Installed::at(root.clone(), "com.pushin.other", None), None);
        assert_eq!(
            Installed::at(directory.join("missing"), "com.pushin.test", None),
            None
        );
        assert_eq!(
            Installed::runtime_directory(Path::new("/Applications/Fixture.app")),
            Path::new("/Applications/Fixture.app/Contents/Resources/runtime")
        );
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_renamed_host_inside_a_bundle_launches_it() {
        let root = temporary_directory("bundle-launch");
        write_bundle(&root, "com.pushin.test", "1.0.0", b"x");
        let launcher = root.join("bin/fixture");
        let arguments = [OsString::from("myapp://open/42")];
        let launch =
            Launch::for_executable(&launcher, &arguments).expect("the launcher of the bundle");
        assert_eq!(
            launch.program,
            root.join("bin")
                .join(format!("pam-desktop{}", std::env::consts::EXE_SUFFIX))
        );
        assert_eq!(
            launch.arguments,
            [OsString::from("run"), root.join("app").into_os_string()]
        );
        assert_eq!(launch.directory, root.join("app"));
        let variable = |name: &str| {
            launch
                .environment
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(
            variable("PAM_DESKTOP_PHP"),
            Some(root.join("runtime/php/bin/php").into_os_string())
        );
        assert_eq!(
            variable("PAM_DESKTOP_BUNDLE_ROOT"),
            Some(root.clone().into_os_string())
        );
        assert_eq!(
            variable("PAM_DESKTOP_LAUNCHER"),
            Some(launcher.clone().into_os_string())
        );
        assert_eq!(
            variable("PHPRC"),
            Some(root.join("etc/php.ini").into_os_string())
        );
        assert_eq!(variable("PHP_INI_SCAN_DIR"), Some(OsString::new()));
        assert_eq!(
            variable("PAM_DESKTOP_LAUNCH_URL"),
            Some(OsString::from("myapp://open/42"))
        );

        // The host itself, another file of `bin` and a host outside a bundle are not launchers.
        assert_eq!(
            Launch::for_executable(&root.join("bin/pam-desktop"), &[]),
            None
        );
        assert_eq!(Launch::for_executable(&root.join("bin/other"), &[]), None);
        assert_eq!(Launch::for_executable(&root.join("app/fixture"), &[]), None);
        let _ = fs::remove_dir_all(&root);
    }
}
