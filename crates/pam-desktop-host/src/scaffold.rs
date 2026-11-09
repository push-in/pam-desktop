//! `pam-desktop init`: the files a new application starts from.
//!
//! Nothing that exists is overwritten: the command fills in what is missing
//! and says what it left alone.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::manifest;

const USAGE: &str = "usage: pam-desktop init [directory] [--name NAME] [--id APPLICATION-ID]";

/// The release line new applications require.
const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION_MAJOR"),
    ".",
    env!("CARGO_PKG_VERSION_MINOR")
);

/// The skeleton of a new application, from `packages/desktop/resources/skeleton`:
/// its path in the project and its contents. `@NAME@` is the application name.
const SKELETON: [(&str, &str); 11] = {
    macro_rules! file {
        ($path:literal) => {
            (
                $path,
                include_str!(concat!(
                    "../../../packages/desktop/resources/skeleton/",
                    $path
                )),
            )
        };
    }
    [
        file!("index.php"),
        file!("README.md"),
        file!("composer.json"),
        file!(".gitignore"),
        file!("src/app.css"),
        file!("src/Navigation/AppRoute.php"),
        file!("src/Navigation/AppRouter.php"),
        file!("src/Screens/Home/HomeScreen.pam.php"),
        file!("src/Services/Greeting.php"),
        file!("src/Lib/Plural.php"),
        file!("tests/HomeScreenTest.php"),
    ]
};

#[derive(Debug, PartialEq, Eq)]
pub struct InitOptions {
    pub directory: PathBuf,
    pub name: Option<String>,
    pub id: Option<String>,
}

impl InitOptions {
    /// # Errors
    ///
    /// When an option is unknown or misses its value.
    pub fn parse(arguments: impl Iterator<Item = OsString>) -> Result<Self, String> {
        let mut options = Self {
            directory: PathBuf::from("."),
            name: None,
            id: None,
        };
        let mut directory = None;
        let mut arguments = arguments;
        while let Some(argument) = arguments.next() {
            let mut value = |flag: &str| {
                arguments
                    .next()
                    .and_then(|value| value.into_string().ok())
                    .ok_or_else(|| format!("{flag} needs a value\n{USAGE}"))
            };
            match argument.to_str() {
                Some("--name") => options.name = Some(value("--name")?),
                Some("--id") => options.id = Some(value("--id")?),
                Some(flag) if flag.starts_with('-') => {
                    return Err(format!("unknown option {flag}\n{USAGE}"));
                }
                _ if directory.is_none() => directory = Some(PathBuf::from(argument)),
                _ => return Err(USAGE.to_owned()),
            }
        }
        if let Some(directory) = directory {
            options.directory = directory;
        }
        Ok(options)
    }
}

/// Lowercase letters, digits and dashes of `text`, starting with a letter.
fn slug(text: &str) -> String {
    let mut slug = String::new();
    for character in text.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug
        .trim_end_matches('-')
        .trim_start_matches(|character: char| character.is_ascii_digit() || character == '-');
    if slug.is_empty() {
        String::from("app")
    } else {
        slug.chars().take(40).collect()
    }
}

