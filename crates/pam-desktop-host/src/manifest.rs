//! `pam-desktop.json`: the application manifest.
//!
//! Running an application never fails because of its manifest: a missing or
//! malformed file means defaults ([`Manifest::load`]). Shipping one does:
//! [`Manifest::application`] validates the identity and the distribution
//! fields the packager and the updater rely on.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use pam_desktop_ui::backend::WindowConfig;
use pam_desktop_ui::geometry::Size;
use pam_desktop_ui::modules::http::parse_url;
use serde_json::{Map, Value};

use crate::paths;

pub const FILE_NAME: &str = "pam-desktop.json";
/// The script a project directory starts with unless `entry` names another.
pub const DEFAULT_ENTRY: &str = "index.php";
/// Used when the manifest declares no version: older than any release.
pub const DEFAULT_VERSION: &str = "0.0.0";

const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_EXCLUSIONS: usize = 256;
const MAX_EXTENSIONS: usize = 128;
const MAX_URL_SCHEMES: usize = 8;
/// Schemes that belong to the browser, the mail client or the system.
const RESERVED_URL_SCHEMES: [&str; 6] = ["http", "https", "file", "mailto", "javascript", "data"];

/// Where the application belongs in the menus of the system.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Category {
    Development,
    Productivity,
    Graphics,
    AudioVideo,
    Network,
    #[default]
    Utility,
    Game,
    Education,
}

impl Category {
    const ALL: [Self; 8] = [
        Self::Development,
        Self::Productivity,
        Self::Graphics,
        Self::AudioVideo,
        Self::Network,
        Self::Utility,
        Self::Game,
        Self::Education,
    ];

    /// The keyword used in `pam-desktop.json`.
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Productivity => "productivity",
            Self::Graphics => "graphics",
            Self::AudioVideo => "audio-video",
            Self::Network => "network",
            Self::Utility => "utility",
            Self::Game => "game",
            Self::Education => "education",
        }
    }

    fn parse(keyword: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|category| category.keyword() == keyword)
    }
}

/// When the running application looks for a newer release.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpdatePolicy {
    /// Only when the application asks.
    #[default]
    Manual,
    /// Checks at startup and reports what it found.
    Notify,
    /// Checks, downloads, installs and restarts.
    Automatic,
}

impl UpdatePolicy {
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Notify => "notify",
            Self::Automatic => "automatic",
        }
    }

    fn parse(keyword: &str) -> Option<Self> {
        [Self::Manual, Self::Notify, Self::Automatic]
            .into_iter()
            .find(|policy| policy.keyword() == keyword)
    }
}

/// The `updates` block: the feed the application trusts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateConfig {
    pub endpoint: String,
    /// Ed25519 public key, 64 lowercase hexadecimal characters.
    pub public_key: String,
    pub channel: String,
    pub policy: UpdatePolicy,
}

/// The validated identity and distribution settings of an application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    /// Lowercase reverse-DNS name: package name, desktop entry, update feed.
    pub identifier: String,
    pub name: String,
    pub version: String,
    /// Build number; empty when the manifest has none.
    pub build: String,
    /// The script the host starts, relative to the project directory.
    pub entry: String,
    pub description: String,
    pub publisher: String,
    pub category: Category,
    /// Project-relative PNG or SVG; `None` ships the generic icon.
    pub icon: Option<String>,
    /// Project-relative paths left out of the bundle.
    pub exclude: Vec<String>,
    /// PHP extensions to ship; `None` ships every shared extension the PHP
    /// of the build machine loads.
    pub extensions: Option<Vec<String>>,
    /// Schemes of the links that open the application (`myapp://…`).
    pub url_schemes: Vec<String>,
    pub updates: Option<UpdateConfig>,
}

impl Application {
    /// The file name of the launcher: the last segment of the identifier.
    #[must_use]
    pub fn executable(&self) -> &str {
        self.identifier
            .rsplit('.')
            .next()
            .unwrap_or(&self.identifier)
    }
}

/// The manifest of the project in `project`, as it was written.
#[derive(Clone, Debug, Default)]
pub struct Manifest {
    document: Map<String, Value>,
    /// The name of the project directory: default identity of the application.
    folder: String,
}

impl Manifest {
    /// Reads the manifest for running the application: anything that cannot
    /// be read is treated as absent.
    #[must_use]
    pub fn load(project: &Path) -> Self {
        Self::read(project).unwrap_or_else(|_| Self {
            document: Map::new(),
            folder: folder_name(project),
        })
    }

