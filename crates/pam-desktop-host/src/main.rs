//! `pam-desktop`: the native host of PAM Desktop applications.

mod paths;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pam_desktop_ui::backend::WindowConfig;

use crate::paths::AppPaths;
use pam_desktop_ui::geometry::Size;
use pam_desktop_ui::runtime::{Directories, RuntimeConfig};
use pam_desktop_ui::worker::WorkerCommand;

fn usage() -> ExitCode {
    eprintln!(
        "PAM Desktop {}\n\n\
         usage:\n  \
         pam-desktop dev [app]   run with hot reload (default: current directory)\n  \
         pam-desktop run [app]   run the application\n  \
         pam-desktop --version",
        env!("CARGO_PKG_VERSION"),
    );
    ExitCode::from(64)
}

/// The PHP launcher: `PAM_DESKTOP_PHP`, then `pam exec`, then `php`.
fn php_command() -> (PathBuf, Vec<OsString>) {
    if let Some(php) = std::env::var_os("PAM_DESKTOP_PHP") {
        return (PathBuf::from(php), Vec::new());
    }
    let on_path = |name: &str| {
        std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths).map(|directory| directory.join(name)).find(|candidate| candidate.is_file())
        })
    };
    let executable = |name: &str| if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() };
    if let Some(pam) = on_path(&executable("pam")) {
        return (pam, vec![OsString::from("exec")]);
    }
    (on_path(&executable("php")).unwrap_or_else(|| PathBuf::from("php")), Vec::new())
}

/// The worker bootstrap shipped with the PHP package.
fn worker_bootstrap(project: &Path) -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("PAM_DESKTOP_WORKER") {
        return Some(PathBuf::from(explicit));
    }
    let relative = Path::new("packages/desktop/bin/pam-desktop-worker");
    let mut candidates = vec![project.join("vendor/pushinbr/pam-desktop").join(relative)];
    // Running from a checkout of this repository (examples, development).
    let mut directory = Some(project);
    while let Some(current) = directory {
        candidates.push(current.join(relative));
        directory = current.parent();
    }
    candidates.into_iter().find(|candidate| candidate.is_file())
}

/// A string (or number) field of `pam-desktop.json`.
fn manifest_text(project: &Path, key: &str) -> Option<String> {
    let manifest = std::fs::read_to_string(project.join("pam-desktop.json")).ok()?;
    let manifest = serde_json::from_str::<serde_json::Value>(&manifest).ok()?;
    match manifest.get(key)? {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// `applicationId` of `pam-desktop.json`, or one derived from the folder name.
fn application_id(project: &Path) -> String {
    let declared = std::fs::read_to_string(project.join("pam-desktop.json"))
        .ok()
        .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
        .and_then(|manifest| manifest.get("applicationId").and_then(serde_json::Value::as_str).map(str::to_owned))
        .filter(|id| paths::valid_application_id(id));
    declared.unwrap_or_else(|| {
        let name: String = project
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("app")
            .chars()
            .map(|character| if character.is_ascii_alphanumeric() { character.to_ascii_lowercase() } else { '_' })
            .collect();
        format!("dev.pam.desktop.app_{name}")
    })
}

/// The first window, from `pam-desktop.json` (`window.title`, `width`,
/// `height`, `minWidth`, `minHeight`, `resizable`) with the app name as the
/// default title. `PAM_DESKTOP_WINDOW=WIDTHxHEIGHT` overrides the size.
fn window_config(project: &Path) -> WindowConfig {
    let mut config = WindowConfig::default();
    let manifest = std::fs::read_to_string(project.join("pam-desktop.json"))
        .ok()
        .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok());
    if let Some(manifest) = &manifest {
        if let Some(name) = manifest.get("name").and_then(serde_json::Value::as_str) {
            config.title = name.to_owned();
        }
        if let Some(window) = manifest.get("window") {
            let number = |key: &str| window.get(key).and_then(serde_json::Value::as_f64).map(|value| value as f32);
            if let Some(title) = window.get("title").and_then(serde_json::Value::as_str) {
                config.title = title.to_owned();
            }
            if let (Some(width), Some(height)) = (number("width"), number("height")) {
                config.size = Size::new(width.clamp(120.0, 16_384.0), height.clamp(120.0, 16_384.0));
            }
            if let (Some(width), Some(height)) = (number("minWidth"), number("minHeight")) {
                config.min_size = Some(Size::new(width.max(1.0), height.max(1.0)));
            }
            if let Some(resizable) = window.get("resizable").and_then(serde_json::Value::as_bool) {
                config.resizable = resizable;
            }
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

fn run(target: &Path, development: bool) -> ExitCode {
    let entry = if target.is_dir() { target.join("index.php") } else { target.to_path_buf() };
    let Ok(entry) = entry.canonicalize() else {
        eprintln!("pam-desktop: {} does not exist", entry.display());
        return ExitCode::from(66);
    };
    let project = entry.parent().map(Path::to_path_buf).unwrap_or_default();
    let Some(bootstrap) = worker_bootstrap(&project) else {
        eprintln!("pam-desktop: the pushinbr/pam-desktop package is not installed in {}", project.display());
        return ExitCode::from(69);
    };
    let (program, mut arguments) = php_command();
    arguments.push(bootstrap.into_os_string());

    let platform = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    };
    let application_id = application_id(&project);
    let mut environment = vec![
        ("PAM_DESKTOP_PROJECT_ROOT".into(), project.display().to_string()),
        ("PAM_DESKTOP_APPLICATION_ID".into(), application_id.clone()),
    ];
    // Navigation state, hot reload checkpoints and module storage are per application.
    let app_paths = if development { AppPaths::for_development(&project) } else { AppPaths::for_application(&application_id) }
        .with_overrides();
    if let Err(error) = app_paths.create() {
        eprintln!("pam-desktop: cannot create the application directories: {error}");
        return ExitCode::from(73);
    }
    environment.extend(app_paths.environment());
    let config = RuntimeConfig {
        worker: WorkerCommand { program, arguments, entry, working_directory: Some(project.clone()), environment },
        window: window_config(&project),
        platform,
        development,
        watch: if development { vec![project.clone()] } else { Vec::new() },
        directories: Directories { data: app_paths.data.clone(), cache: app_paths.cache.clone(), state: app_paths.state.clone() },
        app_version: manifest_text(&project, "version").unwrap_or_default(),
        build_number: manifest_text(&project, "build").unwrap_or_default(),
        launch_url: std::env::var("PAM_DESKTOP_LAUNCH_URL").ok().filter(|url| !url.is_empty()),
        modules: Vec::new(),
    };

    #[cfg(target_os = "linux")]
    {
        pam_desktop_gtk::run(&application_id, config)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (config, application_id);
        eprintln!("pam-desktop: this platform is not supported yet");
        ExitCode::FAILURE
    }
}

fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1);
    match arguments.next().as_deref().and_then(|argument| argument.to_str()) {
        Some(command @ ("run" | "dev")) => {
            let target = arguments.next().map_or_else(|| PathBuf::from("."), PathBuf::from);
            run(&target, command == "dev")
        }
        Some("--version" | "-V") => {
            println!("pam-desktop {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => usage(),
    }
}
