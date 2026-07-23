use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use pam_desktop_protocol::{ApplicationCategory, ApplicationManifest, Bootstrap, PROTOCOL_VERSION};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::project::Project;

const DEFAULT_SOURCE_DATE_EPOCH: u64 = 0;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
enum PackageFormat {
    Directory = 1,
    Portable = 2,
    Debian = 3,
}

/// Parsed `pam desktop build` action.
pub enum BuildCommand {
    Help,
    Build(BuildOptions),
}

/// Validated inputs for a Linux desktop distribution build.
pub struct BuildOptions {
    pub project: PathBuf,
    pub output: Option<PathBuf>,
    formats: BTreeSet<PackageFormat>,
    force: bool,
}

impl BuildOptions {
    /// Parses build paths, formats and replacement policy.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown options, missing values, repeated project
    /// paths or unsupported package formats.
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<BuildCommand, String> {
        let mut arguments = arguments.into_iter();
        let mut project = None;
        let mut output = None;
        let mut formats = BTreeSet::from([PackageFormat::Directory, PackageFormat::Portable]);
        let mut explicit_format = false;
        let mut force = false;

        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some("--help" | "-h") => return Ok(BuildCommand::Help),
                Some("--output" | "-o") => {
                    let value = arguments
                        .next()
                        .ok_or_else(|| "--output requires a directory".to_owned())?;
                    if output.replace(PathBuf::from(value)).is_some() {
                        return Err("--output may only be declared once".to_owned());
                    }
                }
                Some("--format") => {
                    let value = arguments
                        .next()
                        .ok_or_else(|| "--format requires a value".to_owned())?;
                    let value = value
                        .to_str()
                        .ok_or_else(|| "package format must be valid UTF-8".to_owned())?;
                    if !explicit_format {
                        formats.clear();
                        explicit_format = true;
                    }
                    match value {
                        "directory" => {
                            formats.insert(PackageFormat::Directory);
                        }
                        "portable" | "tar.gz" => {
                            formats.insert(PackageFormat::Portable);
                        }
                        "deb" | "debian" => {
                            formats.insert(PackageFormat::Debian);
                        }
                        "all" => {
                            formats.extend([
                                PackageFormat::Directory,
                                PackageFormat::Portable,
                                PackageFormat::Debian,
                            ]);
                        }
                        _ => {
                            return Err(format!(
                                "unknown package format {value:?}; expected directory, portable, deb, or all"
                            ));
                        }
                    }
                }
                Some("--force") => force = true,
                Some(value) if value.starts_with('-') => {
                    return Err(format!("unknown build option {value:?}"));
                }
                _ => {
                    if project.replace(PathBuf::from(argument)).is_some() {
                        return Err("build accepts only one project directory".to_owned());
                    }
                }
            }
        }

        if formats.is_empty() {
            return Err("at least one package format is required".to_owned());
        }
        Ok(BuildCommand::Build(Self {
            project: project.unwrap_or_else(|| PathBuf::from(".")),
            output,
            formats,
            force,
        }))
    }
}

/// Paths published by one successful distribution build.
pub struct BuildResult {
    pub artifacts: Vec<PathBuf>,
}

/// Builds a self-contained application-runtime bundle and requested installers.
///
/// # Errors
///
/// Returns an error when the host platform is unsupported, binaries or project
/// files cannot be materialized, metadata is unsafe, a packaging tool fails, or
/// an artifact already exists without `--force`.
pub fn build(
    project: &Project,
    bootstrap: &Bootstrap,
    options: &BuildOptions,
) -> Result<BuildResult, String> {
    if std::env::consts::OS != "linux" {
        return Err("Pam Desktop 0.4 packaging currently targets Linux only".to_owned());
    }
    if !cfg!(feature = "servo-engine") {
        return Err(
            "this pam-desktop binary was built without Servo and cannot produce a distributable application"
                .to_owned(),
        );
    }
    let host = std::env::current_exe()
        .map_err(|error| format!("cannot resolve the pam-desktop executable: {error}"))?;
    let pam = std::env::var_os("PAM_BINARY")
        .ok_or_else(|| {
            "PAM_BINARY is missing; invoke builds through `pam desktop build`".to_owned()
        })
        .and_then(|value| resolve_executable(&value))?;
    build_with_binaries(project, bootstrap, options, &host, &pam)
}

fn build_with_binaries(
    project: &Project,
    bootstrap: &Bootstrap,
    options: &BuildOptions,
    host_binary: &Path,
    pam_binary: &Path,
) -> Result<BuildResult, String> {
    bootstrap.validate()?;
    let icon = project.resolve_icon(&bootstrap.manifest.icon)?;
    let host_binary = validated_binary(host_binary, "Pam Desktop host")?;
    let pam_binary = validated_binary(pam_binary, "Pam worker")?;
    let output = output_directory(project, options)?;
    let names = ArtifactNames::new(&bootstrap.manifest)?;
    let destinations = ArtifactDestinations::new(&output, &names, &options.formats);
    destinations.ensure_available(options.force)?;

    let workspace = BuildWorkspace::create(&output)?;
    let bundle = workspace.path.join(&names.bundle);
    materialize_bundle(
        project,
        bootstrap,
        &bundle,
        &output,
        &host_binary,
        &pam_binary,
        &icon,
    )?;

    let source_date_epoch = source_date_epoch()?;
    let mut staged = Vec::new();
    if options.formats.contains(&PackageFormat::Portable) {
        let archive = workspace.path.join(&names.portable);
        create_portable_archive(&workspace.path, &names.bundle, &archive, source_date_epoch)?;
        staged.push((archive, destinations.portable.clone()));
    }
    if options.formats.contains(&PackageFormat::Debian) {
        let debian = workspace.path.join(&names.debian);
        create_debian_package(
            &workspace.path,
            &bundle,
            bootstrap,
            &names,
            &icon,
            &debian,
            source_date_epoch,
        )?;
        staged.push((debian, destinations.debian.clone()));
    }
    if options.formats.contains(&PackageFormat::Directory) {
        staged.push((bundle, destinations.directory.clone()));
    }

    destinations.remove_existing(options.force)?;
    let mut artifacts = Vec::with_capacity(staged.len());
    for (source, destination) in staged {
        let destination =
            destination.ok_or_else(|| "internal package destination is missing".to_owned())?;
        fs::rename(&source, &destination).map_err(|error| {
            format!(
                "cannot publish artifact {} as {}: {error}",
                source.display(),
                destination.display()
            )
        })?;
        artifacts.push(destination);
    }
    Ok(BuildResult { artifacts })
}

