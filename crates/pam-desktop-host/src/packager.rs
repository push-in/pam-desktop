//! `pam-desktop build`: a self-contained bundle of an application and the
//! installers of the system the build runs on.
//!
//! Everything is staged in a private workspace inside the output directory
//! and published by renaming only after every requested artifact was built,
//! so a failing tool never leaves a half-made artifact behind.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use pam_desktop_ui::modules::support::hex;
use sha2::{Digest, Sha256};

use crate::bundle::{
    self, BundleApplication, BundleManifest, BundleRuntime, BundleTarget, BundleUpdates, PHP_BIN,
    PHP_EXTENSION_DIR_VARIABLE, PHP_EXTENSIONS, PHP_INI, PHP_LIB, PHP_WRAPPER,
};
use crate::manifest::{self, Application, Category, Manifest};

const DEFAULT_SOURCE_DATE_EPOCH: u64 = 0;
const MAX_ICON_BYTES: u64 = 2 * 1024 * 1024;
/// The worker bootstrap inside the PHP package, and the package inside `vendor`.
const BOOTSTRAP: &str = "packages/desktop/bin/pam-desktop-worker";
const VENDOR_PACKAGE: &str = "vendor/pushinbr/pam-desktop";
/// What an application of this repository's checkout needs from the PHP package.
const CHECKOUT_PACKAGE_FILES: [&str; 6] = [
    "bin/pam-desktop-worker",
    "src",
    "resources",
    "composer.json",
    "LICENSE",
    "LICENSE-APACHE",
];

/// Shipped when the manifest names no icon.
const DEFAULT_ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
  <rect x="16" y="16" width="224" height="224" rx="48" fill="#3b5bdb"/>
  <rect x="56" y="72" width="144" height="112" rx="14" fill="#ffffff"/>
  <rect x="56" y="72" width="144" height="30" rx="14" fill="#dbe4ff"/>
  <circle cx="76" cy="87" r="6" fill="#3b5bdb"/>
  <circle cx="96" cy="87" r="6" fill="#3b5bdb"/>
</svg>
"##;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum PackageFormat {
    Directory,
    Portable,
    Debian,
    AppImage,
    Rpm,
    Dmg,
    Pkg,
    Msix,
    Msi,
}

/// Parsed `pam-desktop build` action.
pub enum BuildCommand {
    Help,
    Build(BuildOptions),
}

/// What to build, where, and whether existing artifacts may be replaced.
#[derive(Debug)]
pub struct BuildOptions {
    pub project: PathBuf,
    pub output: Option<PathBuf>,
    formats: BTreeSet<PackageFormat>,
    force: bool,
    sign: bool,
}

impl BuildOptions {
    /// Parses the arguments of `build`.
    ///
    /// # Errors
    ///
    /// For unknown options, missing values, a second project path or an
    /// unknown package format.
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<BuildCommand, String> {
        let mut arguments = arguments.into_iter();
        let mut project = None;
        let mut output = None;
        let mut formats = BTreeSet::from([PackageFormat::Directory, PackageFormat::Portable]);
        let mut explicit_format = false;
        let mut force = false;
        let mut sign = false;

        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some("--help" | "-h") => return Ok(BuildCommand::Help),
                Some("--output" | "-o") => {
                    let value = arguments.next().ok_or("--output requires a directory")?;
                    if output.replace(PathBuf::from(value)).is_some() {
                        return Err("--output may only be declared once".to_owned());
                    }
                }
                Some("--format") => {
                    let value = arguments.next().ok_or("--format requires a value")?;
                    let value = value.to_str().ok_or("package format must be valid UTF-8")?;
                    if !explicit_format {
                        formats.clear();
                        explicit_format = true;
                    }
                    formats.extend(parse_format(value)?);
                }
                Some("--force") => force = true,
                Some("--sign") => sign = true,
                Some(value) if value.starts_with('-') => {
                    return Err(format!("unknown build option {value:?}"));
                }
                _ => {
                    if project.replace(PathBuf::from(argument)).is_some() {
                        return Err("build accepts only one application".to_owned());
                    }
                }
            }
        }
        Ok(BuildCommand::Build(Self {
            project: project.unwrap_or_else(|| PathBuf::from(".")),
            output,
            formats,
            force,
            sign,
        }))
    }
}

fn parse_format(value: &str) -> Result<Vec<PackageFormat>, String> {
    Ok(match value {
        "directory" => vec![PackageFormat::Directory],
        "portable" | "tar.gz" | "zip" => vec![PackageFormat::Portable],
        "deb" | "debian" => vec![PackageFormat::Debian],
        "appimage" => vec![PackageFormat::AppImage],
        "rpm" => vec![PackageFormat::Rpm],
        "dmg" => vec![PackageFormat::Dmg],
        "pkg" => vec![PackageFormat::Pkg],
        "msix" => vec![PackageFormat::Msix],
        "msi" => vec![PackageFormat::Msi],
        // The main installer of the system the build runs on.
        "native" if cfg!(target_os = "macos") => vec![PackageFormat::Dmg],
        "native" if cfg!(target_os = "windows") => vec![PackageFormat::Msix],
        "native" => vec![PackageFormat::Debian],
        "all" => {
            let mut formats = vec![PackageFormat::Directory, PackageFormat::Portable];
            if cfg!(target_os = "macos") {
                formats.extend([PackageFormat::Dmg, PackageFormat::Pkg]);
            } else if cfg!(target_os = "windows") {
                formats.extend([PackageFormat::Msix, PackageFormat::Msi]);
            } else {
                formats.extend([
                    PackageFormat::Debian,
                    PackageFormat::AppImage,
                    PackageFormat::Rpm,
                ]);
            }
            formats
        }
        _ => {
            return Err(format!(
                "unknown package format {value:?}; expected directory, portable, deb, appimage, rpm, dmg, pkg, msix, msi or all"
            ));
        }
    })
}

/// Paths published by one successful build.
pub struct BuildResult {
    pub artifacts: Vec<PathBuf>,
    /// What the build decided on its own, for the person who ran it.
    pub notes: Vec<String>,
}

/// The binaries a bundle is made of and where packaging tools are looked up.
struct Toolchain {
    host: PathBuf,
    php: PathBuf,
    /// What the host passes to `php` before the worker bootstrap (`exec` for `pam`).
    php_arguments: Vec<OsString>,
    /// The `PATH` packaging tools must be on.
    search_path: OsString,
}

/// Builds the bundle of an application and the requested installers.
///
/// # Errors
///
/// When the system is unsupported, the manifest is invalid, a file cannot be
/// materialized, a packaging tool is missing or fails, or an artifact already
/// exists without `--force`.
pub fn build(options: &BuildOptions) -> Result<BuildResult, String> {
    if !matches!(std::env::consts::OS, "linux" | "windows" | "macos") {
        return Err(format!(
            "PAM Desktop packaging does not support {}",
            std::env::consts::OS
        ));
    }
    let host = std::env::current_exe()
        .map_err(|error| format!("cannot resolve the pam-desktop executable: {error}"))?;
    let (php, php_arguments) = crate::php_command();
    let search_path = std::env::var_os("PATH").unwrap_or_default();
    let php = resolve_executable(&php, &search_path)?;
    build_with(
        options,
        &Toolchain {
            host,
            php,
            php_arguments,
            search_path,
        },
    )
}

fn build_with(options: &BuildOptions, toolchain: &Toolchain) -> Result<BuildResult, String> {
    validate_platform_formats(options)?;
    require_tools(options, &toolchain.search_path)?;
    let project = Project::discover(&options.project)?;
    let application = &project.application;
    let host = validated_binary(&toolchain.host, "PAM Desktop host")?;
    let php = validated_binary(&toolchain.php, "PHP runtime")?;
    let output = output_directory(&project, options)?;
    let names = ArtifactNames::new(application)?;
    let destinations = ArtifactDestinations::new(&output, &names, &options.formats);
    destinations.ensure_available(options.force)?;

    let workspace = BuildWorkspace::create(&output)?;
    let icon = resolve_icon(&project, &workspace.path)?;
    // Named as it was found (`php`), not as the link resolves (`php8.5`).
    let php_name = toolchain
        .php
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or("the PHP runtime name must be valid UTF-8")?;
    let runtime = PhpRuntime::probe(
        &php,
        php_name,
        &toolchain.php_arguments,
        ExtensionChoice::of(application.extensions.as_deref()),
    )?;
    let bundle = workspace.path.join(&names.bundle);
    let context = BundleContext {
        project: &project,
        names: &names,
        icon: &icon,
        host: &host,
        runtime: &runtime,
        output: &output,
        sign: options.sign,
    };
    let Materialized { system, notes } = materialize_bundle(&context, &bundle)?;

    let source_date_epoch = source_date_epoch()?;
    let package = PackageContext {
        workspace: &workspace.path,
        bundle: &bundle,
        application,
        names: &names,
        icon: &icon,
        system: &system,
        sign: options.sign,
    };
    let mut staged = Vec::new();
    for format in &options.formats {
        let (name, destination) = match format {
            PackageFormat::Directory => continue,
            PackageFormat::Portable => (&names.portable, &destinations.portable),
            PackageFormat::Debian => (&names.debian, &destinations.debian),
            PackageFormat::AppImage => (&names.appimage, &destinations.appimage),
            PackageFormat::Rpm => (&names.rpm, &destinations.rpm),
            PackageFormat::Dmg => (&names.dmg, &destinations.dmg),
            PackageFormat::Pkg => (&names.pkg, &destinations.pkg),
            PackageFormat::Msix => (&names.msix, &destinations.msix),
            PackageFormat::Msi => (&names.msi, &destinations.msi),
        };
        let artifact = workspace.path.join(name);
        match format {
            PackageFormat::Directory => {}
            PackageFormat::Portable => {
                create_portable_archive(&package, &artifact, source_date_epoch)?
            }
            PackageFormat::Debian => create_debian_package(&package, &artifact, source_date_epoch)?,
            PackageFormat::AppImage => create_appimage_package(&package, &artifact)?,
            PackageFormat::Rpm => create_rpm_package(&package, &artifact)?,
            PackageFormat::Dmg => create_dmg_package(&package, &artifact)?,
            PackageFormat::Pkg => create_pkg_package(&package, &artifact)?,
            PackageFormat::Msix => create_msix_package(&package, &artifact)?,
            PackageFormat::Msi => create_msi_package(&package, &artifact)?,
        }
        staged.push((artifact, destination.clone()));
    }
    // Last: the installers above hard-link from the staged bundle.
    if options.formats.contains(&PackageFormat::Directory) {
        staged.push((bundle, destinations.directory.clone()));
    }

    destinations.remove_existing(options.force)?;
    let mut artifacts = Vec::with_capacity(staged.len());
    for (source, destination) in staged {
        let destination = destination.ok_or("internal package destination is missing")?;
        fs::rename(&source, &destination).map_err(|error| {
            format!(
                "cannot publish artifact {} as {}: {error}",
                source.display(),
                destination.display()
            )
        })?;
        artifacts.push(destination);
    }
    Ok(BuildResult { artifacts, notes })
}

fn validate_platform_formats(options: &BuildOptions) -> Result<(), String> {
    let requested = |formats: &[PackageFormat]| {
        formats
            .iter()
            .any(|format| options.formats.contains(format))
    };
    if options.formats.is_empty() {
        return Err("at least one package format is required".to_owned());
    }
    if requested(&[
        PackageFormat::Debian,
        PackageFormat::AppImage,
        PackageFormat::Rpm,
    ]) && !cfg!(target_os = "linux")
    {
        return Err("the appimage, deb and rpm formats are only available on Linux".to_owned());
    }
    if requested(&[PackageFormat::Dmg, PackageFormat::Pkg]) && !cfg!(target_os = "macos") {
        return Err("the dmg and pkg formats are only available on macOS".to_owned());
    }
    if requested(&[PackageFormat::Msix, PackageFormat::Msi]) && !cfg!(target_os = "windows") {
        return Err("the msix and msi formats are only available on Windows".to_owned());
    }
    if options.sign && cfg!(target_os = "linux") {
        return Err(
            "--sign is available for macOS codesigning/notarization and Windows Authenticode"
                .to_owned(),
        );
    }
    Ok(())
}

/// Fails before any work when a tool or a signing credential a requested
/// format needs is missing.
fn require_tools(options: &BuildOptions, search_path: &OsStr) -> Result<(), String> {
    let mut tools: Vec<(&str, &str)> = Vec::new();
    if cfg!(target_os = "linux") {
        tools.push(("ldd", "part of the C library tools"));
    }
    for format in &options.formats {
        match format {
            PackageFormat::Directory => {}
            PackageFormat::Portable if cfg!(target_os = "linux") => {
                tools.extend([("tar", "install GNU tar"), ("gzip", "install gzip")])
            }
            PackageFormat::Portable if cfg!(target_os = "macos") => {
                tools.push(("ditto", "part of macOS"))
            }
            PackageFormat::Portable => tools.push(("powershell", "part of Windows")),
            PackageFormat::Debian => tools.push(("dpkg-deb", "install dpkg")),
            PackageFormat::AppImage => tools.push((
                "appimagetool",
                "install the official AppImageKit appimagetool",
            )),
            PackageFormat::Rpm => tools.push(("rpmbuild", "install rpm-build")),
            PackageFormat::Dmg => tools.push(("hdiutil", "part of macOS")),
            PackageFormat::Pkg => tools.push(("pkgbuild", "install the Xcode command-line tools")),
            PackageFormat::Msix => tools.extend([
                ("makeappx", "install the Windows SDK"),
                ("powershell", "part of Windows"),
            ]),
            PackageFormat::Msi => {
                tools.push(("wix", "install WiX Toolset 5 and place wix on PATH"))
            }
        }
    }
    if options.sign && cfg!(target_os = "macos") {
        tools.push(("codesign", "install the Xcode command-line tools"));
        require_variable("PAM_MACOS_SIGNING_IDENTITY", "macOS signing")?;
        if options.formats.contains(&PackageFormat::Pkg) {
            require_variable("PAM_MACOS_INSTALLER_IDENTITY", "signed PKG builds")?;
        }
    }
    if options.sign && cfg!(target_os = "windows") {
        tools.push(("signtool", "install the Windows SDK"));
        windows_certificate_thumbprint()?;
    }
    for (name, hint) in tools {
        if find_tool(name, search_path).is_none() {
            return Err(format!(
                "{name} is required ({hint}) and was not found on PATH"
            ));
        }
    }
    Ok(())
}

fn require_variable(name: &str, purpose: &str) -> Result<String, String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{name} is required for {purpose}"))
}

fn find_tool(name: &str, search_path: &OsStr) -> Option<PathBuf> {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(search_path)
        .map(|directory| directory.join(&file))
        .find(|candidate| candidate.is_file())
}

// ---- the project ----------------------------------------------------------

/// What is copied into `app/` and where the application sits inside it.
struct Project {
    /// The directory of the entry script, canonical.
    root: PathBuf,
    manifest: Manifest,
    application: Application,
    /// The directory `app/` mirrors: the project, or the checkout around it.
    source_root: PathBuf,
    /// The project relative to `source_root`; empty for a project of its own.
    relative: PathBuf,
    /// The paths of `source_root` to copy; `None` copies everything.
    include: Option<Vec<PathBuf>>,
}

impl Project {
    fn discover(target: &Path) -> Result<Self, String> {
        let target = target
            .canonicalize()
            .map_err(|error| format!("cannot resolve {}: {error}", target.display()))?;
        let (root, entry_override) = if target.is_dir() {
            (target, None)
        } else {
            let name = target
                .file_name()
                .and_then(OsStr::to_str)
                .ok_or("the entry script name must be valid UTF-8")?
                .to_owned();
            (
                target.parent().map(Path::to_path_buf).unwrap_or_default(),
                Some(name),
            )
        };
        let manifest = Manifest::read(&root)?;
        let mut application = manifest
            .application()
            .map_err(|error| format!("{}: {error}", root.join(manifest::FILE_NAME).display()))?;
        if let Some(entry) = entry_override {
            application.entry = entry;
        }
        if !root.join(&application.entry).is_file() {
            return Err(format!(
                "the entry script {} does not exist",
                root.join(&application.entry).display()
            ));
        }
        if matches!(application.executable(), "pam-desktop" | PHP_WRAPPER) {
            return Err(format!(
                "applicationId cannot end in {:?}: the bundle uses that file name itself",
                application.executable()
            ));
        }

        if root.join("vendor/autoload.php").is_file() {
            return Ok(Self {
                source_root: root.clone(),
                relative: PathBuf::new(),
                include: None,
                root,
                manifest,
                application,
            });
        }
        // An application of this repository's checkout (the examples) shares
        // the autoloader and the PHP package of the checkout.
        let checkout = root.ancestors().skip(1).find(|ancestor| {
            ancestor.join("vendor/autoload.php").is_file() && ancestor.join(BOOTSTRAP).is_file()
        });
        let Some(checkout) = checkout else {
            return Err(format!(
                "vendor/autoload.php is missing in {}; run `composer install` in the project",
                root.display()
            ));
        };
        let relative = root
            .strip_prefix(checkout)
            .map_err(|error| error.to_string())?
            .to_path_buf();
        let package = Path::new(BOOTSTRAP)
            .ancestors()
            .nth(2)
            .unwrap_or_else(|| Path::new(""));
        let mut include = vec![PathBuf::from("vendor"), relative.clone()];
        include.extend(CHECKOUT_PACKAGE_FILES.iter().map(|file| package.join(file)));
        Ok(Self {
            source_root: checkout.to_path_buf(),
            relative,
            include: Some(include),
            root,
            manifest,
            application,
        })
    }

