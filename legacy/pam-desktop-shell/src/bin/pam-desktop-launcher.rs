#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    match launch() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => ExitCode::from(
            status
                .code()
                .and_then(|code| u8::try_from(code).ok())
                .unwrap_or(1),
        ),
        Err(error) => {
            log_failure(&error);
            ExitCode::FAILURE
        }
    }
}

fn launch() -> Result<std::process::ExitStatus, String> {
    let launcher = std::env::current_exe()
        .map_err(|error| format!("cannot locate application launcher: {error}"))?
        .canonicalize()
        .map_err(|error| format!("cannot resolve application launcher: {error}"))?;
    let bundle = launcher
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| "application launcher is outside a Pam Desktop bundle".to_owned())?;
    let suffix = std::env::consts::EXE_SUFFIX;
    let host = bundle.join("bin").join(format!("pam-desktop{suffix}"));
    let pam = bundle.join("bin").join(format!("pam{suffix}"));
    let app = bundle.join("app");
    if !host.is_file() || !pam.is_file() || !app.is_dir() {
        return Err("the Pam Desktop bundle is incomplete".to_owned());
    }

    let mut command = Command::new(&host);
    command
        .arg("run")
        .arg(&app)
        .args(forwarded_arguments(std::env::args_os()))
        .current_dir(&app)
        .env("PAM_BINARY", &pam)
        .env("PAM_DESKTOP_BUNDLE", "1")
        .env(
            "PAM_DESKTOP_BUNDLE_ROOT",
            std::env::var_os("PAM_DESKTOP_UPDATE_ROOT")
                .map_or_else(|| bundle.to_path_buf(), PathBuf::from),
        )
        .env("PAM_DESKTOP_LAUNCHER", &launcher)
        .env("PHPRC", bundle.join("etc").join("php.ini"))
        .env("PHP_INI_SCAN_DIR", "");
    prepend_library_path(&mut command, bundle.join("lib"));
    command
        .status()
        .map_err(|error| format!("cannot start Pam Desktop: {error}"))
}

fn forwarded_arguments(
    arguments: impl IntoIterator<Item = std::ffi::OsString>,
) -> impl Iterator<Item = std::ffi::OsString> {
    arguments.into_iter().skip(1)
}

fn prepend_library_path(command: &mut Command, library: PathBuf) {
    let variable = if cfg!(target_os = "windows") {
        "PATH"
    } else if cfg!(target_os = "macos") {
        "DYLD_LIBRARY_PATH"
    } else {
        "LD_LIBRARY_PATH"
    };
    let mut paths = vec![library];
    if let Some(existing) = std::env::var_os(variable) {
        paths.extend(std::env::split_paths(&existing));
    }
    if let Ok(value) = std::env::join_paths(paths) {
        command.env(variable, value);
    }
}

fn log_failure(error: &str) {
    let path = std::env::temp_dir().join("pam-desktop-launcher.log");
    let _ = std::fs::write(path, format!("pam-desktop: {error}\n"));
    #[cfg(not(target_os = "windows"))]
    eprintln!("pam-desktop: {error}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_deep_links_files_and_quick_actions_to_the_host() {
        let arguments = [
            "launcher",
            "pam://open/item",
            "/tmp/example.pam",
            "--pam-quick-action=compose",
        ]
        .into_iter()
        .map(std::ffi::OsString::from)
        .collect::<Vec<_>>();
        assert_eq!(
            forwarded_arguments(arguments)
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            [
                "pam://open/item",
                "/tmp/example.pam",
                "--pam-quick-action=compose",
            ]
        );
    }
}
