use std::os::fd::AsFd;

use ashpd::desktop::open_uri::OpenFileRequest;
use ashpd::desktop::print::{PageSetup, PrintProxy, Settings};
use ashpd::desktop::screenshot::Screenshot;
use pam_desktop_protocol::{DesktopPortalOperation, FileAccess};
use serde::Deserialize;
use serde_json::{Value, json};
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
}

pub async fn execute(
    native: &NativeServices,
    request: &DesktopPortalRequest,
) -> Result<Value, NativeError> {
    if !native.desktop_portal_enabled() {
        return Err(NativeError::disabled("desktop portal"));
    }
    match request.operation {
        DesktopPortalOperation::OpenUri => {
            let url = Url::parse(
                request
                    .url
                    .as_deref()
                    .ok_or_else(|| NativeError::invalid("Opening a URI requires url."))?,
            )
            .map_err(|error| {
                NativeError::invalid(format!("The desktop URI is invalid: {error}"))
            })?;
            if !matches!(url.scheme(), "https" | "mailto" | "tel")
                || url.username() != ""
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
                .map_err(|error| {
                    NativeError::native("Cannot open the URI through XDG portal", error)
                })?
                .response()
                .map_err(|error| NativeError::native("The XDG open request failed", error))?;
            Ok(Value::Null)
        }
        DesktopPortalOperation::Screenshot => {
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
    }
}

fn default_title() -> String {
    "Pam Desktop".to_owned()
}
