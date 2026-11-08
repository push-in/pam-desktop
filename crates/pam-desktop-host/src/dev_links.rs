//! Temporary Linux URL handlers for development runs.
//!
//! A packaged application registers its URL schemes when installed. A `dev`
//! run needs the same path so OAuth and other browser callbacks reach the
//! already-running hot-reload process.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Registration {
    desktop_file: PathBuf,
    config_file: PathBuf,
    changes: Vec<(String, Option<String>)>,
    desktop_id: String,
}

impl Registration {
    pub fn install(
        application_id: &str,
        project: &Path,
        schemes: &[String],
    ) -> Result<Self, String> {
        if schemes.is_empty() {
            return Err("no URL schemes are declared".into());
        }
        let executable = std::env::current_exe()
            .map_err(|error| format!("cannot locate pam-desktop: {error}"))?;
        // A line break would end the `Exec` key and start another entry key.
        for path in [executable.as_path(), project] {
            if path.to_string_lossy().chars().any(char::is_control) {
                return Err(format!(
                    "{} cannot be registered as a URL handler",
                    path.display()
                ));
            }
        }
        let data_home = user_directory("XDG_DATA_HOME", ".local/share")?;
        let config_home = user_directory("XDG_CONFIG_HOME", ".config")?;
        let applications = data_home.join("applications");
        fs::create_dir_all(&applications)
            .map_err(|error| format!("cannot create {}: {error}", applications.display()))?;
        let desktop_id = format!("pam-desktop-dev-{}.desktop", safe_id(application_id));
        let desktop_file = applications.join(&desktop_id);
        let config_file = config_home.join("mimeapps.list");
        fs::create_dir_all(&config_home)
            .map_err(|error| format!("cannot create {}: {error}", config_home.display()))?;

        let entry = format!(
            "[Desktop Entry]\nType=Application\nName=PAM Desktop development: {}\nNoDisplay=true\nTerminal=false\nExec={} dev {} %u\nMimeType={}\n",
            desktop_value(application_id),
            desktop_exec_arg(&executable.to_string_lossy()),
            desktop_exec_arg(&project.to_string_lossy()),
            schemes
                .iter()
                .map(|scheme| format!("x-scheme-handler/{scheme};"))
                .collect::<String>()
        );
        fs::write(&desktop_file, entry)
            .map_err(|error| format!("cannot write {}: {error}", desktop_file.display()))?;
        refresh_mime_database(&applications);

        let old = fs::read_to_string(&config_file).unwrap_or_default();
        let mut changes = Vec::new();
        let mut updated = old.clone();
        for scheme in schemes {
            let mime = format!("x-scheme-handler/{scheme}");
            let previous = default_value(&old, &mime).filter(|value| {
                value
                    .split(';')
                    .find(|part| !part.is_empty())
                    .is_some_and(|handler| handler != desktop_id)
            });
            updated = set_default(&updated, &mime, Some(&desktop_id));
            changes.push((mime, previous));
        }
        if let Err(error) = fs::write(&config_file, updated) {
            let _ = fs::remove_file(&desktop_file);
            return Err(format!("cannot register development URL handlers: {error}"));
        }
        Ok(Self {
            desktop_file,
            config_file,
            changes,
            desktop_id,
        })
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        if let Ok(mut current) = fs::read_to_string(&self.config_file) {
            for (mime, previous) in &self.changes {
                // Do not overwrite a handler the user changed while dev mode ran.
                if default_value(&current, mime)
                    .as_deref()
                    .and_then(|value| value.split(';').find(|part| !part.is_empty()))
                    == Some(&self.desktop_id)
                {
                    current = set_default(&current, mime, previous.as_deref());
                }
            }
            let _ = fs::write(&self.config_file, current);
        }
        let _ = fs::remove_file(&self.desktop_file);
        if let Some(applications) = self.desktop_file.parent() {
            refresh_mime_database(applications);
        }
    }
}

fn user_directory(variable: &str, fallback: &str) -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os(variable).filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(PathBuf::from(home).join(fallback))
}

fn safe_id(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect()
}

fn desktop_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace(['\n', '\r'], " ")
}

fn desktop_exec_arg(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('`', "\\`")
            .replace('$', "\\$")
            // `%` starts a field code (`%u`); a literal one is `%%`.
            .replace('%', "%%")
    )
}

fn refresh_mime_database(applications: &Path) {
    // The cache tool is part of common Linux desktop installations, but a
    // minimal development image may not include it. mimeapps.list remains the
    // source of truth when the cache helper is unavailable.
    let _ = Command::new("update-desktop-database")
        .arg(applications)
        .status();
}

fn default_value(contents: &str, mime: &str) -> Option<String> {
    let mut in_defaults = false;
    for line in contents.lines() {
        if line.starts_with('[') {
            in_defaults = line.trim() == "[Default Applications]";
        } else if in_defaults
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == mime
        {
            return Some(value.trim().to_owned());
        }
    }
    None
}

fn set_default(contents: &str, mime: &str, handler: Option<&str>) -> String {
    let mut lines: Vec<String> = contents.lines().map(str::to_owned).collect();
    let section = "[Default Applications]";
    let mut start = lines.iter().position(|line| line.trim() == section);
    if start.is_none() && handler.is_some() {
        if !lines.is_empty() && !lines.last().is_some_and(String::is_empty) {
            lines.push(String::new());
        }
        lines.push(section.into());
        start = Some(lines.len() - 1);
    }
    if let Some(start) = start {
        let end = lines[start + 1..]
            .iter()
            .position(|line| line.starts_with('['))
            .map_or(lines.len(), |offset| start + 1 + offset);
        let key_line = (start + 1..end).find(|index| {
            lines[*index]
                .split_once('=')
                .is_some_and(|(key, _)| key.trim() == mime)
        });
        match (key_line, handler) {
            (Some(index), Some(value)) => {
                let value = value.trim();
                lines[index] = format!(
                    "{mime}={value}{}",
                    if value.ends_with(';') { "" } else { ";" }
                );
            }
            (Some(index), None) => {
                lines.remove(index);
            }
            (None, Some(value)) => {
                let value = value.trim();
                lines.insert(
                    end,
                    format!(
                        "{mime}={value}{}",
                        if value.ends_with(';') { "" } else { ";" }
                    ),
                );
            }
            (None, None) => {}
        }
    }
    let mut result = lines.join("\n");
    if !result.is_empty() {
        result.push('\n');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_and_restores_only_the_requested_default_handler() {
        let original = "[Added Associations]\ntext/plain=editor.desktop;\n\n[Default Applications]\nx-scheme-handler/demo=browser.desktop;other.desktop;\n";
        let changed = set_default(original, "x-scheme-handler/demo", Some("pam.desktop"));
        assert!(changed.contains("x-scheme-handler/demo=pam.desktop;\n"));
        assert!(!changed.contains("browser.desktop"));
        assert_eq!(
            set_default(
                &changed,
                "x-scheme-handler/demo",
                Some("browser.desktop;other.desktop;")
            ),
            original
        );
    }

    #[test]
    fn removes_a_temporary_default_without_creating_an_empty_section() {
        let original = "[Default Applications]\nx-scheme-handler/demo=pam.desktop;\n";
        assert_eq!(
            set_default(original, "x-scheme-handler/demo", None),
            "[Default Applications]\n"
        );
    }

    #[test]
    fn quotes_exec_arguments_for_desktop_files() {
        assert_eq!(
            desktop_exec_arg("/a path/app\"$x"),
            "\"/a path/app\\\"\\$x\""
        );
    }
}
