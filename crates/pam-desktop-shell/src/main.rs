#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(feature = "gateway")]
#[cfg_attr(not(feature = "servo-engine"), allow(dead_code))]
mod event_hub;
#[cfg(feature = "gateway")]
#[cfg_attr(not(feature = "servo-engine"), allow(dead_code))]
mod gateway;
#[cfg(feature = "gateway")]
#[cfg_attr(not(feature = "servo-engine"), allow(dead_code))]
mod host_event;
#[cfg(feature = "gateway")]
#[cfg_attr(not(feature = "servo-engine"), allow(dead_code))]
mod native;
#[cfg(feature = "gateway")]
#[cfg_attr(not(feature = "servo-engine"), allow(dead_code))]
mod native_shell;
mod packager;
#[cfg(feature = "gateway")]
#[cfg_attr(not(feature = "servo-engine"), allow(dead_code))]
mod plugin;
mod plugin_scaffold;
mod project;
mod runtime;
#[cfg(feature = "gateway")]
#[cfg_attr(not(feature = "servo-engine"), allow(dead_code))]
mod scheduler;
mod updater;
#[cfg(feature = "gateway")]
#[cfg_attr(not(feature = "servo-engine"), allow(dead_code))]
mod watcher;
mod worker;

#[cfg(feature = "servo-engine")]
mod servo_engine;

use std::path::PathBuf;
use std::process::ExitCode;