    /// The project directory relative to the bundle, with forward slashes.
    fn bundle_project(&self) -> Result<String, String> {
        portable_path(&Path::new("app").join(&self.relative))
    }

    /// The entry script relative to the bundle, with forward slashes.
    fn bundle_entry(&self) -> Result<String, String> {
        let entry: PathBuf = Path::new(&self.application.entry)
            .components()
            .filter(|component| matches!(component, Component::Normal(_)))
            .collect();
        portable_path(&Path::new("app").join(&self.relative).join(entry))
    }
}

fn portable_path(path: &Path) -> Result<String, String> {
    let path: PathBuf = path.components().collect();
    path.to_str()
        .map(|value| value.replace('\\', "/"))
        .ok_or_else(|| format!("path is not valid UTF-8: {}", path.display()))
}

/// The icon to ship: the manifest's, validated, or the generic one.
fn resolve_icon(project: &Project, workspace: &Path) -> Result<PathBuf, String> {
    let Some(icon) = &project.application.icon else {
        let path = workspace.join("default-icon.svg");
        fs::write(&path, DEFAULT_ICON)
            .map_err(|error| format!("cannot write the default icon: {error}"))?;
        return Ok(path);
    };
    let declared = project.root.join(icon);
    let resolved = declared.canonicalize().map_err(|error| {
        format!(
            "cannot resolve the application icon {}: {error}",
            declared.display()
        )
    })?;
    if !resolved.starts_with(&project.root) {
        return Err(format!(
            "the application icon must be inside the project: {}",
            resolved.display()
        ));
    }
    let metadata = resolved
        .metadata()
        .map_err(|error| format!("cannot inspect the icon {}: {error}", resolved.display()))?;
    if !metadata.is_file() || metadata.len() > MAX_ICON_BYTES {
        return Err(format!(
            "the application icon must be a file no larger than {MAX_ICON_BYTES} bytes: {}",
            resolved.display()
        ));
    }
    let contents = fs::read(&resolved)
        .map_err(|error| format!("cannot read the icon {}: {error}", resolved.display()))?;
    match icon_extension(&resolved)?.as_str() {
        "png" => validate_png_icon(&contents)?,
        "svg" => validate_svg_icon(&contents)?,
        _ => return Err("the application icon must use the PNG or SVG extension".to_owned()),
    }
    Ok(resolved)
}

fn icon_extension(icon: &Path) -> Result<String, String> {
    icon.extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| "the application icon has no extension".to_owned())
}

fn validate_png_icon(contents: &[u8]) -> Result<(), String> {
    const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    let dimension = |range: std::ops::Range<usize>| {
        contents
            .get(range)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_be_bytes)
    };
    if contents.get(..8) != Some(PNG_SIGNATURE) || contents.get(12..16) != Some(b"IHDR") {
        return Err("the application PNG icon has an invalid header".to_owned());
    }
    match (dimension(16..20), dimension(20..24)) {
        (Some(width), Some(height)) if width == height && (64..=1024).contains(&width) => Ok(()),
        _ => Err("the application PNG icon must be square and between 64px and 1024px".to_owned()),
    }
}

fn validate_svg_icon(contents: &[u8]) -> Result<(), String> {
    let source = std::str::from_utf8(contents)
        .map_err(|_| "the application SVG icon must contain valid UTF-8".to_owned())?;
    let normalized = source.to_ascii_lowercase();
    if !normalized.contains("<svg")
        || normalized.contains("<script")
        || normalized.contains("<!doctype")
        || normalized.contains("href=\"http")
        || normalized.contains("href='http")
    {
        return Err("the application SVG icon must be self-contained and cannot include scripts or remote resources".to_owned());
    }
    Ok(())
}

// ---- artifact names and destinations --------------------------------------

struct ArtifactNames {
    bundle: String,
    portable: String,
    debian: String,
    appimage: String,
    rpm: String,
    dmg: String,
    pkg: String,
    msix: String,
    msi: String,
    executable: String,
    architecture: String,
}

impl ArtifactNames {
    fn new(application: &Application) -> Result<Self, String> {
        let executable = application.executable().to_owned();
        let version = &application.version;
        let architecture = std::env::consts::ARCH.to_owned();
        let bundle = format!(
            "{executable}-{version}-{}-{architecture}",
            std::env::consts::OS
        );
        let portable = if cfg!(target_os = "linux") {
            format!("{bundle}.tar.gz")
        } else {
            format!("{bundle}.zip")
        };
        let debian = if cfg!(target_os = "linux") {
            format!(
                "{executable}_{version}_{}.deb",
                debian_architecture(&architecture)?
            )
        } else {
            format!("{executable}-{version}-{architecture}.deb")
        };
        Ok(Self {
            portable,
            debian,
            appimage: format!("{executable}-{version}-{architecture}.AppImage"),
            rpm: format!("{executable}-{version}.{architecture}.rpm"),
            dmg: format!("{executable}-{version}-{architecture}.dmg"),
            pkg: format!("{executable}-{version}-{architecture}.pkg"),
            msix: format!("{executable}-{version}-{architecture}.msix"),
            msi: format!("{executable}-{version}-{architecture}.msi"),
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
    appimage: Option<PathBuf>,
    rpm: Option<PathBuf>,
    dmg: Option<PathBuf>,
    pkg: Option<PathBuf>,
    msix: Option<PathBuf>,
    msi: Option<PathBuf>,
}

impl ArtifactDestinations {
    fn new(output: &Path, names: &ArtifactNames, formats: &BTreeSet<PackageFormat>) -> Self {
        let destination = |format: PackageFormat, name: &str| {
            formats.contains(&format).then(|| output.join(name))
        };
        Self {
            directory: destination(PackageFormat::Directory, &names.bundle),
            portable: destination(PackageFormat::Portable, &names.portable),
            debian: destination(PackageFormat::Debian, &names.debian),
            appimage: destination(PackageFormat::AppImage, &names.appimage),
            rpm: destination(PackageFormat::Rpm, &names.rpm),
            dmg: destination(PackageFormat::Dmg, &names.dmg),
            pkg: destination(PackageFormat::Pkg, &names.pkg),
            msix: destination(PackageFormat::Msix, &names.msix),
            msi: destination(PackageFormat::Msi, &names.msi),
        }
    }

    fn paths(&self) -> impl Iterator<Item = &PathBuf> {
        [
            &self.directory,
            &self.portable,
            &self.debian,
            &self.appimage,
            &self.rpm,
            &self.dmg,
            &self.pkg,
            &self.msix,
            &self.msi,
        ]
        .into_iter()
        .flatten()
    }

    fn ensure_available(&self, force: bool) -> Result<(), String> {
        if force {
            return Ok(());
        }
        match self.paths().find(|path| path.exists()) {
            Some(existing) => Err(format!(
                "artifact {} already exists; pass --force to replace this exact build",
                existing.display()
            )),
            None => Ok(()),
        }
    }

    fn remove_existing(&self, force: bool) -> Result<(), String> {
        if !force {
            return Ok(());
        }
        for path in self.paths().filter(|path| path.exists()) {
            let metadata = fs::symlink_metadata(path)
                .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
            let removed = if metadata.is_dir() && !metadata.file_type().is_symlink() {
                fs::remove_dir_all(path)
            } else {
                fs::remove_file(path)
            };
            removed.map_err(|error| format!("cannot replace {}: {error}", path.display()))?;
        }
        Ok(())
    }
}

/// The private staging directory of one build, removed when the build ends.
struct BuildWorkspace {
    path: PathBuf,
}

impl BuildWorkspace {
    fn create(output: &Path) -> Result<Self, String> {
        let mut random = [0_u8; 8];
        getrandom::fill(&mut random)
            .map_err(|error| format!("cannot name the build workspace: {error}"))?;
        let path = output.join(format!(
            ".pam-desktop-build-{}-{}",
            std::process::id(),
            hex(&random)
        ));
        fs::create_dir(&path).map_err(|error| {
            format!(
                "cannot create the build workspace {}: {error}",
                path.display()
            )
        })?;
        Ok(Self { path })
    }
}

impl Drop for BuildWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

// ---- the PHP runtime ------------------------------------------------------

/// Which of the shared extensions PHP loads on the build machine are shipped.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ExtensionChoice<'a> {
    /// Those the application refers to, and the ones listed next to `auto`.
    Used(&'a [String]),
    /// Every one (`"all"`).
    All,
    /// Exactly the listed ones.
    Listed(&'a [String]),
}

impl<'a> ExtensionChoice<'a> {
    /// `extensions` of the manifest: absent, or a list that may hold the
    /// words `auto` and `all`.
    fn of(extensions: Option<&'a [String]>) -> Self {
        match extensions {
            None => Self::Used(&[]),
            Some(names) if names.iter().any(|name| name == "all") => Self::All,
            Some(names) if names.iter().any(|name| name == "auto") => Self::Used(names),
            Some(names) => Self::Listed(names),
        }
    }

    /// The extensions named by hand.
    fn listed(self) -> impl Iterator<Item = &'a String> {
        let names = match self {
            Self::Used(names) | Self::Listed(names) => names,
            Self::All => &[],
        };
        names.iter().filter(|name| *name != "auto")
    }
}

/// Finds the extensions an application refers to; see the file.
const EXTENSION_USAGE: &str = include_str!("extension_usage.php");

/// The PHP that runs the application and the shared extensions it ships with.
#[derive(Clone)]
struct PhpRuntime {
    binary: PathBuf,
    /// `php`, `php.exe` or `pam`: the name of the binary inside the bundle.
    file_name: String,
    arguments: Vec<OsString>,
    version: String,
    /// `(name, file)` of the shared extensions to ship, in load order.
    extensions: Vec<(String, PathBuf)>,
    /// Extensions that stay even when the application does not refer to
    /// them; `None` when the shipped set is not reduced to what is used.
    pinned: Option<Vec<String>>,
}

impl PhpRuntime {
    /// Asks the runtime for its version and extensions. A runtime that does
    /// not answer (`pam`, a static build) is shipped as a single binary.
    fn probe(
        binary: &Path,
        file_name: &str,
        arguments: &[OsString],
        choice: ExtensionChoice<'_>,
    ) -> Result<Self, String> {
        if cfg!(windows) && !arguments.is_empty() {
            return Err(format!(
                "{} needs arguments the bundled launcher cannot pass on Windows; set PAM_DESKTOP_PHP to a php.exe",
                binary.display()
            ));
        }
        let mut runtime = Self {
            binary: binary.to_path_buf(),
            file_name: file_name.to_owned(),
            arguments: arguments.to_vec(),
            version: String::from("unknown"),
            extensions: Vec::new(),
            pinned: match choice {
                ExtensionChoice::Used(_) => Some(choice.listed().cloned().collect()),
                ExtensionChoice::All | ExtensionChoice::Listed(_) => None,
            },
        };
        let probe = "echo json_encode(['version' => PHP_VERSION, 'extensionDir' => ini_get('extension_dir'), 'extensions' => get_loaded_extensions()]);";
        let answer = if arguments.is_empty() {
            Command::new(binary).args(["-r", probe]).output().ok()
        } else {
            None
        };
        let answer = answer
            .filter(|output| output.status.success())
            .and_then(|output| serde_json::from_slice::<serde_json::Value>(&output.stdout).ok());
        let Some(answer) = answer else {
            // `pam --version` ends in its version; other runtimes stay unknown.
            if !arguments.is_empty()
                && let Some(version) = binary_version(binary)
            {
                runtime.version = version;
            }
            return Ok(runtime);
        };
        if let Some(version) = answer.get("version").and_then(serde_json::Value::as_str) {
            runtime.version = version.to_owned();
        }
        let directory = answer
            .get("extensionDir")
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from)
            .unwrap_or_default();
        let loaded: Vec<String> = answer
            .get("extensions")
            .and_then(serde_json::Value::as_array)
            .map(|names| {
                names
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_ascii_lowercase)
                    .collect()
            })
            .unwrap_or_default();
        let shared = |name: &str| {
            let file = if cfg!(windows) {
                format!("php_{name}.dll")
            } else {
                format!("{name}.so")
            };
            Some(directory.join(file))
                .filter(|path| !directory.as_os_str().is_empty() && path.is_file())
        };
        // Loaded extensions first, in the order PHP sorted their dependencies.
        let exact = matches!(choice, ExtensionChoice::Listed(_));
        let mut names: Vec<&str> = loaded
            .iter()
            .map(String::as_str)
            .filter(|name| !exact || choice.listed().any(|item| item == name))
            .collect();
        for name in choice.listed() {
            if !loaded.contains(name) {
                names.push(name);
            }
        }
        for name in names {
            match shared(name) {
                Some(path) => runtime.extensions.push((name.to_owned(), path)),
                // Compiled into the binary: nothing to ship.
                None if loaded.iter().any(|item| item == name) => {}
                None => {
                    return Err(format!(
                        "the PHP extension {name:?} is not available in {}",
                        binary.display()
                    ));
                }
            }
        }
        Ok(runtime)
    }

    /// The runtime with only the extensions the application in `app` refers
    /// to, and a note about the ones left out. Nothing changes when the
    /// manifest chose the extensions, or when PHP cannot tell.
    fn for_application(&self, app: &Path) -> (Self, Option<String>) {
        let Some(pinned) = &self.pinned else {
            return (self.clone(), None);
        };
        if self.extensions.is_empty() {
            return (self.clone(), None);
        }
        let script = EXTENSION_USAGE.trim_start_matches("<?php");
        let answer = Command::new(&self.binary)
            .args(["-r", script, "--"])
            .arg(app)
            .args(self.extensions.iter().map(|(name, _)| name))
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| serde_json::from_slice::<serde_json::Value>(&output.stdout).ok());
        let Some(used) = answer
            .as_ref()
            .and_then(|answer| answer.get("used"))
            .and_then(serde_json::Value::as_array)
        else {
            return (self.clone(), None);
        };
        let used: HashSet<&str> = used.iter().filter_map(serde_json::Value::as_str).collect();
        let mut runtime = self.clone();
        let mut left_out = Vec::new();
        runtime.extensions.retain(|(name, _)| {
            let keep = used.contains(name.as_str()) || pinned.contains(name);
            if !keep {
                left_out.push(name.clone());
            }
            keep
        });
        if left_out.is_empty() {
            return (runtime, None);
        }
        let shipped: Vec<&str> = runtime
            .extensions
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        let note = format!(
            "PHP extensions: shipped {}; left out, as the application does not refer to them: {}. \
             To ship one anyway, set \"extensions\": [\"auto\", \"<name>\"] in pam-desktop.json (or \"all\").",
            if shipped.is_empty() {
                "none".to_owned()
            } else {
                shipped.join(", ")
            },
            left_out.join(", ")
        );
        (runtime, Some(note))
    }

    fn ini(&self) -> String {
        let mut ini =
            String::from("expose_php=Off\ndisplay_errors=Off\nlog_errors=On\nmemory_limit=512M\n");
        if !self.extensions.is_empty() {
            // A bundle does not know where it will be installed.
            ini.push_str(&format!(
                "extension_dir=\"${{{PHP_EXTENSION_DIR_VARIABLE}}}\"\n"
            ));
            for (name, _) in &self.extensions {
                ini.push_str(&format!("extension={name}\n"));
            }
        }
        ini
    }
}

/// The last word `--version` prints, as `pam` reports its version.
fn binary_version(binary: &Path) -> Option<String> {
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    String::from_utf8(output.stdout)
        .ok()?
        .split_whitespace()
        .last()
        .map(str::to_owned)
}

// ---- the bundle -----------------------------------------------------------

struct BundleContext<'a> {
    project: &'a Project,
    names: &'a ArtifactNames,
    icon: &'a Path,
    host: &'a Path,
    runtime: &'a PhpRuntime,
    output: &'a Path,
    sign: bool,
}

/// What the bundle expects from the system it is installed on.
#[derive(Debug, Default, PartialEq, Eq)]
struct SystemDependencies {
    debian: Vec<&'static str>,
    rpm: Vec<&'static str>,
}

