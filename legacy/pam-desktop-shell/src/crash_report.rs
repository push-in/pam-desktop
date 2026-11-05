use std::backtrace::Backtrace;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

const MAX_REPORTS: usize = 8;
const MAX_BACKTRACE_BYTES: usize = 256 * 1024;
static INSTALL: Once = Once::new();

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CrashReport<'a> {
    schema_version: u8,
    surface_code: u8,
    application_id: &'a str,
    host_version: &'static str,
    process_id: u32,
    captured_at_unix_ms: u64,
    thread: String,
    location: Option<CrashLocation<'a>>,
    backtrace: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CrashLocation<'a> {
    file: &'a str,
    line: u32,
    column: u32,
}

pub fn install(application_id: &str) -> Result<(), String> {
    let directory = report_directory(application_id)?;
    fs::create_dir_all(&directory)
        .map_err(|error| format!("cannot create crash report directory: {error}"))?;
    let application_id = application_id.to_owned();
    INSTALL.call_once(move || {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |information| {
            let _ = write_report(&directory, &application_id, information);
            previous(information);
        }));
    });
    Ok(())
}

fn write_report(
    directory: &Path,
    application_id: &str,
    information: &std::panic::PanicHookInfo<'_>,
) -> Result<(), String> {
    let captured_at_unix_ms = unix_milliseconds();
    let mut backtrace = Backtrace::force_capture().to_string();
    if backtrace.len() > MAX_BACKTRACE_BYTES {
        backtrace.truncate(MAX_BACKTRACE_BYTES);
    }
    let report = CrashReport {
        schema_version: 1,
        surface_code: 3,
        application_id,
        host_version: env!("CARGO_PKG_VERSION"),
        process_id: std::process::id(),
        captured_at_unix_ms,
        thread: std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .chars()
            .take(80)
            .collect(),
        location: information.location().map(|location| CrashLocation {
            file: location.file(),
            line: location.line(),
            column: location.column(),
        }),
        backtrace,
    };
    let bytes = serde_json::to_vec_pretty(&report)
        .map_err(|error| format!("cannot encode crash report: {error}"))?;
    let path = directory.join(format!(
        "crash-{captured_at_unix_ms}-{}.json",
        std::process::id(),
    ));
    let temporary = path.with_extension("json.tmp");
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options
        .open(&temporary)
        .map_err(|error| format!("cannot create crash report: {error}"))?;
    output
        .write_all(&bytes)
        .and_then(|()| output.sync_all())
        .map_err(|error| format!("cannot persist crash report: {error}"))?;
    fs::rename(&temporary, &path)
        .map_err(|error| format!("cannot publish crash report: {error}"))?;
    prune(directory)
}

fn prune(directory: &Path) -> Result<(), String> {
    let mut reports = fs::read_dir(directory)
        .map_err(|error| format!("cannot list crash reports: {error}"))?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_name().to_str().is_some_and(|name| {
                name.starts_with("crash-")
                    && Path::new(name)
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
            })
        })
        .collect::<Vec<_>>();
    reports.sort_by_key(std::fs::DirEntry::file_name);
    let remove = reports.len().saturating_sub(MAX_REPORTS);
    for report in reports.into_iter().take(remove) {
        let _ = fs::remove_file(report.path());
    }
    Ok(())
}

fn report_directory(application_id: &str) -> Result<PathBuf, String> {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
            })
    }
    .ok_or_else(|| "cannot locate operating-system state directory for crash reports".to_owned())?;
    Ok(base
        .join("pam-desktop")
        .join(application_id)
        .join("crashes"))
}

fn unix_milliseconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}
