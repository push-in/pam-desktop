//! `pam-desktop`: the native host of PAM Desktop applications.

// A release build is a windowed program on Windows: see `console`.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod bundle;
mod console;
#[cfg(target_os = "linux")]
mod dev_links;
mod http_client;
mod launcher_icon;
mod manifest;
mod modules;
mod packager;
mod paths;
mod pe;
mod runtime_install;
mod scaffold;
mod updater;
// Used on Windows; its parsing and checks are tested everywhere.
#[cfg_attr(not(windows), allow(dead_code))]
mod windows_php;

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
         pam-desktop init [directory]          write the files of a new application\n  \
         pam-desktop dev [app] [url] [--yes]   run with hot reload (default: current directory)\n  \
         pam-desktop run [app] [url] [--yes]   run the application, opened with a link\n  \
                                               (--yes: install PHP without asking when none is found)\n  \
         pam-desktop build [app] [options]     bundle the application and build its installers\n  \
         pam-desktop update-key --output FILE  create the key that signs updates\n  \
         pam-desktop publish-update [app] ...  sign the update feed of a release\n  \
         pam-desktop --version",
        env!("CARGO_PKG_VERSION"),
    );
    ExitCode::from(64)
}

/// The PHP launcher: `PAM_DESKTOP_PHP`, then `pam exec` (on `PATH` or where
/// the PAM installer puts it), then `php`. When none exists on Unix, the PAM
/// runtime, which embeds PHP, is downloaded and checked against its signed
/// release manifest (`runtime_install`). PAM has no Windows release: there
/// the official PHP of windows.php.net is used, checked against a pinned and
/// the published SHA-256 (`windows_php`). Both ask first (`--yes` or
/// `PAM_DESKTOP_AUTO_INSTALL=1` agree beforehand).
fn php_command() -> (PathBuf, Vec<OsString>) {
    if let Some(php) = std::env::var_os("PAM_DESKTOP_PHP") {
        return (PathBuf::from(php), Vec::new());
    }
    let on_path = |name: &str| {
        std::env::var_os("PATH").and_then(|paths| {
            // Relative entries (`.`, an empty one) would run a `php`/`pam`
            // planted in the checkout of the application being opened.
            std::env::split_paths(&paths)
                .filter(|directory| directory.is_absolute())
                .map(|directory| directory.join(name))
                .find(|candidate| candidate.is_file())
        })
    };
    let executable = |name: &str| {
        if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_owned()
        }
    };
    // `pam` found on `PATH` inside a PAM installation (`pam desktop dev`
    // puts the installation's `bin` first) is the bare binary: its `pam-run`
    // launcher sets up the private libraries it needs, as `build` does.
    let pam = |path: PathBuf| {
        let launcher = path.with_file_name("pam-run");
        let path = if path.file_name().is_some_and(|name| name == "pam") && launcher.is_file() {
            launcher
        } else {
            path
        };
        (path, vec![OsString::from("exec")])
    };
    if cfg!(windows) {
        let checked = |php: PathBuf| {
            if let Some(problem) = windows_php::missing_runtime(&php) {
                eprintln!("pam-desktop: {problem}");
            }
            (php, Vec::new())
        };
        if let Some(php) = windows_php::installed() {
            return checked(php);
        }
        if let Some(php) = on_path("php.exe") {
            return checked(php);
        }
        return match windows_php::install() {
            Ok(php) => checked(php),
            Err(error) => {
                eprintln!(
                    "pam-desktop: could not install PHP {}: {error}; install PHP {0} (NTS x64) or set PAM_DESKTOP_PHP to its php.exe",
                    windows_php::BRANCH
                );
                (PathBuf::from("php.exe"), Vec::new())
            }
        };
    }
    if let Some(found) = on_path(&executable("pam")) {
        return pam(found);
    }
    let installed = installed_pam();
    if let Some(found) = installed.as_ref().filter(|path| path.is_file()) {
        return pam(found.clone());
    }
    if let Some(php) = on_path(&executable("php")) {
        return (php, Vec::new());
    }
    match installed {
        Some(target) if install_pam(&target) && target.is_file() => pam(target),
        _ => (PathBuf::from(executable("php")), Vec::new()),
    }
}

/// The OPcache settings that keep compiled scripts in `directory`, created
/// here; none when it cannot be created or named in an INI value.
fn opcache_file_cache(directory: &Path) -> Option<[String; 3]> {
    let path = directory.to_str()?;
    if path.contains(['"', '\n', '\r', '\0', '$']) || std::fs::create_dir_all(directory).is_err() {
        return None;
    }
    Some([
        "opcache.enable_cli=1".to_owned(),
        format!("opcache.file_cache=\"{path}\""),
        "opcache.file_cache_only=1".to_owned(),
    ])
}

/// Where the PAM installer links `pam`: `$XDG_BIN_HOME` or `~/.local/bin`.
fn installed_pam() -> Option<PathBuf> {
    if cfg!(windows) {
        return None;
    }
    std::env::var_os("XDG_BIN_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/bin")))
        .map(|directory| directory.join("pam"))
}

