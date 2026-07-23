use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use pam_desktop_protocol::validate_identifier;

pub enum PluginCommand {
    Help,
    New { project: PathBuf, id: String },
    Build { project: PathBuf, id: String },
}

impl PluginCommand {
    pub fn parse(mut arguments: Vec<OsString>) -> Result<Self, String> {
        let Some(command) = arguments.first().map(|value| value.to_string_lossy()) else {
            return Ok(Self::Help);
        };
        if matches!(command.as_ref(), "--help" | "-h" | "help") {
            return Ok(Self::Help);
        }
        if !matches!(command.as_ref(), "new" | "build") {
            return Err(format!(
                "unknown plugin command {command:?}; expected new or build"
            ));
        }
        let command = arguments.remove(0);
        let id = arguments
            .first()
            .ok_or_else(|| {
                format!(
                    "plugin {} requires an identifier",
                    command.to_string_lossy()
                )
            })?
            .to_string_lossy()
            .into_owned();
        arguments.remove(0);
        validate_identifier(&id, "plugin")?;
        let project = if arguments.is_empty() {
            PathBuf::from(".")
        } else {
            PathBuf::from(arguments.remove(0))
        };
        if let Some(unknown) = arguments.first() {
            return Err(format!(
                "unexpected plugin argument: {}",
                unknown.to_string_lossy()
            ));
        }
        match command.to_string_lossy().as_ref() {
            "new" => Ok(Self::New { project, id }),
            "build" => Ok(Self::Build { project, id }),
            _ => unreachable!("plugin command was validated above"),
        }
    }
}

pub fn scaffold(project: &Path, id: &str) -> Result<PathBuf, String> {
    let project = project.canonicalize().map_err(|error| {
        format!(
            "cannot resolve plugin project {}: {error}",
            project.display()
        )
    })?;
    let directory = project.join("plugins").join(id);
    if directory.exists() {
        return Err(format!(
            "Rust plugin directory already exists: {}",
            directory.display()
        ));
    }
    fs::create_dir_all(directory.join("src"))
        .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
    let package = format!("pam-plugin-{}", id.replace('.', "-"));
    let manifest = format!(
        r#"[package]
name = "{package}"
version = "0.1.0"
edition = "2024"
rust-version = "1.88"
publish = false

[dependencies]
pam-desktop-plugin = {{ git = "https://github.com/push-in/pam-desktop", tag = "v{version}" }}
serde_json = "1"

[profile.release]
codegen-units = 1
lto = "thin"
panic = "abort"
strip = "symbols"
"#,
        version = env!("CARGO_PKG_VERSION"),
    );
    fs::write(directory.join("Cargo.toml"), manifest)
        .map_err(|error| format!("cannot write plugin Cargo.toml: {error}"))?;
    let source = format!(
        r#"use pam_desktop_plugin::{{
    Plugin, PluginContext, PluginFailure, PluginOutput, serve,
}};
use pam_desktop_plugin::protocol::PluginMetadata;

struct {type_name};

impl Plugin for {type_name} {{
    fn metadata(&self) -> PluginMetadata {{
        PluginMetadata {{
            identifier: "{id}".to_owned(),
            name: "{display_name}".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            commands: vec!["hello".to_owned()],
        }}
    }}

    fn invoke(
        &mut self,
        context: PluginContext,
    ) -> Result<PluginOutput, PluginFailure> {{
        match context.command.as_str() {{
            "hello" => Ok(PluginOutput::new(serde_json::json!({{
                "message": "Hello from a supervised Rust plugin!",
                "payload": context.payload,
            }}))),
            _ => Err(PluginFailure::handler_failed("Unknown command.")),
        }}
    }}
}}

fn main() -> Result<(), Box<dyn std::error::Error>> {{
    serve({type_name})?;
    Ok(())
}}
"#,
        type_name = rust_type_name(id),
        display_name = display_name(id),
    );
    fs::write(directory.join("src/main.rs"), source)
        .map_err(|error| format!("cannot write plugin source: {error}"))?;
    fs::write(directory.join(".gitignore"), "/target\n")
        .map_err(|error| format!("cannot write plugin .gitignore: {error}"))?;
    Ok(directory)
}

pub fn build(project: &Path, id: &str) -> Result<PathBuf, String> {
    let project = project.canonicalize().map_err(|error| {
        format!(
            "cannot resolve plugin project {}: {error}",
            project.display()
        )
    })?;
    let directory = project.join("plugins").join(id);
    if !directory.join("Cargo.toml").is_file() {
        return Err(format!(
            "Rust plugin source is missing: {}",
            directory.display()
        ));
    }
    let mut cargo = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cargo.args(["build", "--release"]);
    if directory.join("Cargo.lock").is_file() {
        cargo.arg("--locked");
    }
    let status = cargo
        .current_dir(&directory)
        .status()
        .map_err(|error| format!("cannot start Cargo for plugin {id:?}: {error}"))?;
    if !status.success() {
        return Err(format!(
            "Cargo failed to build Rust plugin {id:?}: {status}"
        ));
    }
    let package = format!("pam-plugin-{}", id.replace('.', "-"));
    let source = directory
        .join("target")
        .join("release")
        .join(format!("{package}{}", std::env::consts::EXE_SUFFIX));
    if !source.is_file() {
        return Err(format!(
            "Cargo did not produce the expected plugin executable: {}",
            source.display()
        ));
    }
    let output = project.join("plugins").join("bin");
    fs::create_dir_all(&output)
        .map_err(|error| format!("cannot create {}: {error}", output.display()))?;
    let destination = output.join(format!("{id}{}", std::env::consts::EXE_SUFFIX));
    fs::copy(&source, &destination).map_err(|error| {
        format!(
            "cannot install plugin executable {}: {error}",
            destination.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("cannot mark plugin executable: {error}"))?;
    }
    Ok(destination)
}

pub fn print_usage(executable: &std::ffi::OsStr) {
    println!(
        "Usage:\n  {} plugin new <id> [project]\n  {} plugin build <id> [project]",
        executable.to_string_lossy(),
        executable.to_string_lossy(),
    );
}

fn rust_type_name(id: &str) -> String {
    let mut name = String::new();
    for segment in id.split(['.', '-', '_']) {
        let mut characters = segment.chars();
        if let Some(first) = characters.next() {
            name.extend(first.to_uppercase());
            name.extend(characters);
        }
    }
    name.push_str("Plugin");
    name
}

fn display_name(id: &str) -> String {
    id.split(['.', '-', '_'])
        .map(|segment| {
            let mut characters = segment.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(characters).collect::<String>()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn scaffolds_a_versioned_safe_rust_plugin() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "pam-plugin-scaffold-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("fixture should be created");
        let plugin = scaffold(&root, "system.info").expect("plugin should scaffold");
        let manifest = fs::read_to_string(plugin.join("Cargo.toml"))
            .expect("plugin manifest should be readable");
        let source =
            fs::read_to_string(plugin.join("src/main.rs")).expect("source should be readable");
        assert!(manifest.contains("pam-desktop-plugin"));
        assert!(manifest.contains(&format!("tag = \"v{}\"", env!("CARGO_PKG_VERSION"))));
        assert!(source.contains("serve(SystemInfoPlugin)"));
        assert!(source.contains("identifier: \"system.info\""));
        let _ = fs::remove_dir_all(root);
    }
}