struct ArtifactNames {
    bundle: String,
    portable: String,
    debian: String,
    executable: String,
    architecture: String,
}

impl ArtifactNames {
    fn new(manifest: &ApplicationManifest) -> Result<Self, String> {
        let executable = manifest
            .identifier
            .rsplit('.')
            .next()
            .ok_or_else(|| "application identifier has no executable segment".to_owned())?
            .to_owned();
        let architecture = std::env::consts::ARCH.to_owned();
        let bundle = format!("{}-{}-linux-{architecture}", executable, manifest.version);
        let debian_architecture = debian_architecture(&architecture)?;
        Ok(Self {
            portable: format!("{bundle}.tar.gz"),
            debian: format!(
                "{}_{}_{}.deb",
                executable, manifest.version, debian_architecture
            ),
            bundle,
            executable,
            architecture,
        })
    }
}

struct ArtifactDestinations {
    directory: Option<PathBuf>,
    portable: Option<PathBuf>,
    debian: Option<PathBuf>,
}

impl ArtifactDestinations {
    fn new(output: &Path, names: &ArtifactNames, formats: &BTreeSet<PackageFormat>) -> Self {
        Self {
            directory: formats
                .contains(&PackageFormat::Directory)
                .then(|| output.join(&names.bundle)),
            portable: formats
                .contains(&PackageFormat::Portable)
                .then(|| output.join(&names.portable)),
            debian: formats
                .contains(&PackageFormat::Debian)
                .then(|| output.join(&names.debian)),
        }
    }

    fn paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.directory
            .iter()
            .chain(self.portable.iter())
            .chain(self.debian.iter())
    }

    fn ensure_available(&self, force: bool) -> Result<(), String> {
        if force {
            return Ok(());
        }
        if let Some(existing) = self.paths().find(|path| path.exists()) {
            return Err(format!(
                "artifact {} already exists; pass --force to replace this exact build",
                existing.display()
            ));
        }
        Ok(())
    }

    fn remove_existing(&self, force: bool) -> Result<(), String> {
        if !force {
            return Ok(());
        }
        for path in self.paths().filter(|path| path.exists()) {
            let metadata = fs::symlink_metadata(path)
                .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                fs::remove_dir_all(path)
                    .map_err(|error| format!("cannot replace {}: {error}", path.display()))?;
            } else {
                fs::remove_file(path)
                    .map_err(|error| format!("cannot replace {}: {error}", path.display()))?;
            }
        }
        Ok(())
    }
}

struct BuildWorkspace {
    path: PathBuf,
}

impl BuildWorkspace {
    fn create(output: &Path) -> Result<Self, String> {
        let mut random = [0_u8; 8];
        getrandom::fill(&mut random)
            .map_err(|error| format!("cannot create package workspace identity: {error}"))?;
        let path = output.join(format!(
            ".pam-desktop-build-{}-{}",
            std::process::id(),
            hex(&random)
        ));
        fs::create_dir(&path).map_err(|error| {
            format!("cannot create build workspace {}: {error}", path.display())
        })?;
        Ok(Self { path })
    }
}

impl Drop for BuildWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "bundle materialization owns one explicit, auditable set of build inputs"
)]
fn materialize_bundle(
    project: &Project,
    bootstrap: &Bootstrap,
    bundle: &Path,
    output: &Path,
    host_binary: &Path,
    pam_binary: &Path,
    icon: &Path,
) -> Result<(), String> {
    let app = bundle.join("app");
    let bin = bundle.join("bin");
    let lib = bundle.join("lib");
    let etc = bundle.join("etc");
    let applications = bundle.join("share/applications");
    fs::create_dir_all(&app).map_err(|error| error.to_string())?;
    fs::create_dir_all(&bin).map_err(|error| error.to_string())?;
    fs::create_dir_all(&lib).map_err(|error| error.to_string())?;
    fs::create_dir_all(&etc).map_err(|error| error.to_string())?;
    fs::create_dir_all(&applications).map_err(|error| error.to_string())?;

    let context = CopyContext {
        project_root: project.root(),
        build_output: output,
        excludes: &bootstrap.manifest.bundle_excludes,
    };
    copy_project(&context, project.root(), &app, Path::new(""))?;
    sanitize_composer_metadata(&app)?;
    copy_binary(host_binary, &bin.join("pam-desktop"))?;
    copy_binary(pam_binary, &bin.join("pam"))?;
    copy_runtime_libraries([host_binary, pam_binary], &lib)?;
    write_launcher(&bin.join(executable_name(&bootstrap.manifest)?))?;
    write_php_ini(&etc.join("php.ini"))?;

    let icon_destination = bundle_icon_path(bundle, &bootstrap.manifest, icon)?;
    if let Some(parent) = icon_destination.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::copy(icon, &icon_destination).map_err(|error| {
        format!(
            "cannot copy icon {} to {}: {error}",
            icon.display(),
            icon_destination.display()
        )
    })?;
    let desktop_template = desktop_entry(&bootstrap.manifest, "@PAM_EXEC@");
    fs::write(
        applications.join(format!("{}.desktop.in", bootstrap.manifest.identifier)),
        desktop_template,
    )
    .map_err(|error| format!("cannot write desktop entry template: {error}"))?;
    write_portable_installer(bundle, &bootstrap.manifest, icon)?;
    write_portable_uninstaller(bundle, &bootstrap.manifest, icon)?;
    write_bundle_manifest(bundle, bootstrap, pam_binary)?;
    normalize_permissions(bundle)?;
    Ok(())
}