/// A bundle on disk: what it needs from the system and what was decided
/// while it was put together.
struct Materialized {
    system: SystemDependencies,
    notes: Vec<String>,
}

fn materialize_bundle(context: &BundleContext<'_>, bundle: &Path) -> Result<Materialized, String> {
    let project = context.project;
    let application = &project.application;
    let app = bundle.join("app");
    let bin = bundle.join("bin");
    let php_bin = bundle.join(PHP_BIN);
    for directory in [&app, &bin, &php_bin, &bundle.join("etc")] {
        fs::create_dir_all(directory)
            .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
    }

    copy_project(project, context.output, &app)?;
    let bundled_project = app.join(&project.relative);
    sanitize_composer_metadata(&bundled_project)?;
    complete_bundled_manifest(project, &bundled_project)?;
    if !bundled_bootstrap(&app, &project.relative).is_some_and(|bootstrap| bootstrap.is_file()) {
        return Err("the bundle has no PAM Desktop PHP package: `vendor/pushinbr/pam-desktop` is missing or was excluded".to_owned());
    }
    // Decided on the files that are shipped, not on the whole project.
    let (runtime, note) = context.runtime.for_application(&app);
    let runtime = &runtime;
    let notes: Vec<String> = note.into_iter().collect();

    let exe = std::env::consts::EXE_SUFFIX;
    copy_binary(context.host, &bin.join(format!("pam-desktop{exe}")))?;
    copy_binary(&runtime.binary, &php_bin.join(&runtime.file_name))?;
    let extension_sources: Vec<&Path> = runtime
        .extensions
        .iter()
        .map(|(_, path)| path.as_path())
        .collect();
    if !extension_sources.is_empty() {
        let extensions = bundle.join(PHP_EXTENSIONS);
        fs::create_dir_all(&extensions).map_err(|error| error.to_string())?;
        for source in &extension_sources {
            let name = source
                .file_name()
                .ok_or("a PHP extension has no file name")?;
            fs::copy(source, extensions.join(name))
                .map_err(|error| format!("cannot bundle {}: {error}", source.display()))?;
        }
    }

    let launcher = bin.join(format!("{}{exe}", context.names.executable));
    let mut system = SystemDependencies::default();
    if cfg!(target_os = "linux") {
        let php_objects =
            std::iter::once(runtime.binary.as_path()).chain(extension_sources.iter().copied());
        copy_libraries(&linux_libraries(php_objects)?, &bundle.join(PHP_LIB))?;
        let host_libraries = linux_libraries([context.host])?;
        let platform = platform_libraries(&host_libraries)?;
        system = system_dependencies(host_libraries.keys(), uses_web_view(&bundled_project));
        let own: BTreeMap<OsString, PathBuf> = host_libraries
            .into_iter()
            .filter(|(name, _)| !platform.contains(name))
            .collect();
        copy_libraries(&own, &bundle.join("lib"))?;
    } else if cfg!(target_os = "macos") {
        copy_adjacent_libraries(context.host, &bundle.join("lib"), "dylib")?;
        copy_adjacent_libraries(&runtime.binary, &bundle.join(PHP_LIB), "dylib")?;
    } else {
        // Windows finds the DLLs of an executable beside it.
        copy_adjacent_libraries(context.host, &bin, "dll")?;
        copy_adjacent_libraries(&runtime.binary, &php_bin, "dll")?;
    }
    let bundled_project_path = project.bundle_project()?;
    if cfg!(windows) {
        // A script cannot be the executable of a desktop application there:
        // the host, started under the application's name, launches the bundle.
        copy_binary(context.host, &launcher)?;
    } else {
        write_script(&bin.join(PHP_WRAPPER), &php_wrapper(runtime)?)?;
        write_script(&launcher, &launcher_script(&bundled_project_path)?)?;
    }
    fs::write(bundle.join(PHP_INI), runtime.ini())
        .map_err(|error| format!("cannot write the bundled php.ini: {error}"))?;

    let icon_destination = bundle_icon_path(bundle, application, context.icon)?;
    if let Some(parent) = icon_destination.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::copy(context.icon, &icon_destination).map_err(|error| {
        format!(
            "cannot copy the icon {} to {}: {error}",
            context.icon.display(),
            icon_destination.display()
        )
    })?;
    if cfg!(target_os = "linux") {
        let applications = bundle.join("share/applications");
        fs::create_dir_all(&applications).map_err(|error| error.to_string())?;
        fs::write(
            applications.join(format!("{}.desktop.in", application.identifier)),
            desktop_entry(application, "@PAM_EXEC@"),
        )
        .map_err(|error| format!("cannot write the desktop entry template: {error}"))?;
        write_portable_scripts(bundle, application, context.icon)?;
    }
    if context.sign && cfg!(target_os = "windows") {
        sign_windows_bundle(bundle)?;
    }
    if context.sign && cfg!(target_os = "macos") {
        sign_macos_bundle_binaries(bundle)?;
    }

    let manifest = BundleManifest {
        schema_version: bundle::SCHEMA_VERSION,
        application: BundleApplication {
            identifier: application.identifier.clone(),
            name: application.name.clone(),
            version: application.version.clone(),
            build: application.build.clone(),
            description: application.description.clone(),
            publisher: application.publisher.clone(),
            category: application.category.keyword().to_owned(),
            updates: application.updates.as_ref().map(|updates| BundleUpdates {
                endpoint: updates.endpoint.clone(),
                public_key: updates.public_key.clone(),
                channel: updates.channel.clone(),
                policy: updates.policy.keyword().to_owned(),
            }),
        },
        project: bundled_project_path,
        entry: project.bundle_entry()?,
        launcher: format!("bin/{}{exe}", context.names.executable),
        runtime: BundleRuntime {
            pam_desktop: env!("CARGO_PKG_VERSION").to_owned(),
            php: runtime.version.clone(),
            php_binary: runtime.file_name.clone(),
        },
        target: BundleTarget::current(),
        source_date_epoch: source_date_epoch()?,
        files: Vec::new(),
    };
    bundle::write_manifest(bundle, manifest)?;
    normalize_permissions(bundle)?;
    Ok(Materialized { system, notes })
}

/// Where the host will find the worker bootstrap of the bundled project: in
/// its `vendor`, or in a directory above it (a checkout).
fn bundled_bootstrap(app: &Path, relative: &Path) -> Option<PathBuf> {
    let project = app.join(relative);
    let vendored = project.join(VENDOR_PACKAGE).join(BOOTSTRAP);
    if vendored.is_file() {
        return Some(vendored);
    }
    project
        .ancestors()
        .take_while(|ancestor| ancestor.starts_with(app))
        .map(|ancestor| ancestor.join(BOOTSTRAP))
        .find(|candidate| candidate.is_file())
}

/// Writes what the build resolved into the bundled manifest, so the
/// installed application runs with the identifier and version its packages
/// and update feed carry, whatever its directory is called, and starts with
/// the script the build was given.
fn complete_bundled_manifest(project: &Project, bundled_project: &Path) -> Result<(), String> {
    let application = &project.application;
    let mut document = project.manifest.document().clone();
    let resolved = [
        (
            "applicationId",
            &application.identifier,
            project.manifest.text("applicationId"),
        ),
        (
            "version",
            &application.version,
            project.manifest.text("version"),
        ),
        ("entry", &application.entry, Some(project.manifest.entry())),
    ];
    let mut changed = false;
    for (key, value, current) in resolved {
        if current.as_deref() != Some(value.as_str()) {
            document.insert(key.to_owned(), serde_json::Value::String(value.clone()));
            changed = true;
        }
    }
    if !changed {
        return Ok(());
    }
    write_json_object(&bundled_project.join(manifest::FILE_NAME), &document)
}

struct CopyContext<'a> {
    /// Links may not leave this directory, except below `vendor`.
    source_root: &'a Path,
    build_output: &'a Path,
    /// Relative to `source_root`.
    excludes: Vec<PathBuf>,
}

