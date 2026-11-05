//! `pam-desktop`: the native host of PAM Desktop applications.

// A release build is a windowed program on Windows: see `console`.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

mod bundle;
mod console;
mod http_client;
mod manifest;
mod modules;
mod packager;
mod paths;
mod updater;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use crate::http_client::UreqClient;
use crate::manifest::Manifest;
use crate::paths::AppPaths;
use pam_desktop_ui::runtime::{Directories, RuntimeConfig};
use pam_desktop_ui::worker::WorkerCommand;

fn usage() -> ExitCode {
    eprintln!(
        "PAM Desktop {}\n\n\
         usage:\n  \
         pam-desktop dev [app] [url]           run with hot reload (default: current directory)\n  \
         pam-desktop run [app] [url]           run the application, opened with a link\n  \
         pam-desktop build [app] [options]     bundle the application and build its installers\n  \
         pam-desktop update-key --output FILE  create the key that signs updates\n  \
         pam-desktop publish-update [app] ...  sign the update feed of a release\n  \
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

/// Runs the application at `target`: a project directory, started with the
/// `entry` of its manifest (`index.php` by default), or a script, whose
/// directory is the project. `link` is the URL the application was opened
/// with, when the system passed one.
fn run(target: &Path, development: bool, link: Option<String>) -> ExitCode {
    let directory = target.is_dir().then(|| target.canonicalize().ok()).flatten();
    let entry = match &directory {
        Some(directory) => directory.join(Manifest::load(directory).entry()),
        None => target.to_path_buf(),
    };
    let Ok(entry) = entry.canonicalize() else {
        eprintln!("pam-desktop: {} does not exist", entry.display());
        return ExitCode::from(66);
    };
    let project = directory.unwrap_or_else(|| entry.parent().map(Path::to_path_buf).unwrap_or_default());
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
    let manifest = Manifest::load(&project);
    let application_id = manifest.application_id();
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
    let http_client: Arc<dyn pam_desktop_ui::modules::fetch::HttpClient> = Arc::new(UreqClient::new());
    let config = RuntimeConfig {
        worker: WorkerCommand { program, arguments, entry, working_directory: Some(project.clone()), environment },
        window: manifest.window(),
        platform,
        development,
        watch: if development { vec![project.clone()] } else { Vec::new() },
        directories: Directories { data: app_paths.data.clone(), cache: app_paths.cache.clone(), state: app_paths.state.clone(), assets: project.clone() },
        app_version: manifest.text("version").unwrap_or_default(),
        build_number: manifest.text("build").unwrap_or_default(),
        launch_url: link.or_else(|| std::env::var("PAM_DESKTOP_LAUNCH_URL").ok()).filter(|url| !url.is_empty()),
        single_instance: manifest.single_instance(development),
        modules: modules::host_modules(&app_paths.data, &app_paths.cache, &application_id, Arc::clone(&http_client), development, &manifest),
        http_client: Some(http_client),
    };

    #[cfg(target_os = "linux")]
    {
        pam_desktop_gtk::run(&application_id, config)
    }
    #[cfg(target_os = "macos")]
    {
        pam_desktop_appkit::run(&application_id, config)
    }
    #[cfg(target_os = "windows")]
    {
        pam_desktop_win32::run(&application_id, config)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = (config, application_id);
        eprintln!("pam-desktop: this platform is not supported");
        ExitCode::FAILURE
    }
}

/// The outcome of a command that prints what it did.
fn report(result: Result<(), String>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("pam-desktop: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_build(arguments: impl Iterator<Item = OsString>) -> Result<(), String> {
    match packager::BuildOptions::parse(arguments)? {
        packager::BuildCommand::Help => packager::print_build_usage(),
        packager::BuildCommand::Build(options) => {
            let result = packager::build(&options)?;
            println!("[ok] Built {} artifact(s):", result.artifacts.len());
            for artifact in result.artifacts {
                println!("  {}", artifact.display());
            }
        }
    }
    Ok(())
}

fn run_update_key(arguments: &[OsString]) -> Result<(), String> {
    let path = match arguments {
        [flag, path] if flag == "--output" || flag == "-o" => PathBuf::from(path),
        _ => return Err("usage: pam-desktop update-key --output <private-key-file>".to_owned()),
    };
    let public_key = updater::generate_key(&path)?;
    println!("[ok] Private update key: {}", path.display());
    println!("[ok] Public update key: {public_key}");
    Ok(())
}

fn run_publish_update(mut arguments: Vec<OsString>) -> Result<(), String> {
    let has_project = arguments.first().is_some_and(|argument| argument.to_str().is_some_and(|argument| !argument.starts_with('-')));
    let project = if has_project { PathBuf::from(arguments.remove(0)) } else { PathBuf::from(".") };
    let options = updater::PublishOptions::parse(arguments)?;
    let project = project.canonicalize().map_err(|error| format!("cannot resolve {}: {error}", project.display()))?;
    let application =
        Manifest::read(&project)?.application().map_err(|error| format!("{}: {error}", project.join(manifest::FILE_NAME).display()))?;
    let output = updater::publish_feed(&application, options)?;
    println!("[ok] Signed update feed: {}", output.display());
    Ok(())
}

fn main() -> ExitCode {
    console::attach();
    let mut arguments = std::env::args_os().skip(1);
    // Inside a Windows bundle the launcher is this host under the name of
    // the application: whatever it was started with belongs to the application.
    if let Some(launch) = std::env::current_exe().ok().and_then(|executable| {
        let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
        bundle::Launch::for_executable(&executable, &arguments)
    }) {
        return launch.run();
    }
    match arguments.next().as_deref().and_then(|argument| argument.to_str()) {
        Some(command @ ("run" | "dev")) => {
            let target = arguments.next().map_or_else(|| PathBuf::from("."), PathBuf::from);
            let link = arguments.next().and_then(|link| link.into_string().ok());
            run(&target, command == "dev", link)
        }
        Some("build") => report(run_build(arguments)),
        Some("update-key") => report(run_update_key(&arguments.collect::<Vec<_>>())),
        Some("publish-update") => report(run_publish_update(arguments.collect())),
        // Private: the running application starts it from a copy of the host.
        Some("apply-update") => report(updater::ApplyOptions::parse(arguments).and_then(updater::apply)),
        Some("--version" | "-V") => {
            println!("pam-desktop {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => usage(),
    }
}