use project::Project;
use runtime::DesktopRuntime;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("pam-desktop: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut arguments = std::env::args_os();
    let executable = arguments.next().unwrap_or_else(|| "pam-desktop".into());
    let Some(command) = arguments.next() else {
        print_usage(&executable);
        return Err("a command is required".to_owned());
    };

    match command.to_string_lossy().as_ref() {
        "--help" | "-h" => {
            print_usage(&executable);
            Ok(())
        }
        "--version" | "-V" => {
            println!("pam-desktop {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "dev" => {
            let project = arguments
                .next()
                .map_or_else(|| PathBuf::from("."), PathBuf::from);
            if let Some(unknown) = arguments.next() {
                return Err(format!(
                    "unexpected argument for dev: {}",
                    unknown.to_string_lossy()
                ));
            }
            run_desktop(Project::discover(&project)?, true)
        }
        "run" => {
            let project = arguments
                .next()
                .map_or_else(|| PathBuf::from("."), PathBuf::from);
            if let Some(unknown) = arguments.next() {
                return Err(format!(
                    "unexpected argument for run: {}",
                    unknown.to_string_lossy()
                ));
            }
            run_desktop(Project::discover(&project)?, false)
        }
        "build" => run_build(&executable, arguments.collect()),
        "plugin" => run_plugin(&executable, arguments.collect()),
        "update-key" => run_update_key(&arguments.collect::<Vec<_>>()),
        "publish-update" => run_publish_update(arguments.collect()),
        "apply-update" => updater::apply(updater::ApplyOptions::parse(arguments)?),
        "doctor" => {
            let project = arguments
                .next()
                .map_or_else(|| PathBuf::from("."), PathBuf::from);
            run_doctor(&project)
        }
        unknown => Err(format!("unknown command {unknown:?}")),
    }
}

fn run_plugin(
    executable: &std::ffi::OsStr,
    arguments: Vec<std::ffi::OsString>,
) -> Result<(), String> {
    match plugin_scaffold::PluginCommand::parse(arguments)? {
        plugin_scaffold::PluginCommand::Help => {
            plugin_scaffold::print_usage(executable);
            Ok(())
        }
        plugin_scaffold::PluginCommand::New { project, id } => {
            let directory = plugin_scaffold::scaffold(&project, &id)?;
            println!("[ok] Rust plugin scaffolded: {}", directory.display());
            println!(
                "[next] Implement it, then run: {} plugin build {} {}",
                executable.to_string_lossy(),
                id,
                project.display(),
            );
            Ok(())
        }
        plugin_scaffold::PluginCommand::Build { project, id } => {
            let output = plugin_scaffold::build(&project, &id)?;
            println!("[ok] Rust plugin built: {}", output.display());
            Ok(())
        }
    }
}

fn run_update_key(arguments: &[std::ffi::OsString]) -> Result<(), String> {
    let path = match arguments {
        [flag, path] if flag == "--output" || flag == "-o" => PathBuf::from(path),
        _ => {
            return Err("usage: pam-desktop update-key --output <private-key-file>".to_owned());
        }
    };
    let public_key = updater::generate_key(&path)?;
    println!("[ok] Private update key: {}", path.display());
    println!("[ok] Public update key: {public_key}");
    Ok(())
}

fn run_publish_update(mut arguments: Vec<std::ffi::OsString>) -> Result<(), String> {
    let has_project = arguments.first().is_some_and(|argument| {
        argument
            .to_str()
            .is_some_and(|argument| !argument.starts_with('-'))
    });
    let project = if has_project {
        PathBuf::from(arguments.remove(0))
    } else {
        PathBuf::from(".")
    };
    let options = updater::PublishOptions::parse(arguments)?;
    let runtime = DesktopRuntime::prepare(Project::discover(&project)?)?;
    let output = updater::publish_feed(&runtime.bootstrap().manifest, options)?;
    println!("[ok] Signed update feed: {}", output.display());
    Ok(())
}

fn run_build(
    executable: &std::ffi::OsStr,
    arguments: Vec<std::ffi::OsString>,
) -> Result<(), String> {
    match packager::BuildOptions::parse(arguments)? {
        packager::BuildCommand::Help => {
            packager::print_build_usage(executable);
            Ok(())
        }
        packager::BuildCommand::Build(options) => {
            let project = Project::discover(&options.project)?;
            let runtime = DesktopRuntime::prepare(project)?;
            let result = packager::build(runtime.project(), runtime.bootstrap(), &options)?;
            println!("[ok] Built {} artifact(s):", result.artifacts.len());
            for artifact in result.artifacts {
                println!("  {}", artifact.display());
            }
            Ok(())
        }
    }
}

fn run_doctor(path: &std::path::Path) -> Result<(), String> {
    let runtime = DesktopRuntime::prepare(Project::discover(path)?)?;
    println!(
        "[ok] PHP worker protocol: {}",
        pam_desktop_protocol::PROTOCOL_VERSION
    );
    println!("[ok] Entry: {}", runtime.entry().display());
    println!(
        "[ok] Windows: {} ({})",
        runtime.bootstrap().windows.len(),
        runtime
            .bootstrap()
            .windows
            .iter()
            .map(|window| window.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "[ok] Command timeout: {} ms",
        runtime.bootstrap().command_timeout_ms
    );
    let capabilities = &runtime.bootstrap().capabilities;
    let manifest = &runtime.bootstrap().manifest;
    println!(
        "[ok] Application: {} {} ({})",
        manifest.name, manifest.version, manifest.identifier
    );
    println!(
        "[ok] Package: category={}, icon={}",
        manifest.category as u8, manifest.icon
    );
    if let Some(updates) = &manifest.updates {
        println!(
            "[ok] Updates: policy={}, channel={}, endpoint={}",
            updates.policy as u8, updates.channel, updates.endpoint
        );
    } else {
        println!("[ok] Updates: disabled");
    }
    println!(
        "[ok] Filesystem roots: {} ({})",
        capabilities.filesystem_roots.len(),
        capabilities
            .filesystem_roots
            .iter()
            .map(|root| format!("{}:{}", root.name, root.access as u8))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "[ok] Native capabilities: dialogs={}, clipboard-read={}, clipboard-write={}, notifications={}, drag-and-drop={}",
        capabilities.dialogs,
        capabilities.clipboard_read,
        capabilities.clipboard_write,
        capabilities.notifications,
        capabilities.drag_and_drop,
    );
    println!(
        "[ok] Native shell: menus={}, tray={}, global-shortcuts={}",
        runtime.bootstrap().shell.menus.len(),
        runtime.bootstrap().shell.tray.is_some(),
        runtime.bootstrap().shell.shortcuts.len(),
    );
    println!(
        "[ok] Extensions: PHP plugins={}, PHP background-jobs={}, Rust plugins={}",
        runtime.bootstrap().php_plugins.len(),
        runtime.bootstrap().background_jobs.len(),
        runtime.bootstrap().rust_plugins.len(),
    );
    println!(
        "[ok] PHP worker generation: {}",
        runtime.worker_generation()
    );
    print_engine_diagnostic();
    Ok(())
}

#[cfg(feature = "servo-engine")]
fn run_desktop(project: Project, watch: bool) -> Result<(), String> {
    servo_engine::run(DesktopRuntime::prepare(project)?, watch)
}

#[cfg(not(feature = "servo-engine"))]
fn run_desktop(_project: Project, _watch: bool) -> Result<(), String> {
    Err("this binary was built without the servo-engine feature".to_owned())
}

#[cfg(feature = "servo-engine")]
fn print_engine_diagnostic() {
    println!("[ok] Servo engine: 0.4.0");
}

#[cfg(not(feature = "servo-engine"))]
fn print_engine_diagnostic() {
    println!("[warn] Servo engine: disabled in this validation build");
}

fn print_usage(executable: &std::ffi::OsStr) {
    println!(
        "Usage: {} dev [directory]\n       {} run [directory]\n       {} build [directory] [options]\n       {} plugin new <id> [directory]\n       {} plugin build <id> [directory]\n       {} update-key --output <private-key-file>\n       {} publish-update [directory] [options]\n       {} doctor [directory]\n       {} --version",
        executable.to_string_lossy(),
        executable.to_string_lossy(),
        executable.to_string_lossy(),
        executable.to_string_lossy(),
        executable.to_string_lossy(),
        executable.to_string_lossy(),
        executable.to_string_lossy(),
        executable.to_string_lossy(),
        executable.to_string_lossy(),
    );
}