    /// Reads the manifest for shipping the application.
    ///
    /// # Errors
    ///
    /// When the file exists but is not a JSON object of a sane size.
    pub fn read(project: &Path) -> Result<Self, String> {
        let path = project.join(FILE_NAME);
        let folder = folder_name(project);
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    document: Map::new(),
                    folder,
                });
            }
            Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
        };
        if metadata.len() > MAX_MANIFEST_BYTES {
            return Err(format!("{} is larger than one MiB", path.display()));
        }
        let source = std::fs::read(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        match serde_json::from_slice::<Value>(&source) {
            Ok(Value::Object(document)) => Ok(Self { document, folder }),
            Ok(_) => Err(format!("{} must contain a JSON object", path.display())),
            Err(error) => Err(format!("{} is not valid JSON: {error}", path.display())),
        }
    }

    /// The document, for writing a completed copy into a bundle.
    #[must_use]
    pub fn document(&self) -> &Map<String, Value> {
        &self.document
    }

    /// A string (or number) field.
    #[must_use]
    pub fn text(&self, key: &str) -> Option<String> {
        match self.document.get(key)? {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        }
    }

    /// `applicationId`, or one derived from the folder name.
    #[must_use]
    pub fn application_id(&self) -> String {
        let declared = self
            .document
            .get("applicationId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .filter(|id| paths::valid_application_id(id));
        declared.unwrap_or_else(|| {
            let folder = if self.folder.is_empty() {
                "app"
            } else {
                &self.folder
            };
            let name: String = folder
                .chars()
                .map(|character| {
                    if character.is_ascii_alphanumeric() {
                        character.to_ascii_lowercase()
                    } else {
                        '_'
                    }
                })
                .collect();
            format!("dev.pam.desktop.app_{name}")
        })
    }

    /// `entry`: the script a project directory starts with.
    #[must_use]
    pub fn entry(&self) -> String {
        self.document
            .get("entry")
            .and_then(Value::as_str)
            .filter(|entry| validate_project_path(entry, "entry").is_ok())
            .map_or_else(|| DEFAULT_ENTRY.to_owned(), str::to_owned)
    }

    /// `urlSchemes`: the schemes of the links that open the application.
    ///
    /// # Errors
    ///
    /// When the list is too long or names a scheme that is malformed,
    /// repeated or reserved for the system.
    pub fn url_schemes(&self) -> Result<Vec<String>, String> {
        let schemes = self
            .string_list("urlSchemes", MAX_URL_SCHEMES)?
            .unwrap_or_default();
        let mut unique = HashSet::with_capacity(schemes.len());
        for scheme in &schemes {
            let mut bytes = scheme.bytes();
            let valid = (2..=32).contains(&scheme.len())
                && bytes.next().is_some_and(|first| first.is_ascii_lowercase())
                && bytes.all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'+' | b'.' | b'-')
                });
            if !valid {
                return Err(format!(
                    "urlSchemes lists {scheme:?}: a scheme is a lowercase letter followed by 1 to 31 lowercase letters, digits, +, . or -"
                ));
            }
            if RESERVED_URL_SCHEMES.contains(&scheme.as_str()) {
                return Err(format!("urlSchemes cannot claim the {scheme} scheme"));
            }
            if !unique.insert(scheme.as_str()) {
                return Err(format!("urlSchemes lists {scheme:?} twice"));
            }
        }
        Ok(schemes)
    }

    /// `singleInstance`: whether a second launch hands its link to the
    /// running application instead of starting another. By default an
    /// application that opens links runs once, except in development, where
    /// several checkouts of it may run side by side.
    #[must_use]
    pub fn single_instance(&self, development: bool) -> bool {
        self.document
            .get("singleInstance")
            .and_then(Value::as_bool)
            .unwrap_or_else(|| {
                !development && self.url_schemes().is_ok_and(|schemes| !schemes.is_empty())
            })
    }

    /// The first window (`window.title`, `width`, `height`, `minWidth`,
    /// `minHeight`, `resizable`) with the app name as the default title.
    /// `PAM_DESKTOP_WINDOW=WIDTHxHEIGHT` overrides the size.
    #[must_use]
    pub fn window(&self) -> WindowConfig {
        let mut config = WindowConfig::default();
        if let Some(name) = self.document.get("name").and_then(Value::as_str) {
            config.title = name.to_owned();
        }
        if let Some(window) = self.document.get("window") {
            let number = |key: &str| {
                window
                    .get(key)
                    .and_then(Value::as_f64)
                    .map(|value| value as f32)
            };
            if let Some(title) = window.get("title").and_then(Value::as_str) {
                config.title = title.to_owned();
            }
            if let (Some(width), Some(height)) = (number("width"), number("height")) {
                config.size =
                    Size::new(width.clamp(120.0, 16_384.0), height.clamp(120.0, 16_384.0));
            }
            if let (Some(width), Some(height)) = (number("minWidth"), number("minHeight")) {
                config.min_size = Some(Size::new(width.max(1.0), height.max(1.0)));
            }
            if let Some(resizable) = window.get("resizable").and_then(Value::as_bool) {
                config.resizable = resizable;
            }
        }
        if let Some((width, height)) = std::env::var("PAM_DESKTOP_WINDOW").ok().and_then(|value| {
            let (width, height) = value.split_once('x')?;
            Some((width.parse::<f32>().ok()?, height.parse::<f32>().ok()?))
        }) {
            config.size = Size::new(width.clamp(120.0, 16_384.0), height.clamp(120.0, 16_384.0));
        }
        config
    }

    /// The `updates` block.
    ///
    /// # Errors
    ///
    /// When the block is present and invalid: the application then runs
    /// without updates rather than with a feed it did not mean to trust.
    pub fn updates(&self) -> Result<Option<UpdateConfig>, String> {
        let Some(value) = self
            .document
            .get("updates")
            .filter(|value| !value.is_null())
        else {
            return Ok(None);
        };
        let block = value.as_object().ok_or("updates must be an object")?;
        if let Some(unknown) = block.keys().find(|key| {
            !matches!(
                key.as_str(),
                "endpoint" | "publicKey" | "channel" | "policy"
            )
        }) {
            return Err(format!("updates.{unknown} is not a known field"));
        }
        let text = |key: &str| match block.get(key) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.as_str())),
            Some(_) => Err(format!("updates.{key} must be a string")),
        };
        let endpoint = text("endpoint")?.ok_or("updates.endpoint is required")?;
        validate_update_url(endpoint, "updates.endpoint")?;
        let public_key = text("publicKey")?.ok_or("updates.publicKey is required")?;
        validate_lower_hex(public_key, 64, "updates.publicKey")?;
        let channel = text("channel")?.unwrap_or("stable");
        validate_channel(channel, "updates.channel")?;
        let policy = match text("policy")? {
            None => UpdatePolicy::Manual,
            Some(keyword) => UpdatePolicy::parse(keyword)
                .ok_or("updates.policy must be manual, notify or automatic")?,
        };
        Ok(Some(UpdateConfig {
            endpoint: endpoint.to_owned(),
            public_key: public_key.to_owned(),
            channel: channel.to_owned(),
            policy,
        }))
    }

    /// The identity and distribution settings, validated for shipping.
    ///
    /// # Errors
    ///
    /// When a declared field has the wrong type or an unsafe value.
    pub fn application(&self) -> Result<Application, String> {
        let identifier = match self.document.get("applicationId") {
            None | Some(Value::Null) => self.default_identifier(),
            Some(Value::String(identifier)) => identifier.clone(),
            Some(_) => return Err("applicationId must be a string".into()),
        };
        validate_identifier(&identifier)?;

        let name = match self.string("name")? {
            Some(name) => name,
            None if validate_text(&self.folder, "name", 80, false).is_ok() => self.folder.clone(),
            None => String::from("Application"),
        };
        validate_text(&name, "name", 80, false)?;

        let version = match self.document.get("version") {
            None | Some(Value::Null) => DEFAULT_VERSION.to_owned(),
            Some(Value::String(version)) => version.clone(),
            Some(Value::Number(version)) => version.to_string(),
            Some(_) => return Err("version must be a string or a number".into()),
        };
        validate_version(&version)?;

        let build = match self.document.get("build") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(build)) => build.clone(),
            Some(Value::Number(build)) => build.to_string(),
            Some(_) => return Err("build must be a string or a number".into()),
        };
        validate_text(&build, "build", 64, true)?;

        let entry = self
            .string("entry")?
            .unwrap_or_else(|| DEFAULT_ENTRY.to_owned());
        validate_project_path(&entry, "entry")?;
        if !matches!(
            self.document.get("singleInstance"),
            None | Some(Value::Bool(_) | Value::Null)
        ) {
            return Err("singleInstance must be true or false".into());
        }

        let description = self.string("description")?.unwrap_or_default();
        validate_text(&description, "description", 256, true)?;
        let publisher = self.string("publisher")?.unwrap_or_else(|| name.clone());
        validate_text(&publisher, "publisher", 128, false)?;
        let category = match self.string("category")? {
            None => Category::default(),
            Some(keyword) => Category::parse(&keyword).ok_or_else(|| {
                let known: Vec<&str> = Category::ALL
                    .iter()
                    .map(|category| category.keyword())
                    .collect();
                format!("category must be one of {}", known.join(", "))
            })?,
        };

        let icon = self.string("icon")?;
        if let Some(icon) = &icon {
            validate_project_path(icon, "icon")?;
            let extension = Path::new(icon)
                .extension()
                .and_then(|value| value.to_str())
                .map(str::to_ascii_lowercase);
            if !matches!(extension.as_deref(), Some("png" | "svg")) {
                return Err("icon must be a PNG or SVG file of the project".into());
            }
        }

        let exclude = self
            .string_list("exclude", MAX_EXCLUSIONS)?
            .unwrap_or_default();
        let required = [
            FILE_NAME,
            "composer.json",
            "composer.lock",
            "vendor",
            entry.as_str(),
        ]
        .map(normalized);
        let mut unique = HashSet::with_capacity(exclude.len());
        for path in &exclude {
            validate_project_path(path, "exclude")?;
            let excluded = normalized(path);
            // Excluding a directory excludes what it holds.
            if let Some(required) = required
                .iter()
                .find(|required| required.starts_with(&excluded))
            {
                return Err(format!(
                    "the required project path {:?} cannot be excluded",
                    required.display().to_string()
                ));
            }
            if !unique.insert(excluded) {
                return Err(format!("exclude lists {path:?} twice"));
            }
        }

        let extensions = self.string_list("extensions", MAX_EXTENSIONS)?;
        if let Some(extensions) = &extensions {
            let mut unique = HashSet::with_capacity(extensions.len());
            for extension in extensions {
                let valid = (1..=64).contains(&extension.len())
                    && extension.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                    });
                if !valid || !unique.insert(extension.as_str()) {
                    return Err(format!(
                        "extensions lists {extension:?}, which is not a lowercase PHP extension name or is repeated"
                    ));
                }
            }
        }

        Ok(Application {
            identifier,
            name,
            version,
            build,
            entry,
            description,
            publisher,
            category,
            icon,
            exclude,
            extensions,
            url_schemes: self.url_schemes()?,
            updates: self.updates()?,
        })
    }

    /// The identifier of an application that declares none. The identifier
    /// the host runs such an application with has an underscore, which
    /// package managers refuse.
    fn default_identifier(&self) -> String {
        let slug: String = self
            .folder
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let slug = slug.trim_matches('-');
        let slug: String = slug.chars().take(64).collect();
        if slug.is_empty() {
            String::from("dev.pam.desktop.app")
        } else {
            format!("dev.pam.desktop.app-{slug}")
        }
    }

    fn string(&self, key: &str) -> Result<Option<String>, String> {
        match self.document.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(format!("{key} must be a string")),
        }
    }

    fn string_list(&self, key: &str, limit: usize) -> Result<Option<Vec<String>>, String> {
        let items = match self.document.get(key) {
            None | Some(Value::Null) => return Ok(None),
            Some(Value::Array(items)) => items,
            Some(_) => return Err(format!("{key} must be a list of strings")),
        };
        if items.len() > limit {
            return Err(format!("{key} lists more than {limit} entries"));
        }
        items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("{key} must be a list of strings"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }
}