struct CopyContext<'a> {
    project_root: &'a Path,
    build_output: &'a Path,
    excludes: &'a [String],
}

fn copy_project(
    context: &CopyContext<'_>,
    source: &Path,
    destination: &Path,
    relative: &Path,
) -> Result<(), String> {
    let mut active = HashSet::new();
    copy_directory(context, source, destination, relative, &mut active)
}

fn copy_directory(
    context: &CopyContext<'_>,
    source: &Path,
    destination: &Path,
    relative: &Path,
    active: &mut HashSet<PathBuf>,
) -> Result<(), String> {
    let canonical = source
        .canonicalize()
        .map_err(|error| format!("cannot resolve {}: {error}", source.display()))?;
    if canonical.starts_with(context.build_output) {
        return Ok(());
    }
    if !active.insert(canonical.clone()) {
        return Err(format!(
            "bundle symlink cycle detected at {}",
            source.display()
        ));
    }

    let result = (|| {
        for entry in fs::read_dir(source)
            .map_err(|error| format!("cannot read {}: {error}", source.display()))?
        {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let child_relative = relative.join(entry.file_name());
            if excluded_path(&child_relative, context.excludes) {
                continue;
            }
            let target = destination.join(entry.file_name());
            let file_type = entry
                .file_type()
                .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
            if file_type.is_dir() {
                fs::create_dir(&target)
                    .map_err(|error| format!("cannot create {}: {error}", target.display()))?;
                copy_directory(context, &path, &target, &child_relative, active)?;
            } else if file_type.is_file() {
                fs::copy(&path, &target)
                    .map_err(|error| format!("cannot copy {}: {error}", path.display()))?;
            } else if file_type.is_symlink() {
                copy_project_symlink(context, &path, &target, &child_relative, active)?;
            } else {
                return Err(format!("unsupported project file type: {}", path.display()));
            }
        }
        Ok(())
    })();
    active.remove(&canonical);
    result
}

fn copy_project_symlink(
    context: &CopyContext<'_>,
    source: &Path,
    destination: &Path,
    relative: &Path,
    active: &mut HashSet<PathBuf>,
) -> Result<(), String> {
    let canonical = source
        .canonicalize()
        .map_err(|error| format!("cannot resolve symlink {}: {error}", source.display()))?;
    if !canonical.starts_with(context.project_root) && !relative.starts_with("vendor") {
        return Err(format!(
            "bundle symlink escapes the project outside vendor: {}",
            source.display()
        ));
    }
    if canonical.starts_with(context.build_output) {
        return Err(format!(
            "bundle symlink points into the build output: {}",
            source.display()
        ));
    }
    if canonical.is_dir() {
        fs::create_dir(destination)
            .map_err(|error| format!("cannot create {}: {error}", destination.display()))?;
        copy_directory(context, &canonical, destination, relative, active)
    } else if canonical.is_file() {
        fs::copy(&canonical, destination)
            .map(|_| ())
            .map_err(|error| format!("cannot materialize {}: {error}", source.display()))
    } else {
        Err(format!("unsupported project symlink: {}", source.display()))
    }
}

fn sanitize_composer_metadata(app: &Path) -> Result<(), String> {
    let manifest_path = app.join("composer.json");
    let mut manifest = read_json_object(&manifest_path)?;
    if let Some(repositories) = manifest
        .get_mut("repositories")
        .and_then(serde_json::Value::as_array_mut)
    {
        repositories.retain(|repository| {
            repository.get("type").and_then(serde_json::Value::as_str) != Some("path")
        });
        if repositories.is_empty() {
            manifest.remove("repositories");
        }
    }
    write_json_object(&manifest_path, &manifest)?;

    let lock_path = app.join("composer.lock");
    if !lock_path.is_file() {
        return Ok(());
    }
    let mut lock = read_json_object(&lock_path)?;
    for key in ["packages", "packages-dev"] {
        let Some(packages) = lock.get_mut(key).and_then(serde_json::Value::as_array_mut) else {
            continue;
        };
        for package in packages {
            let path_distribution = package
                .get("dist")
                .and_then(|dist| dist.get("type"))
                .and_then(serde_json::Value::as_str)
                == Some("path");
            if path_distribution && let Some(package) = package.as_object_mut() {
                package.remove("dist");
                package.remove("transport-options");
            }
        }
    }
    write_json_object(&lock_path, &lock)
}

fn read_json_object(path: &Path) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    let source = fs::read(path)
        .map_err(|error| format!("cannot read Composer metadata {}: {error}", path.display()))?;
    serde_json::from_slice::<serde_json::Value>(&source)
        .map_err(|error| format!("invalid Composer metadata {}: {error}", path.display()))?
        .as_object()
        .cloned()
        .ok_or_else(|| format!("Composer metadata must be an object: {}", path.display()))
}

fn write_json_object(
    path: &Path,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    let mut encoded = serde_json::to_vec_pretty(object)
        .map_err(|error| format!("cannot serialize {}: {error}", path.display()))?;
    encoded.push(b'\n');
    fs::write(path, encoded).map_err(|error| format!("cannot sanitize {}: {error}", path.display()))
}

fn excluded_path(relative: &Path, configured: &[String]) -> bool {
    let nested_package_vendor = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name),
            _ => None,
        })
        .skip(1)
        .any(|name| name == OsStr::new("vendor"))
        && relative.starts_with("vendor");
    let default = relative.components().any(|component| {
        let Component::Normal(name) = component else {
            return false;
        };
        let name = name.to_string_lossy();
        matches!(
            name.as_ref(),
            ".git" | ".pam" | ".pamignore" | "dist" | "node_modules" | "target"
        ) || name == ".env"
            || name.starts_with(".env.")
    });
    default
        || nested_package_vendor
        || configured
            .iter()
            .map(Path::new)
            .any(|excluded| relative == excluded || relative.starts_with(excluded))
}