/// Installs the PAM runtime, verified against its signed release manifest,
/// and links `pam` at `target` (`runtime_install`).
fn install_pam(target: &Path) -> bool {
    match runtime_install::install_pam(target) {
        Ok(_) => true,
        Err(error) => {
            eprintln!("pam-desktop: could not install the PAM runtime: {error}");
            eprintln!("pam-desktop: install PHP 8.5 or PAM, or set PAM_DESKTOP_PHP");
            false
        }
    }
}

/// A canonical path in the form PHP takes. On Windows `canonicalize` gives
/// verbatim paths (`\\?\C:\app`), which PHP's `realpath`, `chdir` and
/// includes do not resolve: the prefix goes when the path does not need it.
pub(crate) fn plain_path(path: PathBuf) -> PathBuf {
    if !cfg!(windows) {
        return path;
    }
    match path.to_str().and_then(without_verbatim_prefix) {
        Some(plain) => PathBuf::from(plain),
        None => path,
    }
}

/// `\\?\C:\a` is `C:\a` and `\\?\UNC\host\share` is `\\host\share`, while
/// shorter than `MAX_PATH`; `None` when the path is not verbatim or needs it.
fn without_verbatim_prefix(text: &str) -> Option<String> {
    let plain = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        text.strip_prefix(r"\\?\")
            .filter(|rest| rest.as_bytes().get(1) == Some(&b':'))?
            .to_owned()
    };
    (plain.len() < 260).then_some(plain)
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
    let directory = target
        .is_dir()
        .then(|| target.canonicalize().ok().map(plain_path))
        .flatten();
    let entry = match &directory {
        Some(directory) => directory.join(Manifest::load(directory).entry()),
        None => target.to_path_buf(),
    };
    let Ok(entry) = entry.canonicalize().map(plain_path) else {
        eprintln!("pam-desktop: {} does not exist", entry.display());
        if directory
            .as_deref()
            .is_some_and(|directory| !directory.join(manifest::FILE_NAME).is_file())
        {
            eprintln!(
                "pam-desktop: no {} here or in a parent folder; create an application with `pam-desktop init`",
                manifest::FILE_NAME
            );
        }
        return ExitCode::from(66);
    };
    let project =
        directory.unwrap_or_else(|| entry.parent().map(Path::to_path_buf).unwrap_or_default());
    let Some(bootstrap) = worker_bootstrap(&project) else {
        eprintln!(
            "pam-desktop: the pushinbr/pam-desktop package is not installed in {}",
            project.display()
        );
        return ExitCode::from(69);
    };
    let (program, mut arguments) = php_command();
    // `pam exec` (and a bundle's launcher, which runs it) takes PHP settings
    // from PAM_INI_ENTRIES; a plain `php` found here takes `-d` flags.
    let plain_php = arguments.is_empty() && std::env::var_os("PAM_DESKTOP_PHP").is_none();
    // The runtime PAM Desktop installed on Windows turns on common
    // extensions; the others the application lists are turned on here.
    if windows_php::is_managed(&program) {
        let listed: Vec<String> = Manifest::load(&project)
            .document()
            .get("extensions")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .filter(|name| {
                !name.is_empty()
                    && name.len() <= 64
                    && name.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                    })
            })
            .map(str::to_owned)
            .collect();
        arguments.extend(windows_php::extension_arguments(&program, &listed));
    }
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
    #[cfg(target_os = "linux")]
    let _dev_link_registration = if development && link.is_none() {
        match manifest.url_schemes() {
            Ok(schemes) if !schemes.is_empty() => {
                match dev_links::Registration::install(&application_id, &project, &schemes) {
                    Ok(registration) => Some(registration),
                    Err(error) => {
                        eprintln!("pam-desktop: cannot register development URL schemes: {error}");
                        None
                    }
                }
            }
            _ => None,
        }
    } else {
        None
    };
    let mut environment = vec![
        (
            "PAM_DESKTOP_PROJECT_ROOT".into(),
            project.display().to_string(),
        ),
        ("PAM_DESKTOP_APPLICATION_ID".into(), application_id.clone()),
    ];
    // Navigation state, hot reload checkpoints and module storage are per application.
    let app_paths = if development {
        AppPaths::for_development(&project)
    } else {
        AppPaths::for_application(&application_id)
    }
    .with_overrides();
    if let Err(error) = app_paths.create() {
        eprintln!("pam-desktop: cannot create the application directories: {error}");
        return ExitCode::from(73);
    }
    environment.extend(app_paths.environment());
    // PHP's curl and OpenSSL have no certificate store on Windows: the PHP
    // PAM Desktop installed and the one of a bundle trust the roots of the
    // Windows store and Mozilla's (`windows_php::ca_bundle`). Another PHP
    // keeps its own configuration.
    let bundled = std::env::var_os("PAM_DESKTOP_BUNDLE").is_some_and(|value| value == "1");
    if cfg!(windows)
        && (bundled || windows_php::is_managed(&program))
        && let Some(bundle) = windows_php::ca_bundle(&app_paths.cache)
    {
        arguments.splice(0..0, windows_php::ca_arguments(&bundle));
        environment.push((
            "PAM_DESKTOP_PHP_CA_FILE".into(),
            bundle.display().to_string(),
        ));
    }
    // Compiled PHP is kept between launches in the cache directory (OPcache
    // file cache, no shared memory segment): later launches skip compiling
    // the application and the framework.
    if let Some(settings) = opcache_file_cache(&app_paths.cache.join("opcache")) {
        let mut entries = settings.join("\n");
        if let Ok(own) = std::env::var("PAM_INI_ENTRIES")
            && !own.is_empty()
        {
            entries.push('\n');
            entries.push_str(&own);
        }
        environment.push(("PAM_INI_ENTRIES".into(), entries));
        if plain_php {
            let flags = settings
                .iter()
                .flat_map(|setting| [OsString::from("-d"), OsString::from(setting)]);
            arguments.splice(0..0, flags);
        }
    }
    let http_client: Arc<dyn pam_desktop_ui::modules::fetch::HttpClient> =
        Arc::new(UreqClient::new());
    let config = RuntimeConfig {
        worker: WorkerCommand {
            program,
            arguments,
            entry,
            working_directory: Some(project.clone()),
            environment,
        },
        window: manifest.window(),
        platform,
        development,
        watch: if development {
            vec![project.clone()]
        } else {
            Vec::new()
        },
        directories: Directories {
            data: app_paths.data.clone(),
            cache: app_paths.cache.clone(),
            state: app_paths.state.clone(),
            assets: project.clone(),
        },
        app_version: manifest.text("version").unwrap_or_default(),
        build_number: manifest.text("build").unwrap_or_default(),
        launch_url: link
            .or_else(|| std::env::var("PAM_DESKTOP_LAUNCH_URL").ok())
            .filter(|url| !url.is_empty()),
        single_instance: manifest.single_instance(development),
        modules: modules::host_modules(
            &app_paths.data,
            &app_paths.cache,
            &application_id,
            Arc::clone(&http_client),
            development,
            &manifest,
        ),
        http_client: Some(http_client),
        script: std::env::var_os("PAM_DESKTOP_SCRIPT")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from),
    };

    #[cfg(target_os = "linux")]
    let code = pam_desktop_gtk::run(&application_id, config);
    #[cfg(target_os = "macos")]
    let code = pam_desktop_appkit::run(&application_id, config);
    #[cfg(target_os = "windows")]
    let code = pam_desktop_win32::run(&application_id, config);
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        // A test script that failed fails the run.
        if pam_desktop_ui::driver::failed() {
            return ExitCode::FAILURE;
        }
        code
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
            for note in result.notes {
                println!("[info] {note}");
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
    let has_project = arguments.first().is_some_and(|argument| {
        argument
            .to_str()
            .is_some_and(|argument| !argument.starts_with('-'))
    });
    let project = if has_project {
        PathBuf::from(arguments.remove(0))
    } else {
        manifest::current_project()
    };
    let options = updater::PublishOptions::parse(arguments)?;
    let project = project
        .canonicalize()
        .map_err(|error| format!("cannot resolve {}: {error}", project.display()))?;
    let application = Manifest::read(&project)?
        .application()
        .map_err(|error| format!("{}: {error}", project.join(manifest::FILE_NAME).display()))?;
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
    match arguments
        .next()
        .as_deref()
        .and_then(|argument| argument.to_str())
    {
        Some(command @ ("run" | "dev")) => {
            let mut arguments = runtime_install::take_assume_yes(arguments).into_iter();
            let target = arguments
                .next()
                .map_or_else(manifest::current_project, PathBuf::from);
            let link = arguments.next().and_then(|link| link.into_string().ok());
            run(&target, command == "dev", link)
        }
        Some("init") => report(
            scaffold::InitOptions::parse(arguments)
                .and_then(|options| scaffold::init(&options))
                .map(|lines| {
                    for line in lines {
                        println!("{line}");
                    }
                }),
        ),
        Some("build") => report(run_build(arguments)),
        Some("update-key") => report(run_update_key(&arguments.collect::<Vec<_>>())),
        Some("publish-update") => report(run_publish_update(arguments.collect())),
        // Private: the running application starts it from a copy of the host.
        Some("apply-update") => {
            report(updater::ApplyOptions::parse(arguments).and_then(updater::apply))
        }
        Some("--version" | "-V") => {
            println!("pam-desktop {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => usage(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn verbatim_paths_become_plain_while_short() {
        use super::without_verbatim_prefix as plain;
        assert_eq!(
            plain(r"\\?\C:\app\index.php").as_deref(),
            Some(r"C:\app\index.php")
        );
        assert_eq!(
            plain(r"\\?\UNC\host\share\app").as_deref(),
            Some(r"\\host\share\app")
        );
        assert_eq!(plain(r"C:\app"), None);
        assert_eq!(plain(r"\\?\Volume{x}\app"), None);
        assert_eq!(plain(&format!(r"\\?\C:\{}", "a".repeat(300))), None);
    }
}