fn folder_name(project: &Path) -> String {
    project
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("app")
        .to_owned()
}

/// A project-relative path without `.` components.
fn normalized(path: &str) -> PathBuf {
    Path::new(path)
        .components()
        .filter(|component| matches!(component, Component::Normal(_)))
        .collect()
}

/// A lowercase reverse-DNS identifier: the name of the application in
/// package managers, desktop entries and update feeds.
///
/// # Errors
///
/// When the identifier is longer than 155 bytes, has fewer than two segments
/// or a segment that is not a lowercase letter followed by lowercase letters,
/// digits or dashes.
pub fn validate_identifier(identifier: &str) -> Result<(), String> {
    let valid_segment = |segment: &str| {
        let mut characters = segment.chars();
        characters
            .next()
            .is_some_and(|first| first.is_ascii_lowercase())
            && characters.all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
            })
    };
    if identifier.len() > 155
        || identifier.split('.').count() < 2
        || !identifier.split('.').all(valid_segment)
    {
        return Err(format!(
            "applicationId {identifier:?} must be a lowercase reverse-DNS name of at most 155 bytes \
             whose segments begin with a letter and contain letters, digits or dashes"
        ));
    }
    Ok(())
}

/// A version that every package format can carry.
///
/// # Errors
///
/// Unless the version begins with a digit and has at most 64 ASCII letters,
/// digits, dots, pluses, dashes or tildes.
pub fn validate_version(version: &str) -> Result<(), String> {
    let valid = (1..=64).contains(&version.len())
        && version
            .bytes()
            .next()
            .is_some_and(|first| first.is_ascii_digit())
        && version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'-' | b'~'));
    if !valid {
        return Err(format!(
            "version {version:?} must begin with a number and contain at most 64 letters, numbers, dots, pluses, dashes or tildes"
        ));
    }
    Ok(())
}