fn copy_binary(source: &Path, destination: &Path) -> Result<(), String> {
    fs::copy(source, destination).map_err(|error| {
        format!(
            "cannot copy runtime binary {} to {}: {error}",
            source.display(),
            destination.display()
        )
    })?;
    make_executable(destination)
}

fn copy_runtime_libraries<'a>(
    binaries: impl IntoIterator<Item = &'a Path>,
    destination: &Path,
) -> Result<(), String> {
    let mut libraries = BTreeMap::<OsString, PathBuf>::new();
    for binary in binaries {
        for library in runtime_libraries(binary)? {
            let name = library
                .file_name()
                .ok_or_else(|| format!("runtime library has no filename: {}", library.display()))?
                .to_os_string();
            if let Some(existing) = libraries.insert(name.clone(), library.clone())
                && existing != library
            {
                return Err(format!(
                    "runtime library name conflict for {}: {} and {}",
                    name.to_string_lossy(),
                    existing.display(),
                    library.display()
                ));
            }
        }
    }
    for (name, source) in libraries {
        fs::copy(&source, destination.join(&name))
            .map_err(|error| format!("cannot bundle {}: {error}", source.display()))?;
    }
    Ok(())
}

fn runtime_libraries(binary: &Path) -> Result<Vec<PathBuf>, String> {
    let output = Command::new("ldd")
        .arg(binary)
        .output()
        .map_err(|error| format!("cannot inspect {} with ldd: {error}", binary.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let diagnostics = format!("{stdout}\n{stderr}");
    if !output.status.success()
        && (diagnostics.contains("not a dynamic executable")
            || diagnostics.contains("statically linked"))
    {
        return Ok(Vec::new());
    }
    if !output.status.success() {
        return Err(format!(
            "ldd failed for {}: {diagnostics}",
            binary.display()
        ));
    }

    let mut result = Vec::new();
    for line in stdout.lines() {
        if line.contains("not found") {
            return Err(format!(
                "a runtime dependency is missing for {}: {}",
                binary.display(),
                line.trim()
            ));
        }
        let candidate = if let Some((_, resolved)) = line.split_once("=>") {
            resolved.split_whitespace().next()
        } else {
            line.split_whitespace().find(|value| value.starts_with('/'))
        };
        let Some(path) = candidate.map(PathBuf::from) else {
            continue;
        };
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if path.is_file() && !system_abi_library(name) {
            result.push(path);
        }
    }
    Ok(result)
}

fn system_abi_library(name: &str) -> bool {
    [
        "ld-linux",
        "libc.so",
        "libdl.so",
        "libm.so",
        "libpthread.so",
        "libresolv.so",
        "librt.so",
        "libutil.so",
        "libnss_",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}

fn write_launcher(path: &Path) -> Result<(), String> {
    fs::write(
        path,
        r#"#!/bin/sh
set -eu
PAM_BUNDLE=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
export PAM_BINARY="$PAM_BUNDLE/bin/pam"
export PAM_DESKTOP_BUNDLE=1
export PHPRC="$PAM_BUNDLE/etc/php.ini"
export PHP_INI_SCAN_DIR=
export LD_LIBRARY_PATH="$PAM_BUNDLE/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
cd "$PAM_BUNDLE/app"
exec "$PAM_BUNDLE/bin/pam-desktop" run "$PAM_BUNDLE/app"
"#,
    )
    .map_err(|error| format!("cannot write bundle launcher {}: {error}", path.display()))?;
    make_executable(path)
}

fn write_php_ini(path: &Path) -> Result<(), String> {
    fs::write(
        path,
        "expose_php=Off\ndisplay_errors=Off\nlog_errors=On\nmemory_limit=512M\n",
    )
    .map_err(|error| format!("cannot write bundled php.ini: {error}"))
}

fn bundle_icon_path(
    bundle: &Path,
    manifest: &ApplicationManifest,
    source: &Path,
) -> Result<PathBuf, String> {
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "application icon has no extension".to_owned())?
        .to_ascii_lowercase();
    let theme = if extension == "svg" {
        "scalable"
    } else {
        "512x512"
    };
    Ok(bundle.join(format!(
        "share/icons/hicolor/{theme}/apps/{}.{}",
        manifest.identifier, extension
    )))
}

fn desktop_entry(manifest: &ApplicationManifest, executable: &str) -> String {
    let category = freedesktop_category(manifest.category);
    format!(
        "[Desktop Entry]\nType=Application\nVersion=1.5\nName={}\nComment={}\nExec={executable}\nIcon={}\nTerminal=false\nCategories={category};\nStartupWMClass={}\n",
        desktop_value(&manifest.name),
        desktop_value(&manifest.description),
        manifest.identifier,
        manifest.identifier,
    )
}

fn desktop_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace(';', "\\;")
}

const fn freedesktop_category(category: ApplicationCategory) -> &'static str {
    match category {
        ApplicationCategory::Development => "Development",
        ApplicationCategory::Productivity => "Office",
        ApplicationCategory::Graphics => "Graphics",
        ApplicationCategory::AudioVideo => "AudioVideo",
        ApplicationCategory::Network => "Network",
        ApplicationCategory::Utility => "Utility",
        ApplicationCategory::Game => "Game",
        ApplicationCategory::Education => "Education",
    }
}

fn write_portable_installer(
    bundle: &Path,
    manifest: &ApplicationManifest,
    icon: &Path,
) -> Result<(), String> {
    let executable = executable_name(manifest)?;
    let extension = icon
        .extension()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "application icon has no extension".to_owned())?
        .to_ascii_lowercase();
    let theme = if extension == "svg" {
        "scalable"
    } else {
        "512x512"
    };
    let script = PORTABLE_INSTALLER
        .replace("@APP_ID@", &manifest.identifier)
        .replace("@EXECUTABLE@", executable)
        .replace("@ICON_THEME@", theme)
        .replace("@ICON_EXTENSION@", &extension);
    let path = bundle.join("install.sh");
    fs::write(&path, script)
        .map_err(|error| format!("cannot write portable installer: {error}"))?;
    make_executable(&path)
}

