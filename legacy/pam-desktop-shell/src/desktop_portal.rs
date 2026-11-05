#[cfg(target_os = "linux")]
use std::os::fd::AsFd;
#[cfg(target_os = "linux")]
use std::{
    fs,
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};

#[cfg(target_os = "linux")]
use ashpd::desktop::camera::Camera;
#[cfg(target_os = "linux")]
use ashpd::desktop::open_uri::OpenFileRequest;
#[cfg(target_os = "linux")]
use ashpd::desktop::print::{PageSetup, PrintProxy, Settings};
#[cfg(target_os = "linux")]
use ashpd::desktop::screenshot::Screenshot;
#[cfg(target_os = "linux")]
use pam_desktop_protocol::FileAccess;
use pam_desktop_protocol::{DesktopPortalOperation, ScannerImageFormat};
use serde::Deserialize;
use serde_json::Value;
#[cfg(target_os = "linux")]
use serde_json::json;
#[cfg(target_os = "linux")]
use url::Url;

use crate::native::{FileTarget, NativeError, NativeServices};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopPortalRequest {
    pub operation: DesktopPortalOperation,
    pub window_id: String,
    pub url: Option<String>,
    pub target: Option<FileTarget>,
    #[serde(default = "default_title")]
    pub title: String,
    pub device: Option<String>,
    pub resolution: Option<u16>,
    pub format: Option<ScannerImageFormat>,
}

#[cfg(target_os = "linux")]
pub async fn execute(
    native: Arc<NativeServices>,
    request: &DesktopPortalRequest,
) -> Result<Value, NativeError> {
    if !native.desktop_portal_enabled() {
        return Err(NativeError::disabled("desktop portal"));
    }
    match request.operation {
        DesktopPortalOperation::OpenUri => open_uri(request).await,
        DesktopPortalOperation::Screenshot => screenshot(&native).await,
        DesktopPortalOperation::PrintPdf => {
            if request.title.is_empty() || request.title.len() > 256 {
                return Err(NativeError::invalid(
                    "Print titles must contain 1 to 256 bytes.",
                ));
            }
            let target = request
                .target
                .as_ref()
                .ok_or_else(|| NativeError::invalid("Printing requires a PDF file target."))?;
            if !target.path.to_ascii_lowercase().ends_with(".pdf") {
                return Err(NativeError::invalid(
                    "The print portal accepts PDF targets only.",
                ));
            }
            let (file, _) = native.open_read_stream(target)?;
            let proxy = PrintProxy::new().await.map_err(|error| {
                NativeError::native("Cannot connect to the XDG print portal", error)
            })?;
            let prepared = proxy
                .prepare_print(
                    None,
                    &request.title,
                    Settings::default(),
                    PageSetup::default(),
                    None,
                    true,
                )
                .await
                .map_err(|error| NativeError::native("Cannot prepare the XDG print dialog", error))?
                .response()
                .map_err(|error| NativeError::native("The XDG print dialog failed", error))?;
            proxy
                .print(
                    None,
                    &request.title,
                    &file.as_fd(),
                    Some(prepared.token),
                    true,
                )
                .await
                .map_err(|error| NativeError::native("Cannot submit the PDF to XDG print", error))?
                .response()
                .map_err(|error| NativeError::native("The XDG print request failed", error))?;
            Ok(json!({"submitted": true}))
        }
        DesktopPortalOperation::CameraStatus => camera_status().await,
        DesktopPortalOperation::RequestCamera => request_camera().await,
        DesktopPortalOperation::ListScanners => tokio::task::spawn_blocking(list_scanners)
            .await
            .map_err(|error| NativeError::native("Scanner discovery task failed", error))?,
        DesktopPortalOperation::ScanImage => scan_request(native, request).await,
    }
}

#[cfg(target_os = "linux")]
async fn open_uri(request: &DesktopPortalRequest) -> Result<Value, NativeError> {
    let url = Url::parse(
        request
            .url
            .as_deref()
            .ok_or_else(|| NativeError::invalid("Opening a URI requires url."))?,
    )
    .map_err(|error| NativeError::invalid(format!("The desktop URI is invalid: {error}")))?;
    if !matches!(url.scheme(), "https" | "mailto" | "tel")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(NativeError::permission(
            "Desktop URIs are limited to credential-free https, mailto and tel URLs.",
        ));
    }
    OpenFileRequest::default()
        .ask(false)
        .send_uri(&url)
        .await
        .map_err(|error| NativeError::native("Cannot open the URI through XDG portal", error))?
        .response()
        .map_err(|error| NativeError::native("The XDG open request failed", error))?;
    Ok(Value::Null)
}

