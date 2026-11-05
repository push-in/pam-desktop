//! `pam-desktop`: the native host of PAM Desktop applications.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pam_desktop_ui::backend::WindowConfig;
use pam_desktop_ui::runtime::RuntimeConfig;
use pam_desktop_ui::worker::WorkerCommand;

fn usage() -> ExitCode {
    eprintln!("usage: pam-desktop run <app directory or entry.php>");
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

fn run(target: &Path) -> ExitCode {
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
    let config = RuntimeConfig {
        worker: WorkerCommand {
            program,
            arguments,
            entry,
            working_directory: Some(project.clone()),
            environment: vec![("PAM_DESKTOP_PROJECT_ROOT".into(), project.display().to_string())],
        },
        window: WindowConfig::default(),
        platform,
    };

    #[cfg(target_os = "linux")]
    {
        pam_desktop_gtk::run("dev.pam.desktop", config)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = config;
        eprintln!("pam-desktop: this platform is not supported yet");
        ExitCode::FAILURE
    }
}

fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1);
    match arguments.next().as_deref().and_then(|argument| argument.to_str()) {
        Some("run") => match arguments.next() {
            Some(target) => run(Path::new(&target)),
            None => usage(),
        },
        Some("--version" | "-V") => {
            println!("pam-desktop {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => usage(),
    }
}