fn write_portable_uninstaller(
    bundle: &Path,
    manifest: &ApplicationManifest,
    icon: &Path,
) -> Result<(), String> {
    let executable = executable_name(manifest)?;
    let extension = icon
        .extension()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "application icon has no extension".to_owned())?
        .to_ascii_lowercase();
    let theme = if extension == "svg" {
        "scalable"
    } else {
        "512x512"
    };
    let script = PORTABLE_UNINSTALLER
        .replace("@APP_ID@", &manifest.identifier)
        .replace("@EXECUTABLE@", executable)
        .replace("@ICON_THEME@", theme)
        .replace("@ICON_EXTENSION@", &extension);
    let path = bundle.join("uninstall.sh");
    fs::write(&path, script)
        .map_err(|error| format!("cannot write portable uninstaller: {error}"))?;
    make_executable(&path)
}

const PORTABLE_INSTALLER: &str = r#"#!/bin/sh
set -eu
BUNDLE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
DATA_HOME=${XDG_DATA_HOME:-"$HOME/.local/share"}
BIN_HOME=${XDG_BIN_HOME:-"$HOME/.local/bin"}
TARGET="$DATA_HOME/pam-desktop/apps/@APP_ID@"
TEMPORARY="${TARGET}.install-$$"
BACKUP="${TARGET}.previous-$$"
mkdir -p "$(dirname -- "$TARGET")" "$BIN_HOME" "$DATA_HOME/applications" "$DATA_HOME/icons/hicolor/@ICON_THEME@/apps"
rm -rf "$TEMPORARY" "$BACKUP"
cp -a "$BUNDLE" "$TEMPORARY"
if [ -e "$TARGET" ]; then mv "$TARGET" "$BACKUP"; fi
if ! mv "$TEMPORARY" "$TARGET"; then
    if [ -e "$BACKUP" ]; then mv "$BACKUP" "$TARGET"; fi
    exit 1
fi
rm -rf "$BACKUP"
ln -sfn "$TARGET/bin/@EXECUTABLE@" "$BIN_HOME/@EXECUTABLE@"
while IFS= read -r line; do
    if [ "$line" = "Exec=@PAM_EXEC@" ]; then
        printf 'Exec=%s\n' "$TARGET/bin/@EXECUTABLE@"
    else
        printf '%s\n' "$line"
    fi
done < "$TARGET/share/applications/@APP_ID@.desktop.in" \
    > "$DATA_HOME/applications/@APP_ID@.desktop"
cp "$TARGET/share/icons/hicolor/@ICON_THEME@/apps/@APP_ID@.@ICON_EXTENSION@" \
    "$DATA_HOME/icons/hicolor/@ICON_THEME@/apps/@APP_ID@.@ICON_EXTENSION@"
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$DATA_HOME/applications" >/dev/null 2>&1 || true
fi
printf 'Installed @APP_ID@. Run: %s\n' "$BIN_HOME/@EXECUTABLE@"
"#;

const PORTABLE_UNINSTALLER: &str = r#"#!/bin/sh
set -eu
DATA_HOME=${XDG_DATA_HOME:-"$HOME/.local/share"}
BIN_HOME=${XDG_BIN_HOME:-"$HOME/.local/bin"}
TARGET="$DATA_HOME/pam-desktop/apps/@APP_ID@"
LINK="$BIN_HOME/@EXECUTABLE@"
if [ -L "$LINK" ] && [ "$(readlink "$LINK")" = "$TARGET/bin/@EXECUTABLE@" ]; then
    rm -f "$LINK"
