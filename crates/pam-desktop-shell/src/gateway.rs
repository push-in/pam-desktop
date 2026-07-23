use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use axum::body::Body;
use axum::extract::{Path as AxumPath, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, ORIGIN, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, Response, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use getrandom::fill;
use pam_desktop_protocol::ResponseStatus;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::oneshot;
use winit::event_loop::EventLoopProxy;

use crate::host_event::HostEvent;
use crate::project::Project;
use crate::worker::WorkerClient;

const BRIDGE_HEADER: &str = "x-pam-bridge";

pub struct Gateway {
    url: String,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Gateway {
    pub fn start(
        project: &Project,
        entry: &Path,
        worker: WorkerClient,
        event_proxy: EventLoopProxy<HostEvent>,
    ) -> Result<Self, String> {
        let public_root = entry
            .parent()
            .ok_or_else(|| "desktop entry has no parent directory".to_owned())?
            .to_path_buf();
        let index_name = entry
            .file_name()
            .ok_or_else(|| "desktop entry has no filename".to_owned())?
            .to_os_string();
        if !public_root.starts_with(project.root()) {
            return Err("desktop public directory escapes the project".to_owned());
        }

        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
            .map_err(|error| format!("cannot bind desktop gateway: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("cannot configure desktop gateway: {error}"))?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("cannot read desktop gateway address: {error}"))?;
        let url = format!("http://127.0.0.1:{}/", address.port());
        let token = secure_token()?;

        let state = GatewayState {
            public_root,
            index_name,
            origin: url.trim_end_matches('/').to_owned(),
            token,
            worker: Arc::new(Mutex::new(worker)),
            event_proxy,
        };
        let router = Router::new()
            .route("/", get(serve_index))
            .route("/_pam/bridge.js", get(serve_bridge))
            .route("/_pam/invoke", post(invoke))
            .route("/{*path}", get(serve_asset))
            .with_state(state);
        let (shutdown, receiver) = oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("pam-desktop-gateway".to_owned())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build();
                match runtime {
                    Ok(runtime) => runtime.block_on(async move {
                        let listener = match tokio::net::TcpListener::from_std(listener) {
                            Ok(listener) => listener,
                            Err(error) => {
                                eprintln!("pam-desktop: cannot start gateway listener: {error}");
                                return;
                            }
                        };
                        let server = axum::serve(listener, router).with_graceful_shutdown(async {
                            let _ = receiver.await;
                        });
                        if let Err(error) = server.await {
                            eprintln!("pam-desktop: gateway stopped unexpectedly: {error}");
                        }
                    }),
                    Err(error) => {
                        eprintln!("pam-desktop: cannot start gateway runtime: {error}");
                    }
                }
            })
            .map_err(|error| format!("cannot start desktop gateway thread: {error}"))?;

        Ok(Self {
            url,
            shutdown: Some(shutdown),
            thread: Some(thread),
        })
    }

    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Clone)]
struct GatewayState {
    public_root: PathBuf,
    index_name: std::ffi::OsString,
    origin: String,
    token: String,
    worker: Arc<Mutex<WorkerClient>>,
    event_proxy: EventLoopProxy<HostEvent>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InvokeRequest {
    command: String,
    #[serde(default)]
    payload: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InvokeResponse {
    ok: bool,
    data: Value,
    error: Option<ClientError>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientError {
    code: u16,
    message: String,
}

async fn serve_index(State(state): State<GatewayState>) -> Response<Body> {
    serve_file(&state, Path::new(&state.index_name)).await
}

async fn serve_asset(
    State(state): State<GatewayState>,
    AxumPath(path): AxumPath<String>,
) -> Response<Body> {
    serve_file(&state, Path::new(&path)).await
}

async fn serve_file(state: &GatewayState, relative: &Path) -> Response<Body> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return plain_response(StatusCode::NOT_FOUND, "Not found");
    }

    let candidate = state.public_root.join(relative);
    let resolved = match tokio::fs::canonicalize(&candidate).await {
        Ok(path) if path.starts_with(&state.public_root) && path.is_file() => path,
        _ => return plain_response(StatusCode::NOT_FOUND, "Not found"),
    };
    let Ok(contents) = tokio::fs::read(&resolved).await else {
        return plain_response(StatusCode::NOT_FOUND, "Not found");
    };
    let content_type = content_type(&resolved);
    secure_response(StatusCode::OK, content_type, Body::from(contents))
}

async fn serve_bridge(State(state): State<GatewayState>) -> Response<Body> {
    let script = BRIDGE_SCRIPT
        .replace("__PAM_ORIGIN__", &javascript_string(&state.origin))
        .replace("__PAM_TOKEN__", &javascript_string(&state.token));
    secure_response(
        StatusCode::OK,
        "text/javascript; charset=utf-8",
        Body::from(script),
    )
}

async fn invoke(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<InvokeRequest>,
) -> Response<Body> {
    if !authorized(&state, &headers) {
        return json_response(
            StatusCode::FORBIDDEN,
            &InvokeResponse {
                ok: false,
                data: Value::Null,
                error: Some(ClientError {
                    code: 7,
                    message: "The desktop bridge rejected this origin or token.".to_owned(),
                }),
            },
        );
    }
    if !valid_command(&request.command) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &InvokeResponse {
                ok: false,
                data: Value::Null,
                error: Some(ClientError {
                    code: 1,
                    message: "The command name is invalid.".to_owned(),
                }),
            },
        );
    }

    let worker = state.worker.clone();
    let response = tokio::task::spawn_blocking(move || {
        let mut worker = worker
            .lock()
            .map_err(|_| "PHP worker lock is poisoned".to_owned())?;
        worker.request(request.command, request.payload)
    })
    .await;
    let response = match response {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            return gateway_error(error);
        }
        Err(error) => {
            return gateway_error(format!("PHP worker task failed: {error}"));
        }
    };

    if !response.effects.is_empty() {
        let _ = state
            .event_proxy
            .send_event(HostEvent::ApplyEffects(response.effects.clone()));
    }

    if response.status == ResponseStatus::Failure {
        let error = match response.error {
            Some(error) => ClientError {
                code: error.code as u16,
                message: error.message,
            },
            None => ClientError {
                code: 8,
                message: "PHP worker returned an unspecified failure".to_owned(),
            },
        };
        return json_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            &InvokeResponse {
                ok: false,
                data: Value::Null,
                error: Some(error),
            },
        );
    }

    json_response(
        StatusCode::OK,
        &InvokeResponse {
            ok: true,
            data: response.payload,
            error: None,
        },
    )
}