/// Writes `contents` unless the file exists; reports which happened.
fn create(
    directory: &Path,
    name: &str,
    contents: &str,
    report: &mut Vec<String>,
) -> Result<(), String> {
    let path = directory.join(name);
    let path = path.as_path();
    if path.exists() {
        report.push(format!("[kept] {name}"));
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    fs::write(path, contents)
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    report.push(format!("[new]  {name}"));
    Ok(())
}

/// Creates the starting files in the directory and returns what it did,
/// one line per file.
///
/// # Errors
///
/// When the directory cannot be created or written, or the identity given
/// on the command line is not a valid one.
pub fn init(options: &InitOptions) -> Result<Vec<String>, String> {
    fs::create_dir_all(&options.directory)
        .map_err(|error| format!("cannot create {}: {error}", options.directory.display()))?;
    let directory = options
        .directory
        .canonicalize()
        .map_err(|error| format!("cannot resolve {}: {error}", options.directory.display()))?;
    let folder = directory
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = options.name.clone().unwrap_or_else(|| {
        if folder.is_empty() {
            String::from("My application")
        } else {
            folder.clone()
        }
    });
    let id = options
        .id
        .clone()
        .unwrap_or_else(|| format!("com.example.{}", slug(&folder)));

    // The identity is checked with the rules `build` applies later.
    manifest::validate_identifier(&id).map_err(|error| format!("{error}\n{USAGE}"))?;
    if name.trim().is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
        return Err(format!(
            "the name must be 1 to 80 bytes of printable text\n{USAGE}"
        ));
    }

    let document = serde_json::json!({
        "$schema": "./vendor/pushinbr/pam-desktop/packages/desktop/resources/pam-desktop.schema.json",
        "applicationId": id,
        "name": name,
        "version": "0.1.0",
        "entry": manifest::DEFAULT_ENTRY,
        "window": { "width": 960, "height": 640 },
    });
    let mut text = serde_json::to_string_pretty(&document).map_err(|error| error.to_string())?;
    text.push('\n');

    let mut report = Vec::new();
    create(&directory, manifest::FILE_NAME, &text, &mut report)?;
    // Preserve Unicode and escape only PHP single-quoted string delimiters.
    let title = name.replace('\\', "\\\\").replace('\'', "\\'");
    let json_name = serde_json::to_string(&name).map_err(|error| error.to_string())?;
    let json_name = &json_name[1..json_name.len() - 1];
    for (path, contents) in SKELETON {
        let contents = match path {
            "composer.json" => contents
                .replace("@SLUG@", &slug(&folder))
                .replace("@VERSION@", VERSION)
                .replace("@NAME@", json_name),
            _ if path.ends_with(".php") => contents.replace("@NAME@", &title),
            _ => contents.replace("@NAME@", &name),
        };
        create(&directory, path, &contents, &mut report)?;
    }
    if !directory.join("vendor/autoload.php").is_file() {
        report.push(String::from("next: composer install, then pam desktop dev"));
    } else {
        report.push(String::from("next: pam desktop dev"));
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(arguments: &[&str]) -> Result<InitOptions, String> {
        InitOptions::parse(arguments.iter().map(OsString::from))
    }

    #[test]
    fn parses_the_command_line() {
        assert_eq!(
            options(&[]).unwrap(),
            InitOptions {
                directory: PathBuf::from("."),
                name: None,
                id: None
            }
        );
        let parsed = options(&["notes", "--name", "My Notes", "--id", "com.acme.notes"]).unwrap();
        assert_eq!(parsed.directory, PathBuf::from("notes"));
        assert_eq!(parsed.name.as_deref(), Some("My Notes"));
        assert_eq!(parsed.id.as_deref(), Some("com.acme.notes"));
        assert!(options(&["--name"]).unwrap_err().contains("needs a value"));
        assert!(
            options(&["--force"])
                .unwrap_err()
                .contains("unknown option")
        );
        assert!(options(&["one", "two"]).is_err());
    }

    #[test]
    fn slugs_are_valid_identifier_segments() {
        assert_eq!(slug("My Notes!"), "my-notes");
        assert_eq!(slug("2048 game"), "game");
        assert_eq!(slug("___"), "app");
        assert_eq!(slug("çafé"), "af");
    }

    #[test]
    fn creates_a_project_and_keeps_what_exists() {
        let root = std::env::temp_dir().join(format!("pam-desktop-init-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let directory = root.join("Bill's Notes");
        let created = init(&InitOptions {
            directory: directory.clone(),
            name: None,
            id: None,
        })
        .unwrap();
        assert_eq!(
            created
                .iter()
                .filter(|line| line.starts_with("[new]"))
                .count(),
            12
        );

        let manifest = manifest::Manifest::read(&directory).unwrap();
        let application = manifest.application().unwrap();
        assert_eq!(application.identifier, "com.example.bill-s-notes");
        let index = fs::read_to_string(directory.join("index.php")).unwrap();
        assert!(index.contains("AppRouter::make()"));
        let home =
            fs::read_to_string(directory.join("src/Screens/Home/HomeScreen.pam.php")).unwrap();
        assert!(home.contains(r"$title = 'Bill\'s Notes'"));
        let composer: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(directory.join("composer.json")).unwrap())
                .unwrap();
        assert_eq!(composer["name"], "app/bill-s-notes");
        assert_eq!(
            composer["description"],
            "Bill's Notes, a PAM Desktop application"
        );
        assert_eq!(
            composer["require"]["pushinbr/pam-desktop"],
            format!("^{VERSION}").as_str()
        );
        assert!(directory.join("src/Services/Greeting.php").is_file());

        fs::write(directory.join("index.php"), "<?php // mine").unwrap();
        let again = init(&InitOptions {
            directory: directory.clone(),
            name: None,
            id: None,
        })
        .unwrap();
        assert_eq!(
            again
                .iter()
                .filter(|line| line.starts_with("[kept]"))
                .count(),
            12
        );
        assert_eq!(
            fs::read_to_string(directory.join("index.php")).unwrap(),
            "<?php // mine"
        );

        assert!(
            init(&InitOptions {
                directory: root.join("bad"),
                name: None,
                id: Some("Not An Id".into())
            })
            .is_err()
        );
        let _ = fs::remove_dir_all(&root);
    }
}
