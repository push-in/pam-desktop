//! `pam-desktop init`: the files a new application starts from.
//!
//! Nothing that exists is overwritten: the command fills in what is missing
//! and says what it left alone.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::manifest;

const USAGE: &str = "usage: pam-desktop init [directory] [--name NAME] [--id APPLICATION-ID]";

const INDEX: &str = r#"<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\Style;
use Pam\Desktop\UI\Button;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\Text;

require __DIR__.'/vendor/autoload.php';

final class Home extends Component
{
    #[State]
    public int $count = 0;

    public function render(): Column
    {
        return Column::make(
            Text::make('@NAME@')->style(new Style(fontSize: 26, fontWeight: 700)),
            Text::make("Count: {$this->count}")->style(new Style(fontSize: 16)),
            Button::make('Add one')->onPress(fn () => $this->count++),
        )->style(new Style(flexGrow: 1, padding: 28, gap: 12));
    }
}

App::run(new Home());
"#;

const IGNORED: &str = "/vendor/\n/.pam-desktop/\n/dist/\n";

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
fn create(path: &Path, contents: &str, report: &mut Vec<String>) -> Result<(), String> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if path.exists() {
        report.push(format!("[kept] {name}"));
        return Ok(());
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
    create(&directory.join(manifest::FILE_NAME), &text, &mut report)?;
    let title = serde_json::to_string(&name).map_err(|error| error.to_string())?;
    // A PHP single-quoted string: only the backslash and the quote are special.
    let title = title
        .trim_matches('"')
        .replace("\\\"", "\"")
        .replace('\'', "\\'");
    create(
        &directory.join(manifest::DEFAULT_ENTRY),
        &INDEX.replace("@NAME@", &title),
        &mut report,
    )?;
    let composer = serde_json::json!({
        "name": format!("app/{}", slug(&folder)),
        "type": "project",
        "require": { "php": "^8.5", "pushinbr/pam-desktop": concat!("^", env!("CARGO_PKG_VERSION_MAJOR"), ".", env!("CARGO_PKG_VERSION_MINOR")) },
    });
    let mut composer =
        serde_json::to_string_pretty(&composer).map_err(|error| error.to_string())?;
    composer.push('\n');
    create(&directory.join("composer.json"), &composer, &mut report)?;
    create(&directory.join(".gitignore"), IGNORED, &mut report)?;
    if !directory.join("vendor/autoload.php").is_file() {
        report.push(String::from("next: composer install, then pam-desktop dev"));
    } else {
        report.push(String::from("next: pam-desktop dev"));
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
            4
        );

        let manifest = manifest::Manifest::read(&directory).unwrap();
        let application = manifest.application().unwrap();
        assert_eq!(application.identifier, "com.example.bill-s-notes");
        let index = fs::read_to_string(directory.join("index.php")).unwrap();
        assert!(index.contains(r"Text::make('Bill\'s Notes')"));
        assert!(
            fs::read_to_string(directory.join("composer.json"))
                .unwrap()
                .contains("\"pushinbr/pam-desktop\": \"^2.0\"")
        );

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
            4
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