fn copy_project(project: &Project, output: &Path, app: &Path) -> Result<(), String> {
    let context = CopyContext {
        source_root: &project.source_root,
        build_output: output,
        excludes: project
            .application
            .exclude
            .iter()
            .map(|path| project.relative.join(path))
            .collect(),
    };
    let mut active = HashSet::new();
    let Some(include) = &project.include else {
        return copy_directory(
            &context,
            &project.source_root,
            app,
            Path::new(""),
            &mut active,
        );
    };
    for relative in include {
        let source = project.source_root.join(relative);
        let destination = app.join(relative);
        if excluded_path(relative, &context.excludes) || !source.exists() {
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        }
        if source.is_dir() {
            fs::create_dir_all(&destination)
                .map_err(|error| format!("cannot create {}: {error}", destination.display()))?;
            copy_directory(&context, &source, &destination, relative, &mut active)?;
        } else {
            fs::copy(&source, &destination)
                .map_err(|error| format!("cannot copy {}: {error}", source.display()))?;
        }
    }
    Ok(())
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
            if excluded_path(&child_relative, &context.excludes) {
                continue;
            }
            let target = destination.join(entry.file_name());
            let file_type = entry
                .file_type()
                .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
            if file_type.is_dir() {
                let canonical = path
                    .canonicalize()
                    .map_err(|error| format!("cannot resolve {}: {error}", path.display()))?;
                if canonical.starts_with(context.build_output) {
                    continue;
                }
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

/// A bundle has no links: what a link points at is copied in its place.
/// Composer links path packages from outside the project into `vendor`;
/// anywhere else a link that leaves the project is refused.
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
    let vendored = relative
        .components()
        .any(|component| component.as_os_str() == "vendor");
    if !canonical.starts_with(context.source_root) && !vendored {
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

/// Removes what only makes sense on the build machine: `path` repositories
/// and the local locations Composer recorded for them.
fn sanitize_composer_metadata(project: &Path) -> Result<(), String> {
    let manifest_path = project.join("composer.json");
    if manifest_path.is_file() {
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
    }

    let lock_path = project.join("composer.lock");
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
    let source =
        fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    match serde_json::from_slice::<serde_json::Value>(&source) {
        Ok(serde_json::Value::Object(object)) => Ok(object),
        Ok(_) => Err(format!("{} must contain a JSON object", path.display())),
        Err(error) => Err(format!("{} is not valid JSON: {error}", path.display())),
    }
}

fn write_json_object(
    path: &Path,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    let mut encoded = serde_json::to_vec_pretty(object)
        .map_err(|error| format!("cannot serialize {}: {error}", path.display()))?;
    encoded.push(b'\n');
    fs::write(path, encoded).map_err(|error| format!("cannot write {}: {error}", path.display()))
}

/// Whether a path stays out of the bundle: version control, build output,
/// secrets, the host's development state, the development dependencies of
/// vendored packages, and what the manifest excludes.
fn excluded_path(relative: &Path, configured: &[PathBuf]) -> bool {
    let names = || {
        relative.components().filter_map(|component| {
            if let Component::Normal(name) = component {
                Some(name)
            } else {
                None
            }
        })
    };
    let vendor_depth = names().position(|name| name == "vendor");
    let nested_package_vendor =
        vendor_depth.is_some_and(|depth| names().skip(depth + 1).any(|name| name == "vendor"));
    let default = names().any(|name| {
        let name = name.to_string_lossy();
        matches!(
            name.as_ref(),
            ".git"
                | ".pam"
                | ".pam-desktop"
                | ".pamignore"
                | "dist"
                | "node_modules"
                | "target"
                | ".env"
        ) || name.starts_with(".env.")
    });
    default
        || nested_package_vendor
        || configured
            .iter()
            .any(|excluded| relative.starts_with(excluded))
}

fn copy_binary(source: &Path, destination: &Path) -> Result<(), String> {
    fs::copy(source, destination).map_err(|error| {
        format!(
            "cannot copy {} to {}: {error}",
            source.display(),
            destination.display()
        )
    })?;
    make_executable(destination)
}

/// The shared libraries of `objects` outside the base system ABI, by file name.
fn linux_libraries<'a>(
    objects: impl IntoIterator<Item = &'a Path>,
) -> Result<BTreeMap<OsString, PathBuf>, String> {
    let mut libraries = BTreeMap::<OsString, PathBuf>::new();
    for object in objects {
        for library in runtime_libraries(object)? {
            let name = library
                .file_name()
                .ok_or_else(|| format!("runtime library has no file name: {}", library.display()))?
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
    Ok(libraries)
}

fn copy_libraries(
    libraries: &BTreeMap<OsString, PathBuf>,
    destination: &Path,
) -> Result<(), String> {
    if libraries.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(destination)
        .map_err(|error| format!("cannot create {}: {error}", destination.display()))?;
    for (name, source) in libraries {
        fs::copy(source, destination.join(name))
            .map_err(|error| format!("cannot bundle {}: {error}", source.display()))?;
    }
    Ok(())
}

/// The libraries that ship beside a binary (macOS and Windows builds).
fn copy_adjacent_libraries(
    binary: &Path,
    destination: &Path,
    extension: &str,
) -> Result<(), String> {
    let Some(parent) = binary.parent() else {
        return Ok(());
    };
    for entry in fs::read_dir(parent).map_err(|error| {
        format!(
            "cannot inspect the runtime directory {}: {error}",
            parent.display()
        )
    })? {
        let path = entry.map_err(|error| error.to_string())?.path();
        let matches = path
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|found| found.eq_ignore_ascii_case(extension));
        let Some(name) = path.file_name().filter(|_| matches && path.is_file()) else {
            continue;
        };
        fs::create_dir_all(destination)
            .map_err(|error| format!("cannot create {}: {error}", destination.display()))?;
        fs::copy(&path, destination.join(name))
            .map_err(|error| format!("cannot bundle {}: {error}", path.display()))?;
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
        let Some(name) = path.file_name().and_then(OsStr::to_str) else {
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

/// Libraries of the desktop platform the host is built on. They come from
/// the system: a bundled GTK would not match the theme, the portals or the
/// GPU drivers of the machine it runs on.
fn platform_root(name: &str) -> bool {
    [
        "libgtk-4.",
        "libadwaita-1.",
        "libwebkitgtk-",
        "libjavascriptcoregtk-",
        "libgst",
        "libglib-2.0.",
        "libgobject-2.0.",
        "libgio-2.0.",
        "libgmodule-2.0.",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}

/// The platform libraries among `libraries` and everything they depend on.
fn platform_libraries(
    libraries: &BTreeMap<OsString, PathBuf>,
) -> Result<HashSet<OsString>, String> {
    let mut platform = HashSet::new();
    for (name, path) in libraries {
        // `ldd` lists whole closures: a root inside one is already covered.
        if !name.to_str().is_some_and(platform_root) || platform.contains(name) {
            continue;
        }
        platform.insert(name.clone());
        for dependency in runtime_libraries(path)? {
            if let Some(name) = dependency.file_name() {
                platform.insert(name.to_os_string());
            }
        }
    }
    Ok(platform)
}

/// The packages that provide the platform libraries the host links against,
/// and WebKitGTK when the application shows web pages: the host loads it on
/// the first `WebView`, so it is not among the linked libraries.
fn system_dependencies<'a>(
    libraries: impl IntoIterator<Item = &'a OsString>,
    web_view: bool,
) -> SystemDependencies {
    let names: Vec<&str> = libraries
        .into_iter()
        .filter_map(|name| name.to_str())
        .collect();
    let links = |prefix: &str| names.iter().any(|name| name.starts_with(prefix));
    let mut dependencies = SystemDependencies {
        debian: vec!["libc6"],
        rpm: vec!["glibc"],
    };
    let mut add = |linked: bool, debian: &[&'static str], rpm: &[&'static str]| {
        if linked {
            dependencies.debian.extend(debian);
            dependencies.rpm.extend(rpm);
        }
    };
    add(links("libgtk-4."), &["libgtk-4-1"], &["gtk4"]);
    add(links("libadwaita-1."), &["libadwaita-1-0"], &["libadwaita"]);
    add(web_view, &["libwebkitgtk-6.0-4"], &["webkitgtk6.0"]);
    // GTK plays media through GStreamer, which needs its plugins at run time.
    add(
        links("libgst"),
        &["gstreamer1.0-plugins-base", "gstreamer1.0-plugins-good"],
        &["gstreamer1-plugins-base", "gstreamer1-plugins-good"],
    );
    dependencies
}

/// Whether a source file of the application names the `WebView` element.
/// Packages installed by Composer are not searched: the PAM Desktop package
/// itself defines the class.
fn uses_web_view(project: &Path) -> bool {
    fn search(directory: &Path, depth: usize) -> bool {
        let Ok(entries) = fs::read_dir(directory) else {
            return false;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                let skipped = matches!(
                    entry.file_name().to_str(),
                    Some("vendor" | "node_modules" | ".git")
                );
                if !skipped && depth < 24 && search(&path, depth + 1) {
                    return true;
                }
            } else if kind.is_file()
                && matches!(
                    path.extension().and_then(OsStr::to_str),
                    Some("php" | "pam")
                )
                && fs::read(&path).is_ok_and(|source| {
                    source
                        .windows(b"WebView".len())
                        .any(|window| window == b"WebView")
                })
            {
                return true;
            }
        }
        false
    }
    search(project, 0)
}

/// `value` inside a double-quoted shell word.
fn shell_double_quoted(value: &str) -> Result<String, String> {
    if value.contains(['\n', '\r', '\0']) {
        return Err(format!("path {value:?} cannot be used in a launcher"));
    }
    let mut quoted = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '"' | '\\' | '$' | '`') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    Ok(quoted)
}

const fn library_path_variable() -> &'static str {
    if cfg!(target_os = "macos") {
        "DYLD_LIBRARY_PATH"
    } else {
        "LD_LIBRARY_PATH"
    }
}

/// The launcher: finds the bundle through links, points the host at the
/// bundled PHP and starts the application.
fn launcher_script(project: &str) -> Result<String, String> {
    Ok(LAUNCHER
        .replace("@LIBRARY_PATH@", library_path_variable())
        .replace("@PHP@", PHP_WRAPPER)
        .replace("@PROJECT@", &shell_double_quoted(project)?))
}

const LAUNCHER: &str = r#"#!/bin/sh
set -eu
PAM_LAUNCHER=$0
while [ -L "$PAM_LAUNCHER" ]; do
    PAM_LINK=$(readlink "$PAM_LAUNCHER")
    case "$PAM_LINK" in
        /*) PAM_LAUNCHER=$PAM_LINK ;;
        *) PAM_LAUNCHER=$(dirname -- "$PAM_LAUNCHER")/$PAM_LINK ;;
    esac
done
PAM_LAUNCHER=$(CDPATH= cd -- "$(dirname -- "$PAM_LAUNCHER")" && pwd)/$(basename -- "$PAM_LAUNCHER")
PAM_BUNDLE=$(CDPATH= cd -- "$(dirname -- "$PAM_LAUNCHER")/.." && pwd)
export PAM_DESKTOP_BUNDLE=1
export PAM_DESKTOP_BUNDLE_ROOT="$PAM_BUNDLE"
export PAM_DESKTOP_LAUNCHER="$PAM_LAUNCHER"
export PAM_DESKTOP_PHP="$PAM_BUNDLE/bin/@PHP@"
if [ -d "$PAM_BUNDLE/lib" ]; then
    export @LIBRARY_PATH@="$PAM_BUNDLE/lib${@LIBRARY_PATH@:+:$@LIBRARY_PATH@}"
fi
# The link the system opened the application with (`%u` of the desktop entry).
if [ "$#" -gt 0 ] && [ -z "${PAM_DESKTOP_LAUNCH_URL:-}" ]; then
    export PAM_DESKTOP_LAUNCH_URL="$1"
fi
cd "$PAM_BUNDLE/@PROJECT@"
exec "$PAM_BUNDLE/bin/pam-desktop" run "$PAM_BUNDLE/@PROJECT@"
"#;

/// Starts the bundled PHP. Its libraries and configuration are set here and
/// not in the launcher, so the host keeps resolving its own libraries (GTK
/// and what GTK loads) from the system.
fn php_wrapper(runtime: &PhpRuntime) -> Result<String, String> {
    let mut arguments = String::new();
    for argument in &runtime.arguments {
        let argument = argument
            .to_str()
            .ok_or("the PHP runtime arguments must be valid UTF-8")?;
        arguments.push_str(&format!("\"{}\" ", shell_double_quoted(argument)?));
    }
    Ok(PHP_WRAPPER_SCRIPT
        .replace("@LIBRARY_PATH@", library_path_variable())
        .replace("@EXTENSION_DIR@", PHP_EXTENSION_DIR_VARIABLE)
        .replace("@INI@", PHP_INI)
        .replace("@EXTENSIONS@", PHP_EXTENSIONS)
        .replace("@LIB@", PHP_LIB)
        .replace("@BIN@", PHP_BIN)
        .replace("@PHP@", &shell_double_quoted(&runtime.file_name)?)
        .replace("@ARGUMENTS@", &arguments))
}

const PHP_WRAPPER_SCRIPT: &str = r#"#!/bin/sh
set -eu
PAM_BUNDLE=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
export PHPRC="$PAM_BUNDLE/@INI@"
export PHP_INI_SCAN_DIR=
export @EXTENSION_DIR@="$PAM_BUNDLE/@EXTENSIONS@"
if [ -d "$PAM_BUNDLE/@LIB@" ]; then
    export @LIBRARY_PATH@="$PAM_BUNDLE/@LIB@${@LIBRARY_PATH@:+:$@LIBRARY_PATH@}"
fi
exec "$PAM_BUNDLE/@BIN@/@PHP@" @ARGUMENTS@"$@"
"#;

fn write_script(path: &Path, script: &str) -> Result<(), String> {
    fs::write(path, script).map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    make_executable(path)
}

/// Where the icon sits in the hicolor theme: `(theme directory, extension)`.
fn icon_theme(icon: &Path) -> Result<(&'static str, String), String> {
    let extension = icon_extension(icon)?;
    Ok((
        if extension == "svg" {
            "scalable"
        } else {
            "512x512"
        },
        extension,
    ))
}

fn bundle_icon_path(
    bundle: &Path,
    application: &Application,
    icon: &Path,
) -> Result<PathBuf, String> {
    let (theme, extension) = icon_theme(icon)?;
    Ok(bundle.join(format!(
        "share/icons/hicolor/{theme}/apps/{}.{extension}",
        application.identifier
    )))
}

/// The desktop entry: one link per launch (`%u`), and the schemes of
/// `urlSchemes` as the MIME types the application handles.
fn desktop_entry(application: &Application, executable: &str) -> String {
    let mime_types: String = application
        .url_schemes
        .iter()
        .map(|scheme| format!("x-scheme-handler/{scheme};"))
        .collect();
    let mime_line = if mime_types.is_empty() {
        String::new()
    } else {
        format!("MimeType={mime_types}\n")
    };
    format!(
        "[Desktop Entry]\nType=Application\nVersion=1.5\nName={}\nComment={}\nExec={executable} %u\nIcon={}\nTerminal=false\nCategories={};\n{mime_line}StartupWMClass={}\n",
        desktop_value(&application.name),
        desktop_value(&application.description),
        application.identifier,
        freedesktop_category(application.category),
        application.identifier,
    )
}

fn desktop_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace(';', "\\;")
}

const fn freedesktop_category(category: Category) -> &'static str {
    match category {
        Category::Development => "Development",
        Category::Productivity => "Office",
        Category::Graphics => "Graphics",
        Category::AudioVideo => "AudioVideo",
        Category::Network => "Network",
        Category::Utility => "Utility",
        Category::Game => "Game",
        Category::Education => "Education",
    }
}

/// `install.sh` and `uninstall.sh` of the portable bundle.
fn write_portable_scripts(
    bundle: &Path,
    application: &Application,
    icon: &Path,
) -> Result<(), String> {
    let (theme, extension) = icon_theme(icon)?;
    for (name, template) in [
        ("install.sh", PORTABLE_INSTALLER),
        ("uninstall.sh", PORTABLE_UNINSTALLER),
    ] {
        let script = template
            .replace("@APP_ID@", &application.identifier)
            .replace("@EXECUTABLE@", application.executable())
            .replace("@ICON_THEME@", theme)
            .replace("@ICON_EXTENSION@", &extension);
        write_script(&bundle.join(name), &script)?;
    }
    Ok(())
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
    if [ "$line" = "Exec=@PAM_EXEC@ %u" ]; then
        printf 'Exec=%s %%u\n' "$TARGET/bin/@EXECUTABLE@"
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

// ---- archives and installers ----------------------------------------------

struct PackageContext<'a> {
    workspace: &'a Path,
    bundle: &'a Path,
    application: &'a Application,
    names: &'a ArtifactNames,
    icon: &'a Path,
    system: &'a SystemDependencies,
    sign: bool,
}

fn run_tool(command: &mut Command, tool: &str) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|error| format!("cannot start {tool}: {error}"))?;
    if !status.success() {
        return Err(format!("{tool} failed with status {status}"));
    }
    Ok(())
}

/// The update-ready archive: the bundle directory (the `.app` on macOS).
/// On Linux, entry order, ownership, timestamps and the gzip header are
/// fixed, so the same inputs give the same bytes.
fn create_portable_archive(
    package: &PackageContext<'_>,
    output: &Path,
    source_date_epoch: u64,
) -> Result<(), String> {
    let bundle_name = package
        .bundle
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or("the bundle name is not valid UTF-8")?;
    if cfg!(target_os = "linux") {
        run_tool(
            Command::new("tar")
                .args([
                    "--sort=name",
                    &format!("--mtime=@{source_date_epoch}"),
                    "--owner=0",
                    "--group=0",
                    "--numeric-owner",
                ])
                .args(["--use-compress-program", "gzip -n", "-cf"])
                .arg(output)
                .arg("-C")
                .arg(package.workspace)
                .arg(bundle_name),
            "tar",
        )
    } else if cfg!(target_os = "macos") {
        let application = create_macos_application(package)?;
        run_tool(
            Command::new("ditto")
                .args(["-c", "-k", "--sequesterRsrc", "--keepParent"])
                .arg(application)
                .arg(output),
            "ditto",
        )
    } else {
        run_tool(
            Command::new("powershell.exe")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Compress-Archive -LiteralPath $env:PAM_ARCHIVE_SOURCE -DestinationPath $env:PAM_ARCHIVE_OUTPUT -CompressionLevel Optimal -Force",
                ])
                .env("PAM_ARCHIVE_SOURCE", package.bundle)
                .env("PAM_ARCHIVE_OUTPUT", output),
            "the portable archive tool",
        )
    }
}

/// The parts of the bundle a system package installs below `/opt`.
const INSTALLED_BUNDLE_ENTRIES: [&str; 6] =
    ["app", "bin", "lib", "etc", "runtime", bundle::MANIFEST];

/// Stages `/opt/<id>`, the launcher link, the desktop entry and the icon the
/// way Debian and RPM packages install them.
fn stage_system_tree(package: &PackageContext<'_>, root: &Path) -> Result<(), String> {
    let application = package.application;
    let installed = format!(
        "/opt/{}/bin/{}",
        application.identifier, package.names.executable
    );
    let application_root = root.join("opt").join(&application.identifier);
    fs::create_dir_all(&application_root).map_err(|error| error.to_string())?;
    for entry in INSTALLED_BUNDLE_ENTRIES {
        let source = package.bundle.join(entry);
        if source.exists() {
            hardlink_or_copy(&source, &application_root.join(entry))?;
        }
    }
    let applications = root.join("usr/share/applications");
    fs::create_dir_all(&applications).map_err(|error| error.to_string())?;
    fs::write(
        applications.join(format!("{}.desktop", application.identifier)),
        desktop_entry(application, &installed),
    )
    .map_err(|error| format!("cannot write the desktop entry: {error}"))?;

    let source_icon = bundle_icon_path(package.bundle, application, package.icon)?;
    let relative_icon = source_icon
        .strip_prefix(package.bundle)
        .map_err(|error| error.to_string())?;
    let destination_icon = root.join("usr").join(relative_icon);
    if let Some(parent) = destination_icon.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    hardlink_or_copy(&source_icon, &destination_icon)?;

    let usr_bin = root.join("usr/bin");
    fs::create_dir_all(&usr_bin).map_err(|error| error.to_string())?;
    symlink(&installed, &usr_bin.join(&package.names.executable))
}

fn symlink(target: &str, link: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
            .map_err(|error| format!("cannot create the link {}: {error}", link.display()))
    }
    #[cfg(not(unix))]
    {
        let _ = target;
        Err(format!(
            "cannot create the link {}: links are a Unix packaging feature",
            link.display()
        ))
    }
}

fn package_description(application: &Application) -> &str {
    if application.description.is_empty() {
        &application.name
    } else {
        &application.description
    }
}

fn create_debian_package(
    package: &PackageContext<'_>,
    output: &Path,
    source_date_epoch: u64,
) -> Result<(), String> {
    let application = package.application;
    let root = package.workspace.join("debian-root");
    stage_system_tree(package, &root)?;
    let control_directory = root.join("DEBIAN");
    fs::create_dir_all(&control_directory).map_err(|error| error.to_string())?;
    let control = format!(
        "Package: {}\nVersion: {}\nSection: {}\nPriority: optional\nArchitecture: {}\nMaintainer: {}\nInstalled-Size: {}\nDepends: {}\nDescription: {}\n {}\n",
        application.identifier,
        application.version,
        debian_section(application.category),
        debian_architecture(&package.names.architecture)?,
        application.publisher,
        directory_bytes(&root)?.div_ceil(1024),
        package.system.debian.join(", "),
        application.name,
        package_description(application),
    );
    fs::write(control_directory.join("control"), control)
        .map_err(|error| format!("cannot write the Debian control file: {error}"))?;
    normalize_permissions(&root)?;
    run_tool(
        Command::new("dpkg-deb")
            .args(["--root-owner-group", "--build"])
            .arg(&root)
            .arg(output)
            .env("SOURCE_DATE_EPOCH", source_date_epoch.to_string()),
        "dpkg-deb",
    )
}

fn create_appimage_package(package: &PackageContext<'_>, output: &Path) -> Result<(), String> {
    let application = package.application;
    let executable = &package.names.executable;
    let app_dir = package.workspace.join(format!("{executable}.AppDir"));
    hardlink_or_copy(
        package.bundle,
        &app_dir.join("usr/lib").join(&application.identifier),
    )?;
    let usr_bin = app_dir.join("usr/bin");
    fs::create_dir_all(&usr_bin).map_err(|error| error.to_string())?;
    symlink(
        &format!("../lib/{}/bin/{executable}", application.identifier),
        &usr_bin.join(executable),
    )?;
    let desktop_name = format!("{}.desktop", application.identifier);
    fs::write(
        app_dir.join(&desktop_name),
        desktop_entry(application, executable),
    )
    .map_err(|error| format!("cannot write the AppImage desktop entry: {error}"))?;
    let icon_name = format!(
        "{}.{}",
        application.identifier,
        icon_extension(package.icon)?
    );
    fs::copy(package.icon, app_dir.join(&icon_name))
        .map_err(|error| format!("cannot copy the AppImage icon: {error}"))?;
    symlink(&desktop_name, &app_dir.join("AppRun.desktop"))?;
    write_script(
        &app_dir.join("AppRun"),
        &format!(
            "#!/bin/sh\nset -eu\nHERE=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\nexec \"$HERE/usr/bin/{executable}\" \"$@\"\n"
        ),
    )?;
    run_tool(
        Command::new("appimagetool").arg(&app_dir).arg(output).env(
            "ARCH",
            linux_architecture(&package.names.architecture, "AppImage")?,
        ),
        "appimagetool",
    )
}

fn create_rpm_package(package: &PackageContext<'_>, output: &Path) -> Result<(), String> {
    let application = package.application;
    let executable = &package.names.executable;
    let top = package.workspace.join("rpmbuild");
    let payload = package.workspace.join("rpm-payload");
    stage_system_tree(package, &payload)?;
    let specs = top.join("SPECS");
    fs::create_dir_all(&specs).map_err(|error| error.to_string())?;
    let spec = format!(
        "Name: {executable}\nVersion: {}\nRelease: 1%{{?dist}}\nSummary: {}\nLicense: Proprietary\nBuildArch: {}\nRequires: {}\n\n%description\n{}\n\n%install\nmkdir -p %{{buildroot}}\ncp -a {}/* %{{buildroot}}/\n\n%files\n/opt/{}\n/usr/bin/{executable}\n/usr/share/applications/{}.desktop\n/usr/share/icons\n",
        application.version.replace('-', "~"),
        rpm_escape(&application.name),
        linux_architecture(&package.names.architecture, "RPM")?,
        package.system.rpm.join(", "),
        rpm_escape(package_description(application)),
        payload.display(),
        application.identifier,
        application.identifier,
    );
    let spec_path = specs.join(format!("{executable}.spec"));
    fs::write(&spec_path, spec).map_err(|error| format!("cannot write the RPM spec: {error}"))?;
    run_tool(
        Command::new("rpmbuild")
            .args(["-bb", "--define"])
            .arg(format!("_topdir {}", top.display()))
            .arg(&spec_path),
        "rpmbuild",
    )?;
    let built = find_single_extension(&top.join("RPMS"), "rpm")?;
    fs::rename(&built, output).map_err(|error| format!("cannot stage the RPM: {error}"))
}

fn linux_architecture(architecture: &str, format: &str) -> Result<&'static str, String> {
    match architecture {
        "x86_64" => Ok("x86_64"),
        "aarch64" => Ok("aarch64"),
        value => Err(format!("unsupported {format} architecture {value:?}")),
    }
}

fn rpm_escape(value: &str) -> String {
    value.replace(['\n', '\r'], " ").replace('%', "%%")
}

fn find_single_extension(root: &Path, extension: &str) -> Result<PathBuf, String> {
    let mut matches = Vec::new();
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|error| format!("cannot inspect {}: {error}", directory.display()))?
        {
            let path = entry.map_err(|error| error.to_string())?.path();
            if path.is_dir() {
                directories.push(path);
            } else if path.extension().and_then(OsStr::to_str) == Some(extension) {
                matches.push(path);
            }
        }
    }
    match (matches.pop(), matches.is_empty()) {
        (Some(path), true) => Ok(path),
        (found, _) => Err(format!(
            "expected exactly one .{extension} artifact, found {}",
            matches.len() + usize::from(found.is_some())
        )),
    }
}

/// The `.app`: the bundle below `Contents/Resources/runtime` and a launcher
/// that tells the updater to replace the whole application.
fn create_macos_application(package: &PackageContext<'_>) -> Result<PathBuf, String> {
    let executable_name = &package.names.executable;
    let application = package.workspace.join(format!("{executable_name}.app"));
    if application.exists() {
        return Ok(application);
    }
    let contents = application.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    fs::create_dir_all(&macos).map_err(|error| error.to_string())?;
    fs::create_dir_all(&resources).map_err(|error| error.to_string())?;
    hardlink_or_copy(package.bundle, &resources.join("runtime"))?;
    let executable = macos.join(executable_name);
    write_script(
        &executable,
        &format!(
            "#!/bin/sh\nset -eu\nAPP=$(CDPATH= cd -- \"$(dirname -- \"$0\")/../..\" && pwd)\nexport PAM_DESKTOP_UPDATE_ROOT=\"$APP\"\nexec \"$APP/Contents/Resources/runtime/bin/{executable_name}\" \"$@\"\n"
        ),
    )?;
    let icon_name = format!("AppIcon.{}", icon_extension(package.icon)?);
    fs::copy(package.icon, resources.join(&icon_name))
        .map_err(|error| format!("cannot copy the macOS application icon: {error}"))?;
    fs::write(
        contents.join("Info.plist"),
        macos_info_plist(package.application, executable_name, &icon_name),
    )
    .map_err(|error| format!("cannot write the macOS Info.plist: {error}"))?;
    if package.sign {
        sign_macos_path(&executable, None)?;
        sign_macos_path(&application, None)?;
    }
    Ok(application)
}

fn macos_info_plist(application: &Application, executable: &str, icon_name: &str) -> String {
    let version = xml_escape(&numeric_version(&application.version, 3));
    let build = if application.build.is_empty() {
        version.clone()
    } else {
        xml_escape(&numeric_version(&application.build, 3))
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleDisplayName</key><string>{name}</string>
  <key>CFBundleExecutable</key><string>{executable}</string>
  <key>CFBundleIconFile</key><string>{icon}</string>
  <key>CFBundleIdentifier</key><string>{identifier}</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>{name}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundleVersion</key><string>{build}</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
{url_types}</dict>
</plist>
"#,
        url_types = macos_url_types(application),
        name = xml_escape(&application.name),
        executable = xml_escape(executable),
        icon = xml_escape(icon_name),
        identifier = xml_escape(&application.identifier),
    )
}

/// `CFBundleURLTypes`: the schemes macOS opens the application for.
fn macos_url_types(application: &Application) -> String {
    if application.url_schemes.is_empty() {
        return String::new();
    }
    let schemes: String = application
        .url_schemes
        .iter()
        .map(|scheme| format!("<string>{}</string>", xml_escape(scheme)))
        .collect();
    format!(
        "  <key>CFBundleURLTypes</key>\n  <array>\n    <dict>\n      <key>CFBundleURLName</key><string>{}</string>\n      <key>CFBundleURLSchemes</key><array>{schemes}</array>\n    </dict>\n  </array>\n",
        xml_escape(&application.identifier),
    )
}

/// The first `parts` numbers of a version, as Apple and Windows want them.
fn numeric_version(version: &str, parts: usize) -> String {
    let mut numbers: Vec<u16> = version
        .split(|character: char| !character.is_ascii_digit())
        .filter(|value| !value.is_empty())
        .take(parts)
        .map(|value| value.parse::<u16>().unwrap_or(u16::MAX))
        .collect();
    numbers.resize(parts, 0);
    numbers
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

fn create_dmg_package(package: &PackageContext<'_>, output: &Path) -> Result<(), String> {
    let application = create_macos_application(package)?;
    run_tool(
        Command::new("hdiutil")
            .args(["create", "-ov", "-format", "UDZO", "-volname"])
            .arg(&package.application.name)
            .arg("-srcfolder")
            .arg(&application)
            .arg(output),
        "hdiutil",
    )?;
    if package.sign {
        sign_macos_path(output, None)?;
        notarize_macos(output)?;
    }
    Ok(())
}

fn create_pkg_package(package: &PackageContext<'_>, output: &Path) -> Result<(), String> {
    let application = create_macos_application(package)?;
    let root = package.workspace.join("pkg-root");
    let applications = root.join("Applications");
    fs::create_dir_all(&applications).map_err(|error| error.to_string())?;
    hardlink_or_copy(
        &application,
        &applications.join(application.file_name().unwrap_or_default()),
    )?;
    let mut command = Command::new("pkgbuild");
    command.arg("--root").arg(&root).args([
        "--identifier",
        &package.application.identifier,
        "--version",
        &package.application.version,
    ]);
    if package.sign {
        command.args([
            "--sign",
            &require_variable("PAM_MACOS_INSTALLER_IDENTITY", "signed PKG builds")?,
        ]);
    }
    run_tool(command.arg(output), "pkgbuild")?;
    if package.sign {
        notarize_macos(output)?;
    }
    Ok(())
}

fn create_msix_package(package: &PackageContext<'_>, output: &Path) -> Result<(), String> {
    if icon_extension(package.icon)? != "png" {
        return Err("MSIX packaging requires a PNG application icon".to_owned());
    }
    if package.application.identifier.len() > 50 {
        return Err("MSIX application identifiers must contain at most 50 characters".to_owned());
    }
    let root = package.workspace.join("msix-root");
    let assets = root.join("Assets");
    fs::create_dir_all(&assets).map_err(|error| error.to_string())?;
    hardlink_or_copy(package.bundle, &root.join("runtime"))?;
    create_windows_assets(package.icon, &assets)?;
    let publisher = std::env::var("PAM_WINDOWS_PUBLISHER")
        .unwrap_or_else(|_| "CN=Pam Desktop Development".to_owned());
    fs::write(
        root.join("AppxManifest.xml"),
        windows_appx_manifest(package.application, package.names, &publisher),
    )
    .map_err(|error| format!("cannot write AppxManifest.xml: {error}"))?;
    run_tool(
        Command::new("makeappx.exe")
            .args(["pack", "/o", "/d"])
            .arg(&root)
            .arg("/p")
            .arg(output),
        "makeappx.exe",
    )?;
    if package.sign {
        sign_windows_path(output)?;
    }
    Ok(())
}

fn create_windows_assets(icon: &Path, assets: &Path) -> Result<(), String> {
    run_tool(
        Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Add-Type -AssemblyName System.Drawing; $source=[System.Drawing.Image]::FromFile($env:PAM_ICON_SOURCE); foreach($asset in @(@('StoreLogo.png',50),@('Square44x44Logo.png',44),@('Square150x150Logo.png',150))){$bitmap=New-Object System.Drawing.Bitmap($asset[1],$asset[1]);$graphics=[System.Drawing.Graphics]::FromImage($bitmap);$graphics.DrawImage($source,0,0,$asset[1],$asset[1]);$bitmap.Save((Join-Path $env:PAM_ICON_OUTPUT $asset[0]),[System.Drawing.Imaging.ImageFormat]::Png);$graphics.Dispose();$bitmap.Dispose()};$source.Dispose()",
            ])
            .env("PAM_ICON_SOURCE", icon)
            .env("PAM_ICON_OUTPUT", assets),
        "the Windows icon renderer",
    )
}

fn windows_architecture(architecture: &str) -> &str {
    match architecture {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        value => value,
    }
}

fn windows_appx_manifest(
    application: &Application,
    names: &ArtifactNames,
    publisher: &str,
) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
         xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
         xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
         IgnorableNamespaces="uap rescap">
  <Identity Name="{identifier}" Publisher="{signer}" Version="{version}" ProcessorArchitecture="{architecture}"/>
  <Properties>
    <DisplayName>{name}</DisplayName>
    <PublisherDisplayName>{publisher}</PublisherDisplayName>
    <Logo>Assets\StoreLogo.png</Logo>
    <Description>{description}</Description>
  </Properties>
  <Resources><Resource Language="en-us"/></Resources>
  <Dependencies><TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.17763.0" MaxVersionTested="10.0.26100.0"/></Dependencies>
  <Applications>
    <Application Id="App" Executable="runtime\bin\{executable}.exe" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements DisplayName="{name}" Description="{description}" BackgroundColor="transparent"
          Square44x44Logo="Assets\Square44x44Logo.png" Square150x150Logo="Assets\Square150x150Logo.png"/>
{protocols}    </Application>
  </Applications>
  <Capabilities><rescap:Capability Name="runFullTrust"/></Capabilities>
</Package>
"#,
        identifier = xml_escape(&application.identifier),
        signer = xml_escape(publisher),
        version = numeric_version(&application.version, 4),
        architecture = windows_architecture(&names.architecture),
        name = xml_escape(&application.name),
        publisher = xml_escape(&application.publisher),
        description = xml_escape(package_description(application)),
        executable = xml_escape(&names.executable),
        protocols = windows_appx_protocols(application),
    )
}

/// The `windows.protocol` extensions: Windows starts the application with
/// the link as its argument.
fn windows_appx_protocols(application: &Application) -> String {
    if application.url_schemes.is_empty() {
        return String::new();
    }
    let extensions: String = application
        .url_schemes
        .iter()
        .map(|scheme| format!("        <uap:Extension Category=\"windows.protocol\"><uap:Protocol Name=\"{}\"/></uap:Extension>\n", xml_escape(scheme)))
        .collect();
    format!("      <Extensions>\n{extensions}      </Extensions>\n")
}

fn create_msi_package(package: &PackageContext<'_>, output: &Path) -> Result<(), String> {
    let architecture = match package.names.architecture.as_str() {
        architecture @ ("x86_64" | "aarch64") => windows_architecture(architecture),
        value => return Err(format!("unsupported MSI architecture {value:?}")),
    };
    let source = package.workspace.join("pam-desktop.wxs");
    fs::write(
        &source,
        windows_wix_source(package.bundle, package.application, package.names),
    )
    .map_err(|error| format!("cannot write the WiX source: {error}"))?;
    run_tool(
        Command::new("wix")
            .args(["build", "-arch", architecture])
            .arg("-o")
            .arg(output)
            .arg(&source),
        "wix",
    )?;
    if package.sign {
        sign_windows_path(output)?;
    }
    Ok(())
}

fn windows_wix_source(bundle: &Path, application: &Application, names: &ArtifactNames) -> String {
    // Each scheme is registered for the user, pointing at the launcher.
    let mut components = String::new();
    let mut references = String::new();
    for (index, scheme) in application.url_schemes.iter().enumerate() {
        let scheme = xml_escape(scheme);
        components.push_str(&format!(
            r#"    <Component Id="UrlScheme{index}" Directory="INSTALLFOLDER">
      <RegistryKey Root="HKCU" Key="Software\Classes\{scheme}">
        <RegistryValue Type="string" Value="URL:{name}" KeyPath="yes" />
        <RegistryValue Type="string" Name="URL Protocol" Value="" />
        <RegistryKey Key="shell\open\command">
          <RegistryValue Type="string" Value="&quot;[INSTALLFOLDER]bin\{executable}.exe&quot; &quot;%1&quot;" />
        </RegistryKey>
      </RegistryKey>
    </Component>
"#,
            name = xml_escape(&application.name),
            executable = xml_escape(&names.executable),
        ));
        references.push_str(&format!("<ComponentRef Id=\"UrlScheme{index}\" />"));
    }
    format!(
        r#"<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs">
  <Package Name="{}" Manufacturer="{}" Version="{}" UpgradeCode="{}" Scope="perUser">
    <MajorUpgrade DowngradeErrorMessage="A newer version is already installed." />
    <MediaTemplate EmbedCab="yes" />
    <StandardDirectory Id="LocalAppDataFolder">
      <Directory Id="INSTALLFOLDER" Name="{}">
        <Files Include="{}\**" />
      </Directory>
    </StandardDirectory>
{components}    <Feature Id="Main"><FilesRef Id="INSTALLFOLDER" />{references}</Feature>
  </Package>
</Wix>
"#,
        xml_escape(&application.name),
        xml_escape(&application.publisher),
        numeric_version(&application.version, 4),
        deterministic_windows_guid(&application.identifier),
        xml_escape(&names.executable),
        xml_escape(&bundle.display().to_string()),
    )
}

/// The upgrade code of the application: the same for every version, so an
/// installer replaces the previous one.
fn deterministic_windows_guid(identifier: &str) -> String {
    let digest = Sha256::digest(identifier.as_bytes());
    let group = |range: std::ops::Range<usize>| hex(&digest[range]).to_ascii_uppercase();
    format!(
        "{}-{}-4{:X}{}-8{:X}{}-{}",
        group(0..4),
        group(4..6),
        digest[6] & 0x0f,
        group(7..8),
        digest[8] & 0x0f,
        group(9..10),
        group(10..16)
    )
}

/// The directories of a bundle that hold code the system must trust.
const SIGNED_DIRECTORIES: [&str; 3] = ["bin", "lib", "runtime"];

fn signable_files(bundle: &Path) -> Result<Vec<PathBuf>, String> {
    Ok(bundle::files(bundle)?
        .into_iter()
        .filter(|(relative, _)| {
            SIGNED_DIRECTORIES
                .iter()
                .any(|directory| Path::new(relative).starts_with(directory))
        })
        .map(|(_, path)| path)
        .collect())
}

fn sign_windows_bundle(bundle: &Path) -> Result<(), String> {
    for path in signable_files(bundle)? {
        let signable = path
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("exe") || extension.eq_ignore_ascii_case("dll")
            });
        if signable {
            sign_windows_path(&path)?;
        }
    }
    Ok(())
}

fn windows_certificate_thumbprint() -> Result<String, String> {
    let thumbprint = require_variable("PAM_WINDOWS_CERTIFICATE_SHA1", "Windows signing")?;
    if thumbprint.len() != 40 || !thumbprint.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(
            "PAM_WINDOWS_CERTIFICATE_SHA1 must be a 40-digit certificate thumbprint".to_owned(),
        );
    }
    Ok(thumbprint)
}

fn sign_windows_path(path: &Path) -> Result<(), String> {
    let thumbprint = windows_certificate_thumbprint()?;
    let timestamp = std::env::var("PAM_WINDOWS_TIMESTAMP_URL")
        .unwrap_or_else(|_| "http://timestamp.digicert.com".to_owned());
    run_tool(
        Command::new("signtool.exe")
            .args(["sign", "/fd", "SHA256", "/td", "SHA256", "/tr"])
            .arg(timestamp)
            .arg("/sha1")
            .arg(thumbprint)
            .arg(path),
        "signtool.exe",
    )
    .map_err(|error| {
        format!(
            "Authenticode signing failed for {}: {error}",
            path.display()
        )
    })
}

fn sign_macos_bundle_binaries(bundle: &Path) -> Result<(), String> {
    let entitlements = bundle
        .parent()
        .ok_or("the macOS bundle has no build workspace")?
        .join("pam-desktop-entitlements.plist");
    fs::write(&entitlements, MACOS_RUNTIME_ENTITLEMENTS)
        .map_err(|error| format!("cannot write the macOS runtime entitlements: {error}"))?;
    for path in signable_files(bundle)? {
        sign_macos_path(&path, Some(&entitlements))?;
    }
    fs::remove_file(&entitlements)
        .map_err(|error| format!("cannot remove the staged macOS runtime entitlements: {error}"))
}

fn sign_macos_path(path: &Path, entitlements: Option<&Path>) -> Result<(), String> {
    let identity = require_variable("PAM_MACOS_SIGNING_IDENTITY", "macOS signing")?;
    let mut command = Command::new("codesign");
    command
        .args(["--force", "--options", "runtime", "--timestamp", "--sign"])
        .arg(identity);
    if let Some(entitlements) = entitlements {
        command.arg("--entitlements").arg(entitlements);
    }
    run_tool(command.arg(path), "codesign")
        .map_err(|error| format!("signing {} failed: {error}", path.display()))
}

/// PHP's JIT writes and runs code at run time, which the hardened runtime
/// only allows with these.
const MACOS_RUNTIME_ENTITLEMENTS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>com.apple.security.cs.allow-jit</key><true/>
  <key>com.apple.security.cs.allow-unsigned-executable-memory</key><true/>
</dict>
</plist>
"#;

/// Notarizes and staples when `PAM_MACOS_NOTARY_PROFILE` names a keychain profile.
fn notarize_macos(path: &Path) -> Result<(), String> {
    let Ok(profile) = std::env::var("PAM_MACOS_NOTARY_PROFILE") else {
        return Ok(());
    };
    run_tool(
        Command::new("xcrun")
            .args(["notarytool", "submit"])
            .arg(path)
            .args(["--keychain-profile", &profile, "--wait"]),
        "Apple notarytool",
    )?;
    run_tool(
        Command::new("xcrun").args(["stapler", "staple"]).arg(path),
        "Apple stapler",
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Mirrors a tree of the staged bundle with hard links, copying where the
/// file system has none.
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
            "unexpected symlink in the materialized bundle: {}",
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

/// Directories and what sits in a `bin` directory (or ends in `.sh`) are
/// 0755, every other file 0644: the modes of the build machine do not leak
/// into archives.
fn normalize_permissions(root: &Path) -> Result<(), String> {
    #[cfg(not(unix))]
    let _ = root;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fn visit(root: &Path, path: &Path) -> Result<(), String> {
            let metadata = fs::symlink_metadata(path)
                .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
            if metadata.file_type().is_symlink() {
                return Ok(());
            }
            let mode = if metadata.is_dir() {
                for entry in fs::read_dir(path)
                    .map_err(|error| format!("cannot read {}: {error}", path.display()))?
                {
                    visit(root, &entry.map_err(|error| error.to_string())?.path())?;
                }
                0o755
            } else {
                let relative = path.strip_prefix(root).map_err(|error| error.to_string())?;
                let in_bin = relative.parent().is_some_and(|parent| {
                    parent
                        .components()
                        .any(|component| component.as_os_str() == "bin")
                });
                if in_bin || path.extension() == Some(OsStr::new("sh")) {
                    0o755
                } else {
                    0o644
                }
            };
            fs::set_permissions(path, fs::Permissions::from_mode(mode))
                .map_err(|error| format!("cannot set the mode of {}: {error}", path.display()))
        }

        visit(root, root)?;
    }
    Ok(())
}

const fn debian_section(category: Category) -> &'static str {
    match category {
        Category::Development => "devel",
        Category::Productivity => "office",
        Category::Graphics => "graphics",
        Category::AudioVideo => "sound",
        Category::Network => "net",
        Category::Utility => "utils",
        Category::Game => "games",
        Category::Education => "education",
    }
}

fn debian_architecture(architecture: &str) -> Result<&'static str, String> {
    match architecture {
        "x86_64" => Ok("amd64"),
        "aarch64" => Ok("arm64"),
        _ => Err(format!(
            "Debian packaging does not map the architecture {architecture:?}"
        )),
    }
}

/// `SOURCE_DATE_EPOCH`, the timestamp of every archived file; zero by default.
fn source_date_epoch() -> Result<u64, String> {
    match std::env::var("SOURCE_DATE_EPOCH") {
        Ok(value) => value
            .parse()
            .map_err(|_| "SOURCE_DATE_EPOCH must be an unsigned integer".to_owned()),
        Err(_) => Ok(DEFAULT_SOURCE_DATE_EPOCH),
    }
}

fn output_directory(project: &Project, options: &BuildOptions) -> Result<PathBuf, String> {
    let path = options
        .output
        .clone()
        .unwrap_or_else(|| project.root.join("dist"));
    fs::create_dir_all(&path).map_err(|error| {
        format!(
            "cannot create the output directory {}: {error}",
            path.display()
        )
    })?;
    path.canonicalize().map_err(|error| {
        format!(
            "cannot resolve the output directory {}: {error}",
            path.display()
        )
    })
}

fn validated_binary(path: &Path, label: &str) -> Result<PathBuf, String> {
    let resolved = path
        .canonicalize()
        .map_err(|error| format!("cannot resolve the {label} {}: {error}", path.display()))?;
    if !resolved.is_file() {
        return Err(format!("the {label} is not a file: {}", resolved.display()));
    }
    Ok(resolved)
}

/// A program given as a path, or as a name to look up on `PATH`.
fn resolve_executable(program: &Path, search_path: &OsStr) -> Result<PathBuf, String> {
    if program.is_absolute() || program.components().count() > 1 {
        // Not resolved further: the bundle names the runtime as it was found.
        return if program.is_file() {
            Ok(program.to_path_buf())
        } else {
            Err(format!(
                "the PHP runtime {} is not a file",
                program.display()
            ))
        };
    }
    std::env::split_paths(search_path)
        .map(|directory| directory.join(program))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            format!(
                "cannot find {} on PATH; install PHP or set PAM_DESKTOP_PHP",
                program.display()
            )
        })
}

fn make_executable(path: &Path) -> Result<(), String> {
    #[cfg(not(unix))]
    let _ = path;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("cannot mark {} executable: {error}", path.display()))?;
    }
    Ok(())
}

/// Prints the options of `build`.
pub fn print_build_usage() {
    println!(
        "usage: pam-desktop build [app] [--output DIR] [--format directory|portable|deb|appimage|rpm|dmg|pkg|msix|msi|all]... [--sign] [--force]\n\n\
         Default formats: directory and portable.\n\
         Linux builds AppImage, DEB and RPM; macOS builds DMG and PKG; Windows builds MSIX and MSI.\n\
         --sign reads the credentials of the system from PAM_MACOS_* or PAM_WINDOWS_* environment variables."
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use pam_desktop_ui::modules::test_support::temporary_directory;

    fn parse(arguments: &[&str]) -> Result<BuildOptions, String> {
        match BuildOptions::parse(arguments.iter().map(OsString::from))? {
            BuildCommand::Build(options) => Ok(options),
            BuildCommand::Help => Err("help".to_owned()),
        }
    }

    #[test]
    fn parses_build_formats_and_replacement_policy() {
        let options = parse(&[
            "project", "--format", "deb", "--format", "portable", "--force", "-o", "out",
        ])
        .unwrap();
        assert_eq!(options.project, PathBuf::from("project"));
        assert_eq!(options.output, Some(PathBuf::from("out")));
        assert_eq!(
            options.formats,
            BTreeSet::from([PackageFormat::Portable, PackageFormat::Debian])
        );
        assert!(options.force && !options.sign);

        let defaults = parse(&[]).unwrap();
        assert_eq!(defaults.project, PathBuf::from("."));
        assert_eq!(
            defaults.formats,
            BTreeSet::from([PackageFormat::Directory, PackageFormat::Portable])
        );
        assert!(!defaults.force);
    }

    #[test]
    fn parses_every_installer_format() {
        let options = parse(&[
            "--format", "appimage", "--format", "rpm", "--format", "dmg", "--format", "pkg",
            "--format", "msix", "--format", "msi", "--sign",
        ])
        .unwrap();
        for format in [
            PackageFormat::AppImage,
            PackageFormat::Rpm,
            PackageFormat::Dmg,
            PackageFormat::Pkg,
            PackageFormat::Msix,
            PackageFormat::Msi,
        ] {
            assert!(options.formats.contains(&format), "{format:?}");
        }
        assert!(options.sign);
        let all = parse(&["--format", "all"]).unwrap();
        assert!(
            all.formats.contains(&PackageFormat::Directory)
                && all.formats.contains(&PackageFormat::Portable)
        );
        assert_eq!(
            all.formats.contains(&PackageFormat::Debian),
            cfg!(target_os = "linux")
        );
        assert_eq!(
            all.formats.contains(&PackageFormat::Dmg),
            cfg!(target_os = "macos")
        );
        assert_eq!(
            all.formats.contains(&PackageFormat::Msi),
            cfg!(target_os = "windows")
        );

        assert!(
            parse(&["--format", "floppy"])
                .unwrap_err()
                .contains("unknown package format")
        );
        assert!(
            parse(&["--format"])
                .unwrap_err()
                .contains("requires a value")
        );
        assert!(
            parse(&["--fast"])
                .unwrap_err()
                .contains("unknown build option")
        );
        assert!(parse(&["one", "two"]).unwrap_err().contains("only one"));
        assert!(parse(&["-o", "a", "-o", "b"]).unwrap_err().contains("once"));
        assert!(matches!(
            BuildOptions::parse([OsString::from("--help")]),
            Ok(BuildCommand::Help)
        ));
    }

    #[test]
    fn installers_of_other_systems_are_refused() {
        let foreign = if cfg!(target_os = "linux") {
            "msi"
        } else {
            "deb"
        };
        assert!(
            validate_platform_formats(&parse(&["--format", foreign]).unwrap())
                .unwrap_err()
                .contains("only available")
        );
        assert!(validate_platform_formats(&parse(&["--format", "directory"]).unwrap()).is_ok());
        if cfg!(target_os = "linux") {
            assert!(
                validate_platform_formats(&parse(&["--sign"]).unwrap())
                    .unwrap_err()
                    .contains("--sign")
            );
        }
    }

    #[test]
    fn produces_stable_windows_installer_identity() {
        let first = deterministic_windows_guid("com.pushin.studio");
        assert_eq!(first, deterministic_windows_guid("com.pushin.studio"));
        assert_ne!(first, deterministic_windows_guid("com.pushin.other"));
        assert_eq!(first.len(), 36);
        let groups: Vec<&str> = first.split('-').collect();
        assert_eq!(
            groups.iter().map(|group| group.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert!(groups[2].starts_with('4') && groups[3].starts_with('8'));
        assert!(first.chars().all(|character| character == '-'
            || character.is_ascii_digit()
            || ('A'..='F').contains(&character)));
    }

    #[test]
    fn versions_are_reduced_to_the_numbers_installers_accept() {
        assert_eq!(numeric_version("1.4.2-rc.3", 3), "1.4.2");
        assert_eq!(numeric_version("2", 3), "2.0.0");
        assert_eq!(numeric_version("1.2.3", 4), "1.2.3.0");
        assert_eq!(numeric_version("1.99999.0", 3), "1.65535.0");
    }

    #[test]
    fn leaves_development_files_out() {
        for path in [
            ".git/config",
            ".env",
            ".env.production",
            "config/.env.local",
            "node_modules/left-pad/index.js",
            "dist/app.tar.gz",
            "target/debug/app",
            ".pam-desktop/state/navigation.json",
            ".pam/cache",
            ".pamignore",
            "vendor/acme/package/vendor/dev-only.php",
            "examples/app/vendor/acme/package/vendor/dev-only.php",
        ] {
            assert!(excluded_path(Path::new(path), &[]), "{path}");
        }
        for path in [
            "index.php",
            "vendor/autoload.php",
            "vendor/acme/package/src/Vendor.php",
            "environment.php",
            "src/target.php",
            "plugins/system/src/main.rs",
        ] {
            assert!(!excluded_path(Path::new(path), &[]), "{path}");
        }
        let configured = [PathBuf::from("storage/cache"), PathBuf::from("notes.md")];
        assert!(excluded_path(Path::new("storage/cache"), &configured));
        assert!(excluded_path(Path::new("storage/cache/item"), &configured));
        assert!(excluded_path(Path::new("notes.md"), &configured));
        assert!(!excluded_path(Path::new("storage/cache-warm"), &configured));
        assert!(!excluded_path(Path::new("storage/data"), &configured));
    }

    #[test]
    fn validates_self_contained_application_icons() {
        assert!(validate_svg_icon(DEFAULT_ICON.as_bytes()).is_ok());
        assert!(validate_svg_icon(b"<svg><script>alert(1)</script></svg>").is_err());
        assert!(
            validate_svg_icon(b"<svg><image href=\"https://example.com/icon.png\"/></svg>")
                .is_err()
        );
        assert!(validate_svg_icon(b"\xff\xfe").is_err());

        let mut png = Vec::from(*b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0@\0\0\0@");
        assert!(validate_png_icon(&png).is_ok());
        png[23] = 32;
        assert!(validate_png_icon(&png).is_err());
        assert!(validate_png_icon(b"\x89PNG").is_err());
    }

    #[test]
    fn writes_freedesktop_entries() {
        let fixture = Fixture::create("desktop-entry");
        let project = Project::discover(&fixture.project).unwrap();
        let entry = desktop_entry(&project.application, "/opt/com.pushin.hello/bin/hello");
        assert!(entry.starts_with("[Desktop Entry]\nType=Application\n"));
        assert!(entry.contains("Name=Pam Hello\n"));
        assert!(entry.contains("Comment=A package fixture\\; with a semicolon.\n"));
        assert!(entry.contains("Exec=/opt/com.pushin.hello/bin/hello %u\n"));
        assert!(entry.contains("MimeType=x-scheme-handler/hello;x-scheme-handler/pam-hello;\n"));
        let mut plain = project.application.clone();
        plain.url_schemes.clear();
        assert!(!desktop_entry(&plain, "hello").contains("MimeType"));
        assert!(entry.contains("Icon=com.pushin.hello\n"));
        assert!(entry.contains("Categories=Development;\n"));
        assert!(entry.contains("StartupWMClass=com.pushin.hello\n"));
    }

    #[test]
    fn url_schemes_reach_the_installers_of_every_system() {
        let fixture = Fixture::create("url-schemes");
        let project = Project::discover(&fixture.project).unwrap();
        let application = &project.application;
        let names = ArtifactNames::new(application).unwrap();

        let plist = macos_info_plist(application, "hello", "AppIcon.svg");
        assert!(plist.contains("<key>CFBundleURLTypes</key>"));
        assert!(plist.contains("<key>CFBundleURLName</key><string>com.pushin.hello</string>"));
        assert!(plist.contains("<key>CFBundleURLSchemes</key><array><string>hello</string><string>pam-hello</string></array>"));
        assert!(plist.contains("<key>CFBundleShortVersionString</key><string>0.4.0</string>"));

        let appx = windows_appx_manifest(application, &names, "CN=Pushin");
        assert!(appx.contains("<uap:Extension Category=\"windows.protocol\"><uap:Protocol Name=\"hello\"/></uap:Extension>"));
        assert!(appx.contains("<uap:Protocol Name=\"pam-hello\"/>"));
        assert!(appx.contains("Executable=\"runtime\\bin\\hello.exe\""));

        let wix = windows_wix_source(Path::new("C:\\build\\hello"), application, &names);
        assert!(wix.contains("Key=\"Software\\Classes\\hello\""));
        assert!(wix.contains("Key=\"Software\\Classes\\pam-hello\""));
        assert!(wix.contains("&quot;[INSTALLFOLDER]bin\\hello.exe&quot; &quot;%1&quot;"));
        assert!(
            wix.contains("<ComponentRef Id=\"UrlScheme0\" /><ComponentRef Id=\"UrlScheme1\" />")
        );

        let mut plain = application.clone();
        plain.url_schemes.clear();
        assert!(!macos_info_plist(&plain, "hello", "AppIcon.svg").contains("CFBundleURLTypes"));
        assert!(!windows_appx_manifest(&plain, &names, "CN=Pushin").contains("Extensions"));
        assert!(
            !windows_wix_source(Path::new("C:\\build\\hello"), &plain, &names)
                .contains("Component")
        );
    }

    #[test]
    fn web_view_use_is_found_in_application_sources_only() {
        let project = std::env::temp_dir().join(format!("pam-webview-use-{}", std::process::id()));
        let _ = fs::remove_dir_all(&project);
        fs::create_dir_all(project.join("vendor/pushinbr/pam-desktop/src/UI")).unwrap();
        fs::create_dir_all(project.join("src/Screens")).unwrap();
        fs::write(
            project.join("vendor/pushinbr/pam-desktop/src/UI/WebView.php"),
            "<?php final class WebView {}",
        )
        .unwrap();
        fs::write(project.join("index.php"), "<?php Text::make('hello');").unwrap();
        assert!(!uses_web_view(&project));
        fs::write(
            project.join("src/Screens/Help.php"),
            "<?php WebView::make('https://example.com');",
        )
        .unwrap();
        assert!(uses_web_view(&project));
        fs::remove_dir_all(&project).unwrap();
    }

    #[test]
    fn platform_libraries_become_package_dependencies() {
        let names: Vec<OsString> = ["libgtk-4.so.1", "libgstplay-1.0.so.0", "libz.so.1"]
            .into_iter()
            .map(OsString::from)
            .collect();
        let dependencies = system_dependencies(&names, true);
        assert_eq!(
            dependencies.debian,
            [
                "libc6",
                "libgtk-4-1",
                "libwebkitgtk-6.0-4",
                "gstreamer1.0-plugins-base",
                "gstreamer1.0-plugins-good"
            ]
        );
        assert_eq!(
            dependencies.rpm,
            [
                "glibc",
                "gtk4",
                "webkitgtk6.0",
                "gstreamer1-plugins-base",
                "gstreamer1-plugins-good"
            ]
        );
        assert!(
            !system_dependencies(&names, false)
                .debian
                .contains(&"libwebkitgtk-6.0-4")
        );
        assert_eq!(
            system_dependencies(&[], false),
            SystemDependencies {
                debian: vec!["libc6"],
                rpm: vec!["glibc"]
            }
        );

        assert!(
            platform_root("libgtk-4.so.1")
                && platform_root("libgstreamer-1.0.so.0")
                && platform_root("libglib-2.0.so.0")
        );
        assert!(!platform_root("libsqlite3.so.0") && !platform_root("libgtk-3.so.0"));
        assert!(
            system_abi_library("libc.so.6")
                && system_abi_library("ld-linux-x86-64.so.2")
                && !system_abi_library("libcurl.so.4")
        );
    }

    #[test]
    fn launchers_quote_their_paths() {
        let script = launcher_script("app/my \"app\"/$HOME").unwrap();
        assert!(script.contains("cd \"$PAM_BUNDLE/app/my \\\"app\\\"/\\$HOME\"\n"));
        assert!(script.contains(
            "exec \"$PAM_BUNDLE/bin/pam-desktop\" run \"$PAM_BUNDLE/app/my \\\"app\\\"/\\$HOME\"\n"
        ));
        assert!(script.contains(&format!(
            "export PAM_DESKTOP_PHP=\"$PAM_BUNDLE/bin/{PHP_WRAPPER}\"\n"
        )));
        assert!(!script.contains('@'));
        assert!(launcher_script("app/a\nb").is_err());
    }

    #[test]
    fn builds_a_materialized_directory_with_integrity_metadata() {
        let fixture = Fixture::create("directory");
        let result = fixture
            .build(&["--format", "directory"])
            .expect("bundle should build");
        let exe = std::env::consts::EXE_SUFFIX;
        let bundle = &result.artifacts[0];
        assert_eq!(
            bundle,
            &fixture.output.join(format!(
                "hello-0.4.0-{}-{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            ))
        );
        assert!(bundle.join(format!("bin/pam-desktop{exe}")).is_file());
        assert!(bundle.join(format!("bin/hello{exe}")).is_file());
        assert!(bundle.join("runtime/php/bin/php").is_file());
        assert_eq!(bundle.join("bin/pam-desktop-php").is_file(), cfg!(unix));
        assert!(bundle.join("app/index.php").is_file());
        assert!(bundle.join("app/vendor/autoload.php").is_file());
        assert!(
            bundle
                .join("app/vendor/pushinbr/pam-desktop/packages/desktop/bin/pam-desktop-worker")
                .is_file()
        );
        assert!(
            !bundle
                .join("app/vendor/pushinbr/pam-desktop/vendor")
                .exists()
        );
        assert!(
            !fs::read_to_string(bundle.join("app/composer.json"))
                .unwrap()
                .contains("/private/source")
        );
        assert!(
            !fs::read_to_string(bundle.join("app/composer.lock"))
                .unwrap()
                .contains("/private/source")
        );
        assert!(!bundle.join("app/.env").exists());
        assert!(!bundle.join("app/.pam-desktop").exists());
        assert!(!bundle.join("app/storage/cache").exists());
        assert!(bundle.join("app/storage/data.txt").is_file());
        assert!(
            !bundle.join("app/artifacts").exists(),
            "the output directory must not leak into its own bundle"
        );
        assert!(
            bundle
                .join("share/icons/hicolor/scalable/apps/com.pushin.hello.svg")
                .is_file()
        );
        // The manifest was complete: it ships as written.
        assert_eq!(
            fs::read(bundle.join("app/pam-desktop.json")).unwrap(),
            fs::read(fixture.project.join("pam-desktop.json")).unwrap()
        );

        let ini = fs::read_to_string(bundle.join("etc/php.ini")).unwrap();
        assert!(ini.contains("expose_php=Off\n") && ini.contains("memory_limit=512M\n"));
        assert!(
            !ini.contains("extension"),
            "a runtime without shared extensions loads none"
        );

        let manifest = bundle::verify(bundle).expect("the bundle verifies against its manifest");
        assert_eq!(manifest.schema_version, bundle::SCHEMA_VERSION);
        assert_eq!(manifest.application.identifier, "com.pushin.hello");
        assert_eq!(manifest.application.version, "0.4.0");
        assert_eq!(manifest.application.category, "development");
        assert_eq!(
            manifest
                .application
                .updates
                .as_ref()
                .map(|updates| updates.policy.as_str()),
            Some("notify")
        );
        assert_eq!(manifest.project, "app");
        assert_eq!(manifest.entry, "app/index.php");
        assert_eq!(manifest.launcher, format!("bin/hello{exe}"));
        assert_eq!(manifest.runtime.pam_desktop, env!("CARGO_PKG_VERSION"));
        assert_eq!(manifest.runtime.php_binary, "php");
        assert_eq!(manifest.runtime.php, "unknown");
        assert_eq!(manifest.target, BundleTarget::current());
        assert_eq!(manifest.source_date_epoch, 0);
        assert!(
            manifest
                .files
                .iter()
                .any(|file| file.path == "app/index.php")
        );
        assert!(
            manifest
                .files
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &str| {
                fs::metadata(bundle.join(path))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777
            };
            assert_eq!(mode("bin/hello"), 0o755);
            assert_eq!(mode("bin/pam-desktop"), 0o755);
            assert_eq!(mode("runtime/php/bin/php"), 0o755);
            assert_eq!(mode("install.sh"), 0o755);
            assert_eq!(mode("app/index.php"), 0o644);
            assert_eq!(mode("app/secret-mode.txt"), 0o644);
            assert_eq!(mode("manifest.json"), 0o644);
            assert_eq!(mode("app"), 0o755);

            // The launcher finds its bundle through a link and hands the
            // bundled PHP and the entry script to the host.
            let linked_bin = fixture.root.join("linked-bin");
            fs::create_dir(&linked_bin).unwrap();
            std::os::unix::fs::symlink(bundle.join("bin/hello"), linked_bin.join("hello")).unwrap();
            let output = Command::new(linked_bin.join("hello"))
                .arg("hello://open")
                .env_remove("PAM_DESKTOP_LAUNCH_URL")
                .output()
                .expect("the launcher runs");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let bundle_path = bundle.display();
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                format!(
                    "run {bundle_path}/app\nphp={bundle_path}/bin/pam-desktop-php\nroot={bundle_path}\nlauncher={bundle_path}/bin/hello\nurl=hello://open\ncwd={bundle_path}/app\n"
                )
            );
            // The wrapper starts the real PHP with the bundled configuration.
            let output = Command::new(bundle.join("bin/pam-desktop-php"))
                .args(["worker", "entry"])
                .output()
                .expect("the PHP wrapper runs");
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                format!(
                    "php worker entry\nPHPRC={bundle_path}/etc/php.ini\nscan=\nextensions={bundle_path}/runtime/php/ext\n"
                )
            );
        }
    }

    #[test]
    fn the_bundle_starts_with_the_entry_the_build_was_given() {
        let fixture = Fixture::create("entry");
        fs::create_dir_all(fixture.project.join("src")).unwrap();
        fs::write(fixture.project.join("src/main.php"), "<?php\n").unwrap();
        fs::write(fixture.project.join("other.php"), "<?php\n").unwrap();
        fs::write(
            fixture.project.join("pam-desktop.json"),
            r#"{"applicationId": "com.pushin.hello", "version": "0.4.0", "entry": "src/main.php"}"#,
        )
        .unwrap();

        // A declared entry ships as declared: the host reads it at run time.
        let declared = fixture
            .build(&["--format", "directory"])
            .expect("bundle of the declared entry");
        let bundle = &declared.artifacts[0];
        let manifest = bundle::verify(bundle).unwrap();
        assert_eq!(
            (manifest.project.as_str(), manifest.entry.as_str()),
            ("app", "app/src/main.php")
        );
        assert_eq!(
            fs::read(bundle.join("app/pam-desktop.json")).unwrap(),
            fs::read(fixture.project.join("pam-desktop.json")).unwrap()
        );

        // A script given to the build becomes the entry of the bundle.
        let script = fixture.project.join("other.php");
        let options = parse(&[
            script.to_str().unwrap(),
            "--output",
            fixture.output.to_str().unwrap(),
            "--format",
            "directory",
            "--force",
        ])
        .unwrap();
        let given = build_with(&options, &fixture.toolchain()).expect("bundle of the given script");
        let bundle = &given.artifacts[0];
        assert_eq!(bundle::verify(bundle).unwrap().entry, "app/other.php");
        let written: serde_json::Value =
            serde_json::from_slice(&fs::read(bundle.join("app/pam-desktop.json")).unwrap())
                .unwrap();
        assert_eq!(written["entry"], "other.php");
        assert_eq!(written["applicationId"], "com.pushin.hello");
    }

    #[test]
    fn existing_artifacts_need_force() {
        let fixture = Fixture::create("force");
        let first = fixture
            .build(&["--format", "directory"])
            .expect("first build");
        fs::write(first.artifacts[0].join("stale.txt"), b"stale").unwrap();
        let error = fixture
            .build(&["--format", "directory"])
            .err()
            .expect("an existing artifact stops the build");
        assert!(
            error.contains("already exists") && error.contains("--force"),
            "{error}"
        );
        assert!(
            first.artifacts[0].join("stale.txt").is_file(),
            "a refused build touches nothing"
        );

        let second = fixture
            .build(&["--format", "directory", "--force"])
            .expect("forced build");
        assert_eq!(second.artifacts, first.artifacts);
        assert!(!second.artifacts[0].join("stale.txt").exists());
        assert!(bundle::verify(&second.artifacts[0]).is_ok());
        fixture.assert_no_workspace_left();
    }

    #[test]
    fn creates_reproducible_linux_archives_without_bundling_the_output() {
        if !cfg!(target_os = "linux") {
            return;
        }
        let fixture = Fixture::create("portable");
        let first = fixture
            .build(&["--format", "portable"])
            .expect("first portable archive should build");
        assert_eq!(
            first.artifacts,
            [fixture.output.join(format!(
                "hello-0.4.0-linux-{}.tar.gz",
                std::env::consts::ARCH
            ))]
        );
        let first_bytes = fs::read(&first.artifacts[0]).unwrap();
        // The gzip header carries no timestamp and no file name.
        assert_eq!(&first_bytes[..4], [0x1f, 0x8b, 8, 0]);
        assert_eq!(&first_bytes[4..8], [0, 0, 0, 0]);
        let listing = Command::new("tar")
            .arg("-tzvf")
            .arg(&first.artifacts[0])
            .env("TZ", "UTC")
            .output()
            .expect("the archive lists");
        assert!(listing.status.success());
        let listing = String::from_utf8_lossy(&listing.stdout);
        let root = format!("hello-0.4.0-linux-{}/", std::env::consts::ARCH);
        assert!(
            listing.lines().all(|line| line.contains(&root)),
            "one root directory"
        );
        assert!(
            listing.contains(&format!("{root}manifest.json"))
                && listing.contains(&format!("{root}app/index.php"))
        );
        assert!(
            !listing.contains("/app/artifacts/"),
            "the output directory must not leak into its own bundle"
        );
        assert!(
            listing
                .lines()
                .all(|line| line.contains(" 0/0 ") && line.contains("1970-01-01")),
            "{listing}"
        );

        // Other modes and timestamps on the build machine change nothing.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        fs::write(fixture.project.join("index.php"), "<?php\n").unwrap();
        let second = fixture
            .build(&["--format", "portable", "--force"])
            .expect("second portable archive should build");
        assert_eq!(first_bytes, fs::read(&second.artifacts[0]).unwrap());
        fixture.assert_no_workspace_left();
    }

    #[test]
    fn a_missing_tool_fails_before_anything_is_published() {
        let fixture = Fixture::create("tools");
        let empty = fixture.root.join("empty-path");
        fs::create_dir(&empty).unwrap();
        let options = parse(&[
            fixture.project.to_str().unwrap(),
            "--output",
            fixture.output.to_str().unwrap(),
            "--format",
            "directory",
            "--format",
            "portable",
        ])
        .unwrap();
        let mut toolchain = fixture.toolchain();
        toolchain.search_path = empty.into_os_string();
        let error = build_with(&options, &toolchain)
            .err()
            .expect("the build stops");
        assert!(
            error.contains("is required") && error.contains("not found on PATH"),
            "{error}"
        );
        assert_eq!(
            fs::read_dir(&fixture.output).unwrap().count(),
            0,
            "nothing was published or left behind"
        );
        assert!(
            find_tool(
                "definitely-not-a-packaging-tool",
                &std::env::var_os("PATH").unwrap_or_default()
            )
            .is_none()
        );
    }

    #[test]
    fn required_and_unsafe_inputs_stop_the_build() {
        let fixture = Fixture::create("inputs");
        let manifest =
            |json: &str| fs::write(fixture.project.join("pam-desktop.json"), json).unwrap();
        let failure = |fixture: &Fixture| {
            fixture
                .build(&["--format", "directory"])
                .err()
                .expect("the build stops")
        };

        manifest(r#"{"applicationId": "com.pushin.hello", "exclude": ["vendor"]}"#);
        assert!(failure(&fixture).contains("cannot be excluded"));
        manifest(r#"{"applicationId": "com.pushin.hello", "exclude": ["vendor/pushinbr"]}"#);
        assert!(failure(&fixture).contains("no PAM Desktop PHP package"));
        manifest(r#"{"applicationId": "Com.Pushin.Hello"}"#);
        assert!(failure(&fixture).contains("applicationId"));
        manifest(r#"{"applicationId": "com.pushin.pam-desktop"}"#);
        assert!(failure(&fixture).contains("cannot end in"));
        manifest(r#"{"applicationId": "com.pushin.hello", "entry": "missing.php"}"#);
        assert!(failure(&fixture).contains("does not exist"));
        manifest(r#"{"applicationId": "com.pushin.hello", "icon": "resources/missing.svg"}"#);
        assert!(failure(&fixture).contains("icon"));
        manifest(r#"{"applicationId": "com.pushin.hello", "extensions": ["imagick"]}"#);
        assert!(
            fixture.build(&["--format", "directory"]).is_ok(),
            "a runtime that cannot be asked ships as it is"
        );
        manifest("{ broken");
        assert!(failure(&fixture).contains("not valid JSON"));

        fs::remove_dir_all(fixture.project.join("vendor")).unwrap();
        manifest(r#"{"applicationId": "com.pushin.hello"}"#);
        assert!(failure(&fixture).contains("composer install"));
        fixture.assert_no_workspace_left();
    }

    #[cfg(unix)]
    #[test]
    fn links_are_materialized_inside_vendor_and_refused_elsewhere() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::create("links");
        let package = fixture.root.join("linked-package");
        fs::create_dir_all(package.join("src")).unwrap();
        fs::create_dir_all(package.join("vendor/dev")).unwrap();
        fs::write(package.join("src/Linked.php"), "<?php\n").unwrap();
        fs::write(package.join("vendor/dev/only.php"), "<?php\n").unwrap();
        fs::create_dir_all(fixture.project.join("vendor/acme")).unwrap();
        symlink(&package, fixture.project.join("vendor/acme/linked")).unwrap();
        symlink(
            fixture.project.join("index.php"),
            fixture.project.join("alias.php"),
        )
        .unwrap();

        let result = fixture
            .build(&["--format", "directory"])
            .expect("links inside the rules are copied");
        let bundle = &result.artifacts[0];
        let copied = bundle.join("app/vendor/acme/linked/src/Linked.php");
        assert!(
            copied.is_file()
                && !fs::symlink_metadata(bundle.join("app/vendor/acme/linked"))
                    .unwrap()
                    .file_type()
                    .is_symlink()
        );
        assert!(!bundle.join("app/vendor/acme/linked/vendor").exists());
        assert!(
            fs::symlink_metadata(bundle.join("app/alias.php"))
                .unwrap()
                .file_type()
                .is_file()
        );
        assert!(bundle::verify(bundle).is_ok());

        symlink(&package, fixture.project.join("outside")).unwrap();
        let error = fixture
            .build(&["--format", "directory", "--force"])
            .err()
            .expect("a link out of the project stops the build");
        assert!(error.contains("escapes the project"), "{error}");
        fs::remove_file(fixture.project.join("outside")).unwrap();
        symlink(&fixture.project, fixture.project.join("storage/loop")).unwrap();
        let error = fixture
            .build(&["--format", "directory", "--force"])
            .err()
            .expect("a cycle stops the build");
        assert!(error.contains("cycle"), "{error}");
        fs::remove_file(fixture.project.join("storage/loop")).unwrap();
        symlink(&fixture.output, fixture.project.join("vendor/acme/output")).unwrap();
        let error = fixture
            .build(&["--format", "directory", "--force"])
            .err()
            .expect("a link into the output stops the build");
        assert!(error.contains("build output"), "{error}");
    }

    #[test]
    fn an_application_of_a_checkout_ships_the_checkout_package() {
        let fixture = Fixture::create("checkout");
        // The layout of this repository: the autoloader and the PHP package
        // at the root, the application below `examples/`.
        let checkout = fixture.root.join("checkout");
        let example = checkout.join("examples/Counter App");
        fs::create_dir_all(&example).unwrap();
        fs::create_dir_all(checkout.join("vendor/composer")).unwrap();
        fs::create_dir_all(checkout.join("packages/desktop/bin")).unwrap();
        fs::create_dir_all(checkout.join("packages/desktop/src")).unwrap();
        fs::create_dir_all(checkout.join("packages/desktop/tests")).unwrap();
        fs::create_dir_all(checkout.join("crates/host")).unwrap();
        fs::create_dir_all(checkout.join("examples/other")).unwrap();
        fs::write(example.join("index.php"), "<?php\n").unwrap();
        fs::write(checkout.join("examples/other/index.php"), "<?php\n").unwrap();
        fs::write(checkout.join("vendor/autoload.php"), "<?php\n").unwrap();
        fs::write(
            checkout.join("packages/desktop/bin/pam-desktop-worker"),
            "<?php\n",
        )
        .unwrap();
        fs::write(checkout.join("packages/desktop/src/App.php"), "<?php\n").unwrap();
        fs::write(checkout.join("packages/desktop/tests/run.php"), "<?php\n").unwrap();
        fs::write(checkout.join("packages/desktop/composer.json"), "{}\n").unwrap();
        fs::write(checkout.join("crates/host/main.rs"), "fn main() {}\n").unwrap();

        let options = parse(&[example.to_str().unwrap(), "--format", "directory"]).unwrap();
        let result = build_with(&options, &fixture.toolchain()).expect("the example builds");
        let bundle = &result.artifacts[0];
        assert_eq!(
            bundle,
            &example.canonicalize().unwrap().join(format!(
                "dist/app-counter-app-0.0.0-{}-{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            ))
        );
        let app = bundle.join("app");
        assert!(app.join("examples/Counter App/index.php").is_file());
        assert!(app.join("vendor/autoload.php").is_file());
        assert!(
            app.join("packages/desktop/bin/pam-desktop-worker")
                .is_file()
        );
        assert!(app.join("packages/desktop/src/App.php").is_file());
        assert!(app.join("packages/desktop/composer.json").is_file());
        assert!(!app.join("packages/desktop/tests").exists());
        assert!(!app.join("crates").exists());
        assert!(!app.join("examples/other").exists());
        assert!(!app.join("examples/Counter App/dist").exists());

        // Without a manifest, the bundle carries the identity the build chose.
        let written: serde_json::Value = serde_json::from_slice(
            &fs::read(app.join("examples/Counter App/pam-desktop.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            written,
            serde_json::json!({"applicationId": "dev.pam.desktop.app-counter-app", "version": "0.0.0"})
        );
        let manifest = bundle::verify(bundle).unwrap();
        assert_eq!(manifest.project, "app/examples/Counter App");
        assert_eq!(manifest.entry, "app/examples/Counter App/index.php");
        assert_eq!(
            manifest.application.identifier,
            "dev.pam.desktop.app-counter-app"
        );
        assert_eq!(manifest.application.updates, None);
        assert!(
            bundle
                .join("share/icons/hicolor/scalable/apps/dev.pam.desktop.app-counter-app.svg")
                .is_file()
        );
        if cfg!(unix) {
            let launcher = fs::read_to_string(bundle.join("bin/app-counter-app")).unwrap();
            assert!(launcher.contains("cd \"$PAM_BUNDLE/app/examples/Counter App\"\n"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn ships_the_shared_extensions_the_runtime_loads() {
        let fixture = Fixture::create("extensions");
        let extensions = fixture.root.join("php-extensions");
        fs::create_dir(&extensions).unwrap();
        for name in ["ctype", "tokenizer", "intl", "unloaded"] {
            fs::write(extensions.join(format!("{name}.so")), name).unwrap();
        }
        // Core is compiled in; ctype, tokenizer and intl are shared objects.
        fs::write(
            &fixture.php,
            format!(
                "#!/bin/sh\nif [ \"$1\" = \"-r\" ]; then\n  printf '%s' '{{\"version\":\"8.5.1\",\"extensionDir\":\"{}\",\"extensions\":[\"Core\",\"ctype\",\"tokenizer\",\"intl\"]}}'\nfi\n",
                extensions.display()
            ),
        )
        .unwrap();

        let all = fixture
            .build(&["--format", "directory"])
            .expect("bundle with every loaded extension");
        let bundle = &all.artifacts[0];
        for name in ["ctype", "tokenizer", "intl"] {
            assert_eq!(
                fs::read_to_string(bundle.join(format!("runtime/php/ext/{name}.so"))).unwrap(),
                name
            );
        }
        assert!(!bundle.join("runtime/php/ext/unloaded.so").exists());
        let ini = fs::read_to_string(bundle.join("etc/php.ini")).unwrap();
        assert!(ini.ends_with("extension_dir=\"${PAM_DESKTOP_PHP_EXTENSION_DIR}\"\nextension=ctype\nextension=tokenizer\nextension=intl\n"), "{ini}");
        assert_eq!(bundle::verify(bundle).unwrap().runtime.php, "8.5.1");

        fs::write(
            fixture.project.join("pam-desktop.json"),
            r#"{"applicationId": "com.pushin.hello", "version": "0.4.0", "extensions": ["tokenizer", "core", "unloaded"]}"#,
        )
        .unwrap();
        let chosen = fixture
            .build(&["--format", "directory", "--force"])
            .expect("bundle with the listed extensions");
        let bundle = &chosen.artifacts[0];
        let shipped: BTreeSet<String> = fs::read_dir(bundle.join("runtime/php/ext"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            shipped,
            BTreeSet::from(["tokenizer.so".to_owned(), "unloaded.so".to_owned()])
        );
        let ini = fs::read_to_string(bundle.join("etc/php.ini")).unwrap();
        assert!(
            ini.ends_with("extension=tokenizer\nextension=unloaded\n")
                && !ini.contains("extension=ctype"),
            "{ini}"
        );

        fs::write(fixture.project.join("pam-desktop.json"), r#"{"applicationId": "com.pushin.hello", "version": "0.4.0", "extensions": ["imagick"]}"#).unwrap();
        let error = fixture
            .build(&["--format", "directory", "--force"])
            .err()
            .expect("an unavailable extension stops the build");
        assert!(error.contains("\"imagick\" is not available"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn ships_only_the_extensions_the_application_refers_to() {
        let fixture = Fixture::create("used-extensions");
        let extensions = fixture.root.join("php-extensions");
        fs::create_dir(&extensions).unwrap();
        for name in ["ctype", "tokenizer", "intl"] {
            fs::write(extensions.join(format!("{name}.so")), name).unwrap();
        }
        // The second question is which extensions the application uses.
        fs::write(
            &fixture.php,
            format!(
                "#!/bin/sh\nif [ \"$3\" = \"--\" ]; then\n  printf '%s' '{{\"used\":[\"ctype\"]}}'\nelif [ \"$1\" = \"-r\" ]; then\n  printf '%s' '{{\"version\":\"8.5.1\",\"extensionDir\":\"{}\",\"extensions\":[\"Core\",\"ctype\",\"tokenizer\",\"intl\"]}}'\nfi\n",
                extensions.display()
            ),
        )
        .unwrap();
        let shipped = |bundle: &Path| -> BTreeSet<String> {
            fs::read_dir(bundle.join("runtime/php/ext"))
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        };

        let detected = fixture
            .build(&["--format", "directory"])
            .expect("bundle with the used extensions");
        assert_eq!(
            shipped(&detected.artifacts[0]),
            BTreeSet::from(["ctype.so".to_owned()])
        );
        let ini = fs::read_to_string(detected.artifacts[0].join("etc/php.ini")).unwrap();
        assert!(
            ini.ends_with("extension=ctype\n") && !ini.contains("extension=intl"),
            "{ini}"
        );
        assert_eq!(detected.notes.len(), 1);
        assert!(
            detected.notes[0].contains("shipped ctype;")
                && detected.notes[0].contains("tokenizer, intl"),
            "{}",
            detected.notes[0]
        );

        let manifest = |extensions: &str| {
            fs::write(
                fixture.project.join("pam-desktop.json"),
                format!(
                    r#"{{"applicationId": "com.pushin.hello", "version": "0.4.0", "extensions": {extensions}}}"#
                ),
            )
            .unwrap();
        };
        manifest(r#"["auto", "intl"]"#);
        let pinned = fixture
            .build(&["--format", "directory", "--force"])
            .expect("bundle with a pinned extension");
        assert_eq!(
            shipped(&pinned.artifacts[0]),
            BTreeSet::from(["ctype.so".to_owned(), "intl.so".to_owned()])
        );

        manifest(r#"["all"]"#);
        let all = fixture
            .build(&["--format", "directory", "--force"])
            .expect("bundle with every extension");
        assert_eq!(shipped(&all.artifacts[0]).len(), 3);
        assert!(all.notes.is_empty());
    }

    /// The analysis itself, with the PHP of the machine when it has one.
    #[cfg(unix)]
    #[test]
    fn extension_usage_reads_php_sources() {
        let script = EXTENSION_USAGE.trim_start_matches("<?php");
        let loaded = |name: &str| {
            Command::new("php")
                .args(["-r", &format!("exit(extension_loaded('{name}') ? 0 : 1);")])
                .status()
                .is_ok_and(|status| status.success())
        };
        if !["tokenizer", "ctype", "mbstring", "intl", "curl"]
            .into_iter()
            .all(loaded)
        {
            return;
        }
        let root = std::env::temp_dir().join(format!("pam-extension-usage-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("vendor/composer")).unwrap();
        fs::create_dir_all(root.join("vendor/dev/tool")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        // `Locale` is a class of the application, the string is a guard,
        // `->curl_init()` is a method: none of them is the extension.
        fs::write(
            root.join("src/App.php"),
            "<?php\nnamespace App;\nfinal class Locale {}\nfunction f(Locale $l) { return new Locale(); }\nif (extension_loaded('intl')) {}\n$a->curl_init();\necho ctype_digit('1');\n",
        )
        .unwrap();
        fs::write(
            root.join("vendor/composer/installed.json"),
            r#"{"packages": [], "dev-package-names": ["dev/tool"]}"#,
        )
        .unwrap();
        fs::write(
            root.join("vendor/dev/tool/Tool.php"),
            "<?php curl_init(); new \\Collator('en');",
        )
        .unwrap();
        let used = |root: &Path| -> Vec<String> {
            let output = Command::new("php")
                .args(["-r", script, "--"])
                .arg(root)
                .args(["ctype", "mbstring", "intl", "curl"])
                .output()
                .unwrap();
            let answer: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            answer["used"]
                .as_array()
                .unwrap()
                .iter()
                .map(|name| name.as_str().unwrap().to_owned())
                .collect()
        };
        assert_eq!(used(&root), ["ctype"]);

        // A callable in a string, an imported class and a requirement.
        fs::write(
            root.join("src/More.php"),
            "<?php\nnamespace App;\nuse Collator as Sorter;\n$n = array_map('mb_strtolower', $names);\n$s = new Sorter('en');\n",
        )
        .unwrap();
        fs::write(
            root.join("composer.json"),
            r#"{"require": {"ext-curl": "*"}}"#,
        )
        .unwrap();
        assert_eq!(used(&root), ["ctype", "mbstring", "intl", "curl"]);
        fs::remove_dir_all(&root).unwrap();
    }

    /// A project with a vendored PHP package, and scripts standing in for
    /// the host and for PHP.
    struct Fixture {
        root: PathBuf,
        project: PathBuf,
        output: PathBuf,
        host: PathBuf,
        php: PathBuf,
    }

    impl Fixture {
        fn create(name: &str) -> Self {
            let root = temporary_directory(&format!("package-{name}"))
                .canonicalize()
                .expect("temporary directory");
            let project = root.join("project");
            let output = project.join("artifacts");
            let package = project.join("vendor/pushinbr/pam-desktop");
            for directory in [
                "resources",
                "storage/cache",
                ".pam-desktop/state",
                "artifacts",
            ] {
                fs::create_dir_all(project.join(directory)).unwrap();
            }
            fs::create_dir_all(package.join("packages/desktop/bin")).unwrap();
            fs::create_dir_all(package.join("vendor")).unwrap();
            let write = |path: &str, contents: &str| {
                fs::write(project.join(path), contents).expect("fixture file")
            };
            write("index.php", "<?php\n");
            write("secret-mode.txt", "mode\n");
            write(
                "pam-desktop.json",
                r#"{"applicationId": "com.pushin.hello", "name": "Pam Hello", "version": "0.4.0",
                    "description": "A package fixture; with a semicolon.", "publisher": "Pushin", "category": "development",
                    "icon": "resources/icon.svg", "exclude": ["storage/cache"], "urlSchemes": ["hello", "pam-hello"],
                    "updates": {"endpoint": "https://updates.pushin.dev/hello/stable.json", "publicKey": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", "policy": "notify"}}"#,
            );
            write(
                "composer.json",
                "{\"repositories\":[{\"type\":\"path\",\"url\":\"/private/source\"}]}\n",
            );
            write(
                "composer.lock",
                "{\"packages\":[{\"name\":\"pushinbr/pam-desktop\",\"dist\":{\"type\":\"path\",\"url\":\"/private/source\"},\"transport-options\":{\"symlink\":true}}],\"packages-dev\":[]}\n",
            );
            write("vendor/autoload.php", "<?php\n");
            write(
                "vendor/pushinbr/pam-desktop/packages/desktop/bin/pam-desktop-worker",
                "<?php\n",
            );
            write("vendor/pushinbr/pam-desktop/vendor/dev-only.php", "<?php\n");
            write(
                "resources/icon.svg",
                "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 64 64\"></svg>\n",
            );
            write("storage/cache/item", "ignored\n");
            write("storage/data.txt", "kept\n");
            write(".pam-desktop/state/navigation.json", "{}\n");
            write(".env", "SECRET=yes\n");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(
                    project.join("secret-mode.txt"),
                    fs::Permissions::from_mode(0o600),
                )
                .unwrap();
            }
            let host = root.join("pam-desktop");
            let php = root.join("php");
            // The host reports how the launcher started it; PHP how the wrapper did.
            fs::write(
                &host,
                "#!/bin/sh\necho \"$@\"\necho \"php=$PAM_DESKTOP_PHP\"\necho \"root=$PAM_DESKTOP_BUNDLE_ROOT\"\necho \"launcher=$PAM_DESKTOP_LAUNCHER\"\necho \"url=${PAM_DESKTOP_LAUNCH_URL:-}\"\necho \"cwd=$(pwd)\"\n",
            )
            .unwrap();
            fs::write(&php, "#!/bin/sh\nif [ \"$1\" = \"-r\" ]; then exit 1; fi\necho \"php $*\"\necho \"PHPRC=$PHPRC\"\necho \"scan=$PHP_INI_SCAN_DIR\"\necho \"extensions=$PAM_DESKTOP_PHP_EXTENSION_DIR\"\n").unwrap();
            make_executable(&host).unwrap();
            make_executable(&php).unwrap();
            Self {
                root,
                project,
                output,
                host,
                php,
            }
        }

        fn toolchain(&self) -> Toolchain {
            Toolchain {
                host: self.host.clone(),
                php: self.php.clone(),
                php_arguments: Vec::new(),
                search_path: std::env::var_os("PATH").unwrap_or_default(),
            }
        }

        fn build(&self, arguments: &[&str]) -> Result<BuildResult, String> {
            let mut all = vec![
                self.project.to_str().unwrap(),
                "--output",
                self.output.to_str().unwrap(),
            ];
            all.extend(arguments);
            build_with(&parse(&all)?, &self.toolchain())
        }

        fn assert_no_workspace_left(&self) {
            let leftovers: Vec<_> = fs::read_dir(&self.output)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with(".pam-desktop-build-"))
                .collect();
            assert!(leftovers.is_empty(), "{leftovers:?}");
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