fi
rm -f "$DATA_HOME/applications/@APP_ID@.desktop"
rm -f "$DATA_HOME/icons/hicolor/@ICON_THEME@/apps/@APP_ID@.@ICON_EXTENSION@"
rm -rf "$TARGET"
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$DATA_HOME/applications" >/dev/null 2>&1 || true
fi
printf 'Removed @APP_ID@.\n'
"#;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BundleManifest<'a> {
    schema_version: u8,
    protocol_version: u16,
    application: &'a ApplicationManifest,
    runtime: RuntimeManifest,
    target: TargetManifest,
    source_date_epoch: u64,
    files: Vec<BundleFile>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeManifest {
    pam_desktop: String,
    pam: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TargetManifest {
    operating_system: &'static str,
    architecture: &'static str,
    abi: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BundleFile {
    path: String,
    bytes: u64,
    sha256: String,
}

fn write_bundle_manifest(
    bundle: &Path,
    bootstrap: &Bootstrap,
    pam_binary: &Path,
) -> Result<(), String> {
    let manifest = BundleManifest {
        schema_version: 1,
        protocol_version: PROTOCOL_VERSION,
        application: &bootstrap.manifest,
        runtime: RuntimeManifest {
            pam_desktop: env!("CARGO_PKG_VERSION").to_owned(),
            pam: binary_version(pam_binary).unwrap_or_else(|_| "unknown".to_owned()),
        },
        target: TargetManifest {
            operating_system: std::env::consts::OS,
            architecture: std::env::consts::ARCH,
            abi: "glibc",
        },
        source_date_epoch: source_date_epoch()?,
        files: bundle_files(bundle)?,
    };
    let encoded = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("cannot serialize bundle manifest: {error}"))?;
    fs::write(bundle.join("manifest.json"), encoded)
        .map_err(|error| format!("cannot write bundle manifest: {error}"))
}

fn bundle_files(root: &Path) -> Result<Vec<BundleFile>, String> {
    fn visit(root: &Path, directory: &Path, files: &mut Vec<BundleFile>) -> Result<(), String> {
        for entry in fs::read_dir(directory)
            .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
        {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|error| error.to_string())?;
            if file_type.is_dir() {
                visit(root, &path, files)?;
            } else if file_type.is_file() && path.file_name() != Some(OsStr::new("manifest.json")) {
                let mut file = fs::File::open(&path)
                    .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
                let mut digest = Sha256::new();
                let bytes = std::io::copy(&mut file, &mut DigestWriter(&mut digest))
                    .map_err(|error| format!("cannot hash {}: {error}", path.display()))?;
                files.push(BundleFile {
                    path: portable_relative(root, &path)?,
                    bytes,
                    sha256: format!("{:x}", digest.finalize()),
                });
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    visit(root, root, &mut files)?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

struct DigestWriter<'a>(&'a mut Sha256);

impl std::io::Write for DigestWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.update(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn create_portable_archive(
    workspace: &Path,
    bundle_name: &str,
    output: &Path,
    source_date_epoch: u64,
) -> Result<(), String> {
    let status = Command::new("tar")
        .args([
            "--sort=name",
            &format!("--mtime=@{source_date_epoch}"),
            "--owner=0",
            "--group=0",
            "--numeric-owner",
            "-czf",
        ])
        .arg(output)
        .arg("-C")
        .arg(workspace)
        .arg(bundle_name)
        .status()
        .map_err(|error| format!("cannot start tar: {error}"))?;
    if !status.success() {
        return Err(format!("tar failed with status {status}"));
    }
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "Debian staging keeps every resolved build input explicit"
)]
fn create_debian_package(
    workspace: &Path,
    bundle: &Path,
    bootstrap: &Bootstrap,
    names: &ArtifactNames,
    icon: &Path,
    output: &Path,
    source_date_epoch: u64,
) -> Result<(), String> {
    let root = workspace.join("debian-root");
    let application_root = root.join("opt").join(&bootstrap.manifest.identifier);
    fs::create_dir_all(&application_root).map_err(|error| error.to_string())?;
    for entry in ["app", "bin", "lib", "etc", "manifest.json"] {
        hardlink_or_copy(&bundle.join(entry), &application_root.join(entry))?;
    }

    let applications = root.join("usr/share/applications");
    fs::create_dir_all(&applications).map_err(|error| error.to_string())?;
    fs::write(
        applications.join(format!("{}.desktop", bootstrap.manifest.identifier)),
        desktop_entry(
            &bootstrap.manifest,
            &format!(
                "/opt/{}/bin/{}",
                bootstrap.manifest.identifier, names.executable
            ),
        ),
    )
    .map_err(|error| format!("cannot write Debian desktop entry: {error}"))?;

    let source_icon = bundle_icon_path(bundle, &bootstrap.manifest, icon)?;
    let relative_icon = source_icon
        .strip_prefix(bundle)
        .map_err(|error| error.to_string())?;
    let destination_icon = root.join("usr").join(relative_icon);
    if let Some(parent) = destination_icon.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    hardlink_or_copy(&source_icon, &destination_icon)?;

    let usr_bin = root.join("usr/bin");
    fs::create_dir_all(&usr_bin).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        format!(
            "/opt/{}/bin/{}",
            bootstrap.manifest.identifier, names.executable
        ),
        usr_bin.join(&names.executable),
    )
    .map_err(|error| format!("cannot create Debian launcher link: {error}"))?;

    let control_directory = root.join("DEBIAN");
    fs::create_dir_all(&control_directory).map_err(|error| error.to_string())?;
    let installed_size = directory_bytes(&root)?.div_ceil(1024);
    let section = debian_section(bootstrap.manifest.category);
    let control = format!(
        "Package: {}\nVersion: {}\nSection: {section}\nPriority: optional\nArchitecture: {}\nMaintainer: {}\nInstalled-Size: {installed_size}\nDepends: libc6, libfontconfig1, libfreetype6, libgl1, libx11-6, libxcb1\nDescription: {}\n {}\n",
        bootstrap.manifest.identifier,
        bootstrap.manifest.version,
        debian_architecture(&names.architecture)?,
        bootstrap.manifest.publisher,
        bootstrap.manifest.name,
        if bootstrap.manifest.description.is_empty() {
            bootstrap.manifest.name.as_str()
        } else {
            bootstrap.manifest.description.as_str()
        },
    );
    fs::write(control_directory.join("control"), control)
        .map_err(|error| format!("cannot write Debian control metadata: {error}"))?;
    normalize_permissions(&root)?;

    let status = Command::new("dpkg-deb")
        .args(["--root-owner-group", "--build"])
        .arg(&root)
        .arg(output)
        .env("SOURCE_DATE_EPOCH", source_date_epoch.to_string())
        .status()
        .map_err(|error| format!("cannot start dpkg-deb: {error}"))?;
    if !status.success() {
        return Err(format!("dpkg-deb failed with status {status}"));
    }
    Ok(())
}

fn hardlink_or_copy(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("cannot inspect {}: {error}", source.display()))?;
    if metadata.is_dir() {
        fs::create_dir_all(destination)
            .map_err(|error| format!("cannot create {}: {error}", destination.display()))?;
        for entry in fs::read_dir(source)
            .map_err(|error| format!("cannot read {}: {error}", source.display()))?
        {
            let entry = entry.map_err(|error| error.to_string())?;
            hardlink_or_copy(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else if metadata.file_type().is_symlink() {
        return Err(format!(
            "unexpected symlink in materialized bundle: {}",
            source.display()
        ));
    } else if fs::hard_link(source, destination).is_err() {
        fs::copy(source, destination)
            .map_err(|error| format!("cannot copy {}: {error}", source.display()))?;
    }
    Ok(())
}

fn directory_bytes(path: &Path) -> Result<u64, String> {
    let mut bytes = 0_u64;
    for entry in
        fs::read_dir(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?
    {
        let entry = entry.map_err(|error| error.to_string())?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
        if metadata.is_dir() {
            bytes = bytes.saturating_add(directory_bytes(&entry.path())?);
        } else if metadata.is_file() {
            bytes = bytes.saturating_add(metadata.len());
        }
    }
    Ok(bytes)
}

fn normalize_permissions(root: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fn visit(root: &Path, path: &Path) -> Result<(), String> {
            let metadata = fs::symlink_metadata(path)
                .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
            if metadata.file_type().is_symlink() {
                return Ok(());
            }
            if metadata.is_dir() {
                fs::set_permissions(path, fs::Permissions::from_mode(0o755))
                    .map_err(|error| format!("cannot secure {}: {error}", path.display()))?;
                for entry in fs::read_dir(path)
                    .map_err(|error| format!("cannot read {}: {error}", path.display()))?
                {
                    visit(root, &entry.map_err(|error| error.to_string())?.path())?;
                }
            } else if metadata.is_file() {
                let relative = path.strip_prefix(root).map_err(|error| error.to_string())?;
                let executable = relative.components().any(|component| {
                    matches!(component, Component::Normal(name) if name == OsStr::new("bin"))
                }) || path.extension() == Some(OsStr::new("sh"));
                let mode = if executable { 0o755 } else { 0o644 };
                fs::set_permissions(path, fs::Permissions::from_mode(mode))
                    .map_err(|error| format!("cannot secure {}: {error}", path.display()))?;
            }
            Ok(())
        }

        visit(root, root)?;
    }
    Ok(())
}

const fn debian_section(category: ApplicationCategory) -> &'static str {
    match category {
        ApplicationCategory::Development => "devel",
        ApplicationCategory::Productivity => "office",
        ApplicationCategory::Graphics => "graphics",
        ApplicationCategory::AudioVideo => "sound",
        ApplicationCategory::Network => "net",
        ApplicationCategory::Utility => "utils",
        ApplicationCategory::Game => "games",
        ApplicationCategory::Education => "education",
    }
}

fn debian_architecture(architecture: &str) -> Result<&'static str, String> {
    match architecture {
        "x86_64" => Ok("amd64"),
        "aarch64" => Ok("arm64"),
        _ => Err(format!(
            "Debian packaging does not yet map architecture {architecture:?}"
        )),
    }
}

fn source_date_epoch() -> Result<u64, String> {
    Ok(configured_source_date_epoch()?.unwrap_or(DEFAULT_SOURCE_DATE_EPOCH))
}

fn configured_source_date_epoch() -> Result<Option<u64>, String> {
    std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .map(|value| {
            value
                .parse()
                .map_err(|_| "SOURCE_DATE_EPOCH must be an unsigned integer".to_owned())
        })
        .transpose()
}

fn output_directory(project: &Project, options: &BuildOptions) -> Result<PathBuf, String> {
    let path = options
        .output
        .clone()
        .unwrap_or_else(|| project.root().join("dist"));
    fs::create_dir_all(&path)
        .map_err(|error| format!("cannot create output directory {}: {error}", path.display()))?;
    path.canonicalize().map_err(|error| {
        format!(
            "cannot resolve output directory {}: {error}",
            path.display()
        )
    })
}

fn validated_binary(path: &Path, label: &str) -> Result<PathBuf, String> {
    let resolved = path
        .canonicalize()
        .map_err(|error| format!("cannot resolve {label} {}: {error}", path.display()))?;
    if !resolved.is_file() {
        return Err(format!("{label} is not a file: {}", resolved.display()));
    }
    Ok(resolved)
}

fn resolve_executable(value: &OsStr) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if path.is_absolute() || path.components().count() > 1 {
        return validated_binary(&path, "Pam worker");
    }
    let search = std::env::var_os("PATH")
        .ok_or_else(|| "PATH is missing while resolving the Pam worker".to_owned())?;
    std::env::split_paths(&search)
        .map(|directory| directory.join(&path))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| format!("cannot find {} in PATH", path.display()))
}