/// Printable text of a bounded size.
///
/// # Errors
///
/// When the text has control characters, is too long, or is blank where a
/// value is required.
pub fn validate_text(
    value: &str,
    label: &str,
    maximum_bytes: usize,
    allow_empty: bool,
) -> Result<(), String> {
    if (!allow_empty && value.trim().is_empty())
        || value.len() > maximum_bytes
        || value.chars().any(char::is_control)
    {
        return Err(format!(
            "{label} must be printable text of at most {maximum_bytes} bytes"
        ));
    }
    Ok(())
}

/// A path that stays inside the project.
///
/// # Errors
///
/// When the path is empty, absolute (on any system) or walks upwards.
pub fn validate_project_path(value: &str, label: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    let upwards = value.split(['/', '\\']).any(|segment| segment == "..");
    if value.is_empty()
        || value.contains('\0')
        || value.starts_with(['/', '\\'])
        || drive
        || upwards
        || normalized(value).as_os_str().is_empty()
    {
        return Err(format!(
            "{label} path {value:?} must be relative and inside the project"
        ));
    }
    Ok(())
}

/// Exactly `length` lowercase hexadecimal characters.
///
/// # Errors
///
/// When the value has another length or other characters.
pub fn validate_lower_hex(value: &str, length: usize, label: &str) -> Result<(), String> {
    if value.len() != length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!(
            "{label} must contain exactly {length} lowercase hexadecimal characters"
        ));
    }
    Ok(())
}