fn authorized(state: &GatewayState, headers: &HeaderMap) -> bool {
    let origin_matches = headers
        .get(ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == state.origin);
    let token_matches = headers
        .get(BRIDGE_HEADER)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| constant_time_eq(value.as_bytes(), state.token.as_bytes()));

    origin_matches && token_matches
}

fn valid_command(command: &str) -> bool {
    let mut characters = command.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && command.len() <= 64
        && characters
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
}

fn secure_token() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    fill(&mut bytes).map_err(|error| format!("cannot create desktop bridge token: {error}"))?;
    let mut token = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut token, "{byte:02x}")
            .map_err(|error| format!("cannot encode desktop bridge token: {error}"))?;
    }
    Ok(token)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn javascript_string(value: &str) -> String {
    serde_json::to_string(value).expect("strings are always serializable")
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(std::ffi::OsStr::to_str) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn plain_response(status: StatusCode, body: &'static str) -> Response<Body> {
    secure_response(status, "text/plain; charset=utf-8", Body::from(body))
}

fn gateway_error(message: String) -> Response<Body> {
    json_response(
        StatusCode::BAD_GATEWAY,
        &InvokeResponse {
            ok: false,
            data: Value::Null,
            error: Some(ClientError { code: 6, message }),
        },
    )
}

fn json_response(status: StatusCode, value: &impl Serialize) -> Response<Body> {
    match serde_json::to_vec(value) {
        Ok(body) => secure_response(status, "application/json; charset=utf-8", Body::from(body)),
        Err(_) => plain_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal error"),
    }
}

fn secure_response(status: StatusCode, content_type: &'static str, body: Body) -> Response<Body> {
    let mut response = Response::new(body);
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; base-uri 'none'; connect-src 'self'; font-src 'self'; frame-src 'none'; img-src 'self' data:; object-src 'none'; script-src 'self'; style-src 'self'",
        ),
    );
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    );
    response
}

const BRIDGE_SCRIPT: &str = r#"
(() => {
    "use strict";

    const allowedOrigin = __PAM_ORIGIN__;
    const token = __PAM_TOKEN__;
    if (window.location.origin !== allowedOrigin) {
        return;
    }

    const invoke = async (command, payload = null) => {
        if (typeof command !== "string" || command.length === 0) {
            throw new TypeError("Pam command must be a non-empty string.");
        }

        const response = await fetch("/_pam/invoke", {
            method: "POST",
            headers: {
                "Content-Type": "application/json",
                "X-Pam-Bridge": token,
            },
            body: JSON.stringify({ command, payload }),
        });
        const envelope = await response.json();
        if (!envelope.ok) {
            const error = new Error(envelope.error?.message ?? "Pam command failed.");
            error.code = envelope.error?.code ?? 8;
            throw error;
        }
        return envelope.data;
    };

    Object.defineProperty(window, "pam", {
        configurable: false,
        enumerable: true,
        writable: false,
        value: Object.freeze({ invoke }),
    });
})();
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_command_names() {
        assert!(valid_command("greet"));
        assert!(valid_command("window.set-title"));
        assert!(!valid_command("@pam/boot"));
        assert!(!valid_command("../escape"));
        assert!(!valid_command(""));
    }

    #[test]
    fn compares_bridge_tokens_without_early_content_exit() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"diff"));
        assert!(!constant_time_eq(b"short", b"longer"));
    }
}
