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
mod project;
mod runtime;
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
            run_desktop(Project::discover(&project)?)
        }
        "doctor" => {
            let project = arguments
                .next()
                .map_or_else(|| PathBuf::from("."), PathBuf::from);
            let project = Project::discover(&project)?;
            let runtime = DesktopRuntime::prepare(project)?;
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
                "[ok] PHP worker generation: {}",
                runtime.worker_generation()
            );
            print_engine_diagnostic();
            Ok(())
        }
        unknown => Err(format!("unknown command {unknown:?}")),
    }
}

#[cfg(feature = "servo-engine")]
fn run_desktop(project: Project) -> Result<(), String> {
    servo_engine::run(DesktopRuntime::prepare(project)?)
}

#[cfg(not(feature = "servo-engine"))]
fn run_desktop(_project: Project) -> Result<(), String> {
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
        "Usage: {} dev [directory]\n       {} doctor [directory]\n       {} --version",
        executable.to_string_lossy(),
        executable.to_string_lossy(),
        executable.to_string_lossy(),
    );
}