/// The name of an update channel or of a processor architecture.
///
/// # Errors
///
/// Unless it begins with a letter and has at most 64 letters, digits, dots,
/// dashes or underscores.
pub fn validate_channel(value: &str, label: &str) -> Result<(), String> {
    let mut characters = value.chars();
    let valid = characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && value.len() <= 64
        && characters
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character));
    if !valid {
        return Err(format!(
            "{label} must begin with a letter and contain at most 64 letters, numbers, dots, dashes or underscores"
        ));
    }
    Ok(())
}

/// A URL updates may be fetched from: HTTPS, without credentials or
/// fragment. Debug builds also accept plain HTTP to this machine, for tests.
///
/// # Errors
///
/// For any other URL.
pub fn validate_update_url(value: &str, label: &str) -> Result<(), String> {
    let trusted = value.len() <= 2048
        && !value.contains('#')
        && parse_url(value).is_some_and(|url| {
            !url.credentials
                && (url.scheme == "https"
                    || (cfg!(debug_assertions) && url.scheme == "http" && url.is_loopback()))
        });
    if !trusted {
        return Err(format!(
            "{label} must be an HTTPS URL without credentials or fragment"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pam_desktop_ui::modules::test_support::temporary_directory;

    fn manifest(json: &str) -> Manifest {
        let Value::Object(document) = serde_json::from_str::<Value>(json).expect("fixture is JSON")
        else {
            panic!("fixture is an object");
        };
        Manifest {
            document,
            folder: String::from("My App"),
        }
    }

    #[test]
    fn a_missing_or_broken_manifest_runs_with_defaults() {
        let directory = temporary_directory("manifest-defaults");
        let project = directory.join("Hello World");
        std::fs::create_dir_all(&project).unwrap();
        let loaded = Manifest::load(&project);
        assert_eq!(loaded.application_id(), "dev.pam.desktop.app_hello_world");
        assert_eq!(loaded.text("version"), None);
        assert_eq!(loaded.window().title, WindowConfig::default().title);

        std::fs::write(project.join(FILE_NAME), "{ not json").unwrap();
        assert_eq!(
            Manifest::load(&project).application_id(),
            "dev.pam.desktop.app_hello_world"
        );
        assert!(
            Manifest::read(&project)
                .unwrap_err()
                .contains("not valid JSON")
        );
        std::fs::write(project.join(FILE_NAME), "[]").unwrap();
        assert!(
            Manifest::read(&project)
                .unwrap_err()
                .contains("JSON object")
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn running_reads_identity_version_and_window_as_before() {
        let declared = manifest(
            r#"{"version": 3, "build": "41", "applicationId": "com.Acme.my_app", "name": "Acme",
                "window": {"title": "Main", "width": 50, "height": 900, "minWidth": 0, "minHeight": 300, "resizable": false}}"#,
        );
        assert_eq!(declared.text("version").as_deref(), Some("3"));
        assert_eq!(declared.text("build").as_deref(), Some("41"));
        assert_eq!(declared.text("name").as_deref(), Some("Acme"));
        assert_eq!(declared.text("window"), None);
        // The running host accepts identifiers package managers would not.
        assert_eq!(declared.application_id(), "com.Acme.my_app");
        let window = declared.window();
        assert_eq!(window.title, "Main");
        assert_eq!(window.size, Size::new(120.0, 900.0));
        assert_eq!(window.min_size, Some(Size::new(1.0, 300.0)));
        assert!(!window.resizable);

        let invalid = manifest(r#"{"applicationId": "no-dots", "name": "Acme"}"#);
        assert_eq!(invalid.application_id(), "dev.pam.desktop.app_my_app");
        assert_eq!(invalid.window().title, "Acme");
    }

    #[test]
    fn shipping_fills_defaults() {
        let application = manifest("{}").application().unwrap();
        assert_eq!(application.identifier, "dev.pam.desktop.app-my-app");
        assert_eq!(application.executable(), "app-my-app");
        assert_eq!(application.name, "My App");
        assert_eq!(application.version, DEFAULT_VERSION);
        assert_eq!(application.entry, DEFAULT_ENTRY);
        assert_eq!(application.publisher, "My App");
        assert_eq!(application.description, "");
        assert_eq!(application.category, Category::Utility);
        assert_eq!(application.icon, None);
        assert!(application.exclude.is_empty());
        assert_eq!(application.extensions, None);
        assert!(application.url_schemes.is_empty());
        assert_eq!(application.updates, None);
    }

    #[test]
    fn shipping_reads_every_distribution_field() {
        let application = manifest(
            r#"{"applicationId": "com.acme.invoices", "name": "Invoices", "version": "1.4.0-rc.1", "build": 77,
                "entry": "main.php", "description": "Bills.", "publisher": "Acme", "category": "productivity",
                "icon": "assets/icon.PNG", "exclude": ["storage/cache", "./notes.md"], "extensions": ["ctype", "pdo_sqlite"],
                "urlSchemes": ["invoices", "acme-pay"], "singleInstance": false,
                "updates": {"endpoint": "https://updates.acme.test/stable.json", "publicKey": "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff", "policy": "notify"}}"#,
        )
        .application()
        .unwrap();
        assert_eq!(application.identifier, "com.acme.invoices");
        assert_eq!(application.executable(), "invoices");
        assert_eq!(application.version, "1.4.0-rc.1");
        assert_eq!(application.build, "77");
        assert_eq!(application.entry, "main.php");
        assert_eq!(application.category, Category::Productivity);
        assert_eq!(application.icon.as_deref(), Some("assets/icon.PNG"));
        assert_eq!(application.exclude, ["storage/cache", "./notes.md"]);
        assert_eq!(
            application.extensions,
            Some(vec![String::from("ctype"), String::from("pdo_sqlite")])
        );
        assert_eq!(application.url_schemes, ["invoices", "acme-pay"]);
        let updates = application.updates.unwrap();
        assert_eq!(updates.channel, "stable");
        assert_eq!(updates.policy, UpdatePolicy::Notify);
    }

    #[test]
    fn shipping_rejects_unsafe_identity() {
        let rejected = |json: &str| manifest(json).application().unwrap_err();
        assert!(rejected(r#"{"applicationId": "com.Acme.app"}"#).contains("applicationId"));
        assert!(rejected(r#"{"applicationId": "com.acme.my_app"}"#).contains("applicationId"));
        assert!(rejected(r#"{"applicationId": "acme"}"#).contains("applicationId"));
        assert!(rejected(r#"{"applicationId": 7}"#).contains("applicationId"));
        assert!(rejected(r#"{"version": "v1"}"#).contains("version"));
        assert!(rejected(r#"{"version": "1.0 beta"}"#).contains("version"));
        assert!(rejected(r#"{"name": "  "}"#).contains("name"));
        assert!(rejected(r#"{"name": "Line\nbreak"}"#).contains("name"));
        assert!(rejected(r#"{"category": "Games"}"#).contains("category"));
        assert!(rejected(r#"{"entry": "../main.php"}"#).contains("entry"));
        assert!(rejected(r#"{"entry": "/srv/main.php"}"#).contains("entry"));
        assert!(rejected(r#"{"singleInstance": "yes"}"#).contains("singleInstance"));
        assert_eq!(
            manifest(r#"{"entry": "src/main.php"}"#)
                .application()
                .unwrap()
                .entry,
            "src/main.php"
        );
        assert!(rejected(r#"{"extensions": ["Ctype"]}"#).contains("extensions"));
    }

    #[test]
    fn shipping_rejects_unsafe_paths() {
        let rejected = |json: &str| manifest(json).application().unwrap_err();
        assert!(rejected(r#"{"icon": "../icon.png"}"#).contains("icon"));
        assert!(rejected(r#"{"icon": "/usr/share/icon.png"}"#).contains("icon"));
        assert!(rejected(r#"{"icon": "C:\\icon.png"}"#).contains("icon"));
        assert!(rejected(r#"{"icon": "icon.jpg"}"#).contains("PNG or SVG"));
        assert!(rejected(r#"{"exclude": ["a/../../b"]}"#).contains("exclude"));
        assert!(rejected(r#"{"exclude": ["cache", "./cache"]}"#).contains("twice"));
        assert!(rejected(r#"{"exclude": "cache"}"#).contains("list of strings"));
        for required in [
            "vendor",
            "./vendor",
            "composer.json",
            "composer.lock",
            "pam-desktop.json",
            "index.php",
        ] {
            let error = rejected(&format!(r#"{{"exclude": [{required:?}]}}"#));
            assert!(error.contains("cannot be excluded"), "{required}: {error}");
        }
        assert!(
            rejected(r#"{"entry": "main.php", "exclude": ["main.php"]}"#)
                .contains("cannot be excluded")
        );
        assert!(
            rejected(r#"{"entry": "./src/main.php", "exclude": ["src"]}"#)
                .contains("cannot be excluded")
        );
        // Only the declared entry is required.
        assert!(
            manifest(r#"{"entry": "main.php", "exclude": ["index.php", "vendor-notes"]}"#)
                .application()
                .is_ok()
        );
    }

    #[test]
    fn running_honors_the_declared_entry() {
        assert_eq!(manifest("{}").entry(), "index.php");
        assert_eq!(manifest(r#"{"entry": "main.php"}"#).entry(), "main.php");
        assert_eq!(
            manifest(r#"{"entry": "src/main.php"}"#).entry(),
            "src/main.php"
        );
        // An entry that leaves the project is not one.
        assert_eq!(manifest(r#"{"entry": "../main.php"}"#).entry(), "index.php");
        assert_eq!(manifest(r#"{"entry": 7}"#).entry(), "index.php");
    }

    #[test]
    fn url_schemes_are_validated_and_imply_a_single_instance() {
        let schemes = |json: &str| manifest(json).url_schemes();
        assert_eq!(schemes("{}"), Ok(Vec::new()));
        assert_eq!(
            schemes(r#"{"urlSchemes": ["myapp", "my-app+v2.x"]}"#),
            Ok(vec![String::from("myapp"), String::from("my-app+v2.x")])
        );
        for invalid in [
            r#"["MyApp"]"#,
            r#"["a"]"#,
            r#"["1app"]"#,
            r#"["my app"]"#,
            r#"["my_app"]"#,
            r#"["https"]"#,
            r#"["file"]"#,
            r#"["mailto"]"#,
            r#"["javascript"]"#,
            r#"["data"]"#,
            r#"["http"]"#,
            r#"["one", "one"]"#,
            r#""myapp""#,
            r#"[7]"#,
        ] {
            assert!(
                schemes(&format!(r#"{{"urlSchemes": {invalid}}}"#)).is_err(),
                "{invalid}"
            );
        }
        assert!(schemes(&format!(r#"{{"urlSchemes": ["a{}"]}}"#, "b".repeat(31))).is_ok());
        assert!(schemes(&format!(r#"{{"urlSchemes": ["a{}"]}}"#, "b".repeat(32))).is_err());
        let nine: Vec<String> = (0..9).map(|index| format!("\"app{index}\"")).collect();
        assert!(
            schemes(&format!(r#"{{"urlSchemes": [{}]}}"#, nine.join(",")))
                .unwrap_err()
                .contains("more than 8")
        );
        assert!(
            manifest(r#"{"urlSchemes": ["https"]}"#)
                .application()
                .is_err()
        );

        // An application that opens links runs once, outside development.
        let links = manifest(r#"{"urlSchemes": ["myapp"]}"#);
        assert!(links.single_instance(false));
        assert!(!links.single_instance(true));
        assert!(!manifest("{}").single_instance(false));
        assert!(!manifest(r#"{"urlSchemes": ["https"]}"#).single_instance(false));
        // The manifest has the last word.
        assert!(manifest(r#"{"singleInstance": true}"#).single_instance(true));
        assert!(
            !manifest(r#"{"urlSchemes": ["myapp"], "singleInstance": false}"#)
                .single_instance(false)
        );
    }

    #[test]
    fn the_updates_block_is_validated() {
        let key = "ab".repeat(32);
        let block = |fields: &str| manifest(&format!(r#"{{"updates": {{{fields}}}}}"#)).updates();
        assert_eq!(manifest("{}").updates(), Ok(None));
        assert_eq!(manifest(r#"{"updates": null}"#).updates(), Ok(None));
        let config = block(&format!(r#""endpoint": "https://example.com/feed.json", "publicKey": "{key}", "channel": "beta", "policy": "automatic""#))
            .unwrap()
            .unwrap();
        assert_eq!(config.channel, "beta");
        assert_eq!(config.policy, UpdatePolicy::Automatic);
        assert!(
            block(&format!(r#""publicKey": "{key}""#))
                .unwrap_err()
                .contains("endpoint")
        );
        assert!(
            block(r#""endpoint": "https://example.com/feed.json""#)
                .unwrap_err()
                .contains("publicKey")
        );
        assert!(
            block(&format!(
                r#""endpoint": "https://example.com/feed.json", "publicKey": "{}""#,
                key.to_uppercase()
            ))
            .is_err()
        );
        assert!(
            block(&format!(
                r#""endpoint": "http://example.com/feed.json", "publicKey": "{key}""#
            ))
            .is_err()
        );
        assert!(
            block(&format!(
                r#""endpoint": "https://user:pw@example.com/feed.json", "publicKey": "{key}""#
            ))
            .is_err()
        );
        assert!(
            block(&format!(
                r#""endpoint": "https://example.com/feed.json#latest", "publicKey": "{key}""#
            ))
            .is_err()
        );
        assert!(block(&format!(r#""endpoint": "https://example.com/feed.json", "publicKey": "{key}", "policy": "always""#)).is_err());
        assert!(block(&format!(r#""endpoint": "https://example.com/feed.json", "publicKey": "{key}", "channel": "9x""#)).is_err());
        assert!(block(&format!(r#""endpoint": "https://example.com/feed.json", "publicKey": "{key}", "secret": "x""#)).is_err());
        // An invalid block also stops a build.
        assert!(
            manifest(r#"{"updates": {"endpoint": "ftp://example.com"}}"#)
                .application()
                .is_err()
        );
    }

    #[test]
    fn update_urls_are_https_or_debug_loopback() {
        assert!(validate_update_url("https://cdn.example.com/app.tar.gz", "url").is_ok());
        assert_eq!(
            validate_update_url("http://127.0.0.1:8080/feed", "url").is_ok(),
            cfg!(debug_assertions)
        );
        assert!(validate_update_url("http://192.168.0.2/feed", "url").is_err());
        assert!(validate_update_url("https://exa mple.com/", "url").is_err());
        assert!(
            validate_update_url(&format!("https://example.com/{}", "a".repeat(2048)), "url")
                .is_err()
        );
    }
}