fn binary_version(binary: &Path) -> Result<String, String> {
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .map_err(|error| format!("cannot read {} version: {error}", binary.display()))?;
    if !output.status.success() {
        return Err(format!("{} --version failed", binary.display()));
    }
    String::from_utf8(output.stdout)
        .map_err(|_| "Pam version output is not UTF-8".to_owned())?
        .split_whitespace()
        .last()
        .map(str::to_owned)
        .ok_or_else(|| "Pam version output is empty".to_owned())
}

fn executable_name(manifest: &ApplicationManifest) -> Result<&str, String> {
    manifest
        .identifier
        .rsplit('.')
        .next()
        .ok_or_else(|| "application identifier has no executable segment".to_owned())
}

fn portable_relative(root: &Path, path: &Path) -> Result<String, String> {
    path.strip_prefix(root)
        .map_err(|error| error.to_string())?
        .to_str()
        .ok_or_else(|| format!("bundle path is not valid UTF-8: {}", path.display()))
        .map(|value| value.replace('\\', "/"))
}

fn make_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("cannot mark {} executable: {error}", path.display()))?;
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}

/// Prints the stable Linux build command surface.
pub fn print_build_usage(executable: &OsStr) {
    println!(
        "Usage: {} build [directory] [--output directory] [--format directory|portable|deb|all] [--force]\n\nDefault formats: directory and portable.\n`deb` requires dpkg-deb; portable bundles include install.sh and uninstall.sh.",
        executable.to_string_lossy()
    );
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use pam_desktop_protocol::{
        ApplicationCategory, ApplicationManifest, Bootstrap, NativeCapabilities, WindowConfig,
        WindowTheme,
    };

    use super::*;

    #[test]
    fn parses_build_formats_and_replacement_policy() {
        let BuildCommand::Build(options) = BuildOptions::parse([
            OsString::from("project"),
            OsString::from("--format"),
            OsString::from("deb"),
            OsString::from("--format"),
            OsString::from("portable"),
            OsString::from("--force"),
        ])
        .expect("options should parse") else {
            panic!("expected a build action");
        };
        assert_eq!(options.project, PathBuf::from("project"));
        assert!(options.formats.contains(&PackageFormat::Debian));
        assert!(options.formats.contains(&PackageFormat::Portable));
        assert!(!options.formats.contains(&PackageFormat::Directory));
        assert!(options.force);
    }

    #[test]
    fn builds_a_materialized_directory_with_integrity_metadata() {
        let fixture = Fixture::create();
        let project = Project::discover(&fixture.project).expect("project should be valid");
        let options = BuildOptions {
            project: fixture.project.clone(),
            output: Some(fixture.output.clone()),
            formats: BTreeSet::from([PackageFormat::Directory]),
            force: false,
        };
        let result = build_with_binaries(
            &project,
            &fixture.bootstrap(),
            &options,
            &fixture.host,
            &fixture.pam,
        )
        .expect("bundle should build");

        let bundle = &result.artifacts[0];
        assert!(bundle.join("bin/pam-desktop").is_file());
        assert!(bundle.join("bin/pam").is_file());
        assert!(bundle.join("bin/hello").is_file());
        assert!(bundle.join("app/vendor/autoload.php").is_file());
        assert!(!bundle.join("app/vendor/pam/desktop/vendor").exists());
        assert!(
            !fs::read_to_string(bundle.join("app/composer.json"))
                .expect("bundled Composer manifest should be readable")
                .contains("/private/source")
        );
        assert!(
            !fs::read_to_string(bundle.join("app/composer.lock"))
                .expect("bundled Composer lock should be readable")
                .contains("/private/source")
        );
        assert!(!bundle.join("app/.env").exists());
        assert!(!bundle.join("app/storage/cache").exists());
        assert!(bundle.join("manifest.json").is_file());
        let manifest: serde_json::Value = serde_json::from_slice(
            &fs::read(bundle.join("manifest.json")).expect("manifest should be readable"),
        )
        .expect("manifest should be JSON");
        assert_eq!(manifest["schemaVersion"], 1);
        assert_eq!(manifest["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(manifest["sourceDateEpoch"], 0);
        assert_eq!(manifest["application"]["category"], 1);
        assert!(
            manifest["files"]
                .as_array()
                .is_some_and(|files| !files.is_empty())
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(bundle.join("bin/hello"))
                    .expect("launcher metadata should exist")
                    .permissions()
                    .mode()
                    & 0o777,
                0o755
            );
            assert_eq!(
                fs::metadata(bundle.join("app/app.php"))
                    .expect("application metadata should exist")
                    .permissions()
                    .mode()
                    & 0o777,
                0o644
            );
        }
    }

    struct Fixture {
        root: PathBuf,
        project: PathBuf,
        output: PathBuf,
        host: PathBuf,
        pam: PathBuf,
    }

    impl Fixture {
        fn create() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be valid")
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "pam-desktop-package-test-{}-{unique}",
                std::process::id()
            ));
            let project = root.join("project");
            let output = root.join("output");
            fs::create_dir_all(project.join("resources")).expect("resources should be created");
            fs::create_dir_all(project.join("vendor")).expect("vendor should be created");
            fs::create_dir_all(project.join("vendor/pam/desktop/vendor"))
                .expect("nested package vendor should be created");
            fs::create_dir_all(project.join("storage/cache")).expect("cache should be created");
            fs::create_dir(&output).expect("output should be created");
            fs::write(project.join("app.php"), "<?php\n").expect("app should be written");
            fs::write(
                project.join("composer.json"),
                "{\"repositories\":[{\"type\":\"path\",\"url\":\"/private/source\"}]}\n",
            )
            .expect("composer should be written");
            fs::write(
                project.join("composer.lock"),
                "{\"packages\":[{\"name\":\"pam/desktop\",\"dist\":{\"type\":\"path\",\"url\":\"/private/source\"},\"transport-options\":{\"symlink\":true}}],\"packages-dev\":[]}\n",
            )
            .expect("composer lock should be written");
            fs::write(project.join("vendor/autoload.php"), "<?php\n")
                .expect("autoload should be written");
            fs::write(
                project.join("vendor/pam/desktop/vendor/dev-only.php"),
                "<?php\n",
            )
            .expect("nested package dependency should be written");
            fs::write(project.join("resources/index.html"), "<!doctype html>\n")
                .expect("entry should be written");
            fs::write(
                project.join("resources/icon.svg"),
                "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 64 64\"></svg>\n",
            )
            .expect("icon should be written");
            fs::write(project.join("storage/cache/item"), "ignored\n")
                .expect("cache should be written");
            fs::write(project.join(".env"), "SECRET=yes\n").expect("secret should be written");
            let host = root.join("pam-desktop");
            let pam = root.join("pam");
            fs::write(&host, "#!/bin/sh\nexit 0\n").expect("host should be written");
            fs::write(
                &pam,
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'pam 0.1.1'; fi\n",
            )
            .expect("pam should be written");
            make_executable(&host).expect("host should be executable");
            make_executable(&pam).expect("pam should be executable");
            Self {
                root,
                project,
                output,
                host,
                pam,
            }
        }

        fn bootstrap(&self) -> Bootstrap {
            Bootstrap {
                manifest: ApplicationManifest {
                    identifier: "com.pushin.hello".to_owned(),
                    name: "Pam Hello".to_owned(),
                    version: "0.4.0".to_owned(),
                    description: "A package fixture.".to_owned(),
                    publisher: "Pushin".to_owned(),
                    category: ApplicationCategory::Development,
                    icon: "resources/icon.svg".to_owned(),
                    bundle_excludes: vec!["storage/cache".to_owned()],
                },
                windows: vec![WindowConfig {
                    id: "main".to_owned(),
                    entry: "resources/index.html".to_owned(),
                    title: "Pam Hello".to_owned(),
                    width: 800,
                    height: 600,
                    min_width: 320,
                    min_height: 240,
                    resizable: true,
                    visible: true,
                    theme: WindowTheme::System,
                }],
                command_timeout_ms: 30_000,
                capabilities: NativeCapabilities::default(),
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