#[cfg(target_os = "linux")]
async fn screenshot(native: &NativeServices) -> Result<Value, NativeError> {
    let screenshot = Screenshot::request()
        .interactive(true)
        .modal(true)
        .send()
        .await
        .map_err(|error| NativeError::native("Cannot request an XDG screenshot", error))?
        .response()
        .map_err(|error| NativeError::native("The XDG screenshot request failed", error))?;
    let path = screenshot.uri().to_file_path().map_err(|()| {
        NativeError::native("XDG screenshot returned a non-file URI", screenshot.uri())
    })?;
    let reference = native.grant_path(&path, FileAccess::Read)?;
    serde_json::to_value(reference)
        .map_err(|error| NativeError::native("Cannot encode screenshot grant", error))
}

#[cfg(target_os = "linux")]
async fn camera_status() -> Result<Value, NativeError> {
    let camera = Camera::new()
        .await
        .map_err(|error| NativeError::native("Cannot connect to the XDG camera portal", error))?;
    let present = camera
        .is_present()
        .await
        .map_err(|error| NativeError::native("Cannot query camera availability", error))?;
    Ok(json!({"present": present}))
}

#[cfg(target_os = "linux")]
async fn request_camera() -> Result<Value, NativeError> {
    let camera = Camera::new()
        .await
        .map_err(|error| NativeError::native("Cannot connect to the XDG camera portal", error))?;
    if !camera
        .is_present()
        .await
        .map_err(|error| NativeError::native("Cannot query camera availability", error))?
    {
        return Ok(json!({"granted": false, "present": false}));
    }
    camera
        .request_access()
        .await
        .map_err(|error| NativeError::native("Cannot request camera access", error))?
        .response()
        .map_err(|error| NativeError::permission(format!("Camera access was denied: {error}")))?;
    Ok(json!({"granted": true, "present": true}))
}

#[cfg(target_os = "linux")]
const MAX_SCAN_BYTES: u64 = 256 * 1024 * 1024;

#[cfg(target_os = "linux")]
async fn scan_request(
    native: Arc<NativeServices>,
    request: &DesktopPortalRequest,
) -> Result<Value, NativeError> {
    let target = request
        .target
        .as_ref()
        .ok_or_else(|| NativeError::invalid("Scanning requires a destination target."))?;
    let target = target.clone();
    let device = validate_scanner_device(request.device.as_deref())?;
    let resolution = request.resolution.unwrap_or(300);
    if !(75..=1_200).contains(&resolution) {
        return Err(NativeError::invalid(
            "Scanner resolution must be between 75 and 1,200 DPI.",
        ));
    }
    let format = request.format.unwrap_or_default();
    tokio::task::spawn_blocking(move || scan_image(&native, &target, device, resolution, format))
        .await
        .map_err(|error| NativeError::native("Scanner task failed", error))?
}

#[cfg(target_os = "linux")]
fn scanner_tool() -> Result<&'static str, NativeError> {
    ["/usr/bin/scanimage", "/usr/local/bin/scanimage"]
        .into_iter()
        .find(|path| fs::metadata(path).is_ok_and(|metadata| metadata.is_file()))
        .ok_or_else(|| NativeError::native("SANE scanimage is unavailable", "install sane-utils"))
}

#[cfg(target_os = "linux")]
fn list_scanners() -> Result<Value, NativeError> {
    let output = Command::new(scanner_tool()?)
        .args(["--formatted-device-list", "%d\t%v\t%m\t%t\\n"])
        .output()
        .map_err(|error| NativeError::native("Cannot start scanner discovery", error))?;
    if !output.status.success() {
        return Err(NativeError::native(
            "Scanner discovery failed",
            bounded_stderr(&output.stderr),
        ));
    }
    if output.stdout.len() > 64 * 1024 {
        return Err(NativeError::invalid(
            "Scanner discovery output exceeded 64 KiB.",
        ));
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|_| NativeError::invalid("Scanner discovery returned invalid UTF-8."))?;
    let scanners = text
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(4, '\t');
            Some(json!({
                "device": fields.next()?.trim(),
                "vendor": fields.next()?.trim(),
                "model": fields.next()?.trim(),
                "kind": fields.next()?.trim(),
            }))
        })
        .collect::<Vec<_>>();
    Ok(json!({"scanners": scanners}))
}

#[cfg(target_os = "linux")]
fn validate_scanner_device(device: Option<&str>) -> Result<Option<String>, NativeError> {
    device
        .map(|device| {
            if device.is_empty()
                || device.len() > 256
                || device.chars().any(char::is_control)
                || device.starts_with('-')
            {
                return Err(NativeError::invalid(
                    "Scanner device identifier is invalid.",
                ));
            }
            Ok(device.to_owned())
        })
        .transpose()
}

#[cfg(target_os = "linux")]
struct ScanStaging(std::path::PathBuf);

#[cfg(target_os = "linux")]
impl Drop for ScanStaging {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(target_os = "linux")]
fn scan_image(
    native: &NativeServices,
    target: &FileTarget,
    device: Option<String>,
    resolution: u16,
    format: ScannerImageFormat,
) -> Result<Value, NativeError> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random)
        .map_err(|error| NativeError::native("Cannot create scanner staging identity", error))?;
    let mut identity = String::with_capacity(32);
    for byte in random {
        use std::fmt::Write as _;
        write!(&mut identity, "{byte:02x}")
            .map_err(|error| NativeError::native("Cannot encode scanner identity", error))?;
    }
    let staging = ScanStaging(std::env::temp_dir().join(format!(
        "pam-desktop-scan-{}-{identity}.part",
        std::process::id()
    )));
    let staging_file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&staging.0)
        .map_err(|error| NativeError::native("Cannot create scanner staging file", error))?;
    let mut command = Command::new(scanner_tool()?);
    if let Some(device) = device {
        command.args(["--device-name", &device]);
    }
    command
        .arg(format!("--resolution={resolution}"))
        .arg(format!("--format={}", scanner_format(format)))
        .stdout(Stdio::from(staging_file))
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|error| NativeError::native("Cannot start scanner", error))?;
    let status = loop {
        if fs::metadata(&staging.0).is_ok_and(|metadata| metadata.len() > MAX_SCAN_BYTES) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(NativeError::invalid(
                "Scanned images are limited to 256 MiB.",
            ));
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| NativeError::native("Cannot monitor scanner", error))?
        {
            break status;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if !status.success() {
        return Err(NativeError::native("Scanning failed", status));
    }
    let mut destination = native.open_write_stream(target)?;
    let mut source = fs::File::open(&staging.0)
        .map_err(|error| NativeError::native("Cannot reopen scanned image", error))?;
    let bytes = std::io::copy(&mut source, &mut destination)
        .map_err(|error| NativeError::native("Cannot publish scanned image", error))?;
    destination
        .sync_all()
        .map_err(|error| NativeError::native("Cannot persist scanned image", error))?;
    Ok(json!({"bytesWritten": bytes, "format": format as u8}))
}

#[cfg(target_os = "linux")]
fn scanner_format(format: ScannerImageFormat) -> &'static str {
    match format {
        ScannerImageFormat::Png => "png",
        ScannerImageFormat::Jpeg => "jpeg",
        ScannerImageFormat::Pnm => "pnm",
    }
}

#[cfg(target_os = "linux")]
fn bounded_stderr(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(4 * 1024)]).into_owned()
}

#[cfg(not(target_os = "linux"))]
#[allow(
    clippy::unused_async,
    reason = "the platform fallback preserves the asynchronous gateway contract"
)]
pub async fn execute(
    _native: std::sync::Arc<NativeServices>,
    _request: &DesktopPortalRequest,
) -> Result<Value, NativeError> {
    Err(NativeError::native(
        "The XDG desktop portal is unavailable on this platform",
        std::env::consts::OS,
    ))
}

fn default_title() -> String {
    "Pam Desktop".to_owned()
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn validates_scanner_identifiers_and_integer_formats() {
        assert_eq!(
            validate_scanner_device(Some("airscan:e0:Office Scanner"))
                .expect("scanner identifier should be valid")
                .as_deref(),
            Some("airscan:e0:Office Scanner")
        );
        assert!(validate_scanner_device(Some("--batch")).is_err());
        assert!(validate_scanner_device(Some("device\nother")).is_err());
        assert_eq!(scanner_format(ScannerImageFormat::Png), "png");
        assert_eq!(scanner_format(ScannerImageFormat::Jpeg), "jpeg");
        assert_eq!(scanner_format(ScannerImageFormat::Pnm), "pnm");
    }
}
