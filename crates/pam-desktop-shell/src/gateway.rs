use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path as AxumPath, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, ORIGIN, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, Response, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use getrandom::fill;
use pam_desktop_protocol::{
    Bootstrap, ClientEvent, DialogKind, EVENT_COMMAND, ErrorCode, FileAccess, FileEntryKind,
    MAIN_WINDOW_ID, MAX_COMMAND_TIMEOUT_MS, MIN_COMMAND_TIMEOUT_MS, ResponseEnvelope,
    ResponseStatus, validate_identifier,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::oneshot;
use winit::event_loop::EventLoopProxy;

use crate::event_hub::{EventHub, PublishedEvent};
use crate::host_event::HostEvent;
use crate::native::{
    ClipboardRequest, DialogBridgeRequest, DialogRequest, FileRequest, NativeError, NativeServices,
    NotificationRequest,
};
use crate::plugin::{PluginError, PluginSupervisor};
use crate::project::Project;
use crate::scheduler::BackgroundScheduler;
use crate::updater::{UpdateError, UpdateSnapshot, Updater};
use crate::watcher::{ChangeKind, ProjectWatcher};
use crate::worker::{CancellationToken, WorkerRequestError, WorkerSupervisor};

const BRIDGE_HEADER: &str = "x-pam-bridge";
const EVENT_POLL_TIMEOUT: Duration = Duration::from_secs(20);

pub struct Gateway {
    url: String,
    state: GatewayState,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
    watcher: Option<ProjectWatcher>,
}

impl Gateway {
    pub fn start(
        project: &Project,
        supervisor: WorkerSupervisor,
        bootstrap: Bootstrap,
        event_proxy: EventLoopProxy<HostEvent>,
        watch: bool,
    ) -> Result<Self, String> {
        project.validate_bootstrap(&bootstrap)?;
        let public_root = project.public_root()?;
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
        let native = NativeServices::prepare(project.root(), &bootstrap.capabilities)?;
        let updater = Updater::prepare(&bootstrap.manifest);
        let plugins = PluginSupervisor::prepare(project, &bootstrap.rust_plugins)?;
        let background_jobs = bootstrap.background_jobs.clone();

        let state = GatewayState {
            project: project.clone(),
            public_root,
            origin: url.trim_end_matches('/').to_owned(),
            token,
            bootstrap: Arc::new(RwLock::new(bootstrap)),
            supervisor: Arc::new(Mutex::new(supervisor)),
            cancellations: Arc::new(Mutex::new(HashMap::new())),
            events: EventHub::default(),
            event_proxy,
            native: Arc::new(RwLock::new(Arc::new(native))),
            updater: Arc::new(RwLock::new(updater)),
            plugins: Arc::new(RwLock::new(Arc::new(plugins))),
            scheduler: Arc::new(Mutex::new(None)),
        };
        state.replace_scheduler(&background_jobs)?;
        let router = Router::new()
            .route("/", get(serve_main))
            .route("/_pam/window/{window}", get(serve_window))
            .route("/_pam/bridge.js", get(serve_bridge))
            .route("/_pam/invoke", post(invoke))
            .route("/_pam/emit", post(emit))
            .route("/_pam/cancel", post(cancel))
            .route("/_pam/events", post(events))
            .route("/_pam/fs", post(filesystem))
            .route("/_pam/dialog", post(dialog))
            .route("/_pam/clipboard", post(clipboard))
            .route("/_pam/notification", post(notification))
            .route("/_pam/update/status", post(update_status))
            .route("/_pam/update/check", post(update_check))
            .route("/_pam/update/download", post(update_download))
            .route("/_pam/update/install", post(update_install))
            .route("/_pam/plugin/invoke", post(plugin_invoke))
            .route("/{*path}", get(serve_asset))
            .with_state(state.clone());
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

        let watcher = if watch {
            let watcher_state = state.clone();
            Some(ProjectWatcher::start(
                project.root().to_path_buf(),
                move |kind| watcher_state.reload(kind),
            )?)
        } else {
            None
        };
        start_update_policy(&state);

        Ok(Self {
            url,
            state,
            shutdown: Some(shutdown),
            thread: Some(thread),
            watcher,
        })
    }

    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn window_url(&self, window_id: &str) -> Result<String, String> {
        validate_identifier(window_id, "window")?;
        Ok(format!("{}_pam/window/{window_id}", self.url))
    }

    pub fn drag_hover(&self, window_id: &str, path: &Path) {
        if !self.state.has_window(window_id) {
            return;
        }
        let native = self.state.native_services();
        if !native.drag_and_drop_enabled() {
            return;
        }
        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            return;
        };
        if metadata.file_type().is_symlink() {
            return;
        }
        let kind = if metadata.is_file() {
            FileEntryKind::File
        } else if metadata.is_dir() {
            FileEntryKind::Directory
        } else {
            return;
        };
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        self.state.events.publish(ClientEvent {
            name: "pam.drag.enter".to_owned(),
            payload: serde_json::json!({"name": name, "kind": kind}),
            window_id: Some(window_id.to_owned()),
        });
    }

    pub fn drag_drop(&self, window_id: &str, path: &Path) {
        if !self.state.has_window(window_id) {
            return;
        }
        let native = self.state.native_services();
        if !native.drag_and_drop_enabled() {
            return;
        }
        match native.grant_path(path, FileAccess::Read) {
            Ok(file) => self.state.events.publish(ClientEvent {
                name: "pam.drag.drop".to_owned(),
                payload: serde_json::json!({"files": [file]}),
                window_id: Some(window_id.to_owned()),
            }),
            Err(error) => self.state.events.publish(ClientEvent {
                name: "pam.drag.error".to_owned(),
                payload: serde_json::json!({
                    "code": error.code as u16,
                    "message": error.message,
                }),
                window_id: Some(window_id.to_owned()),
            }),
        }
    }

    pub fn drag_leave(&self, window_id: &str) {
        if !self.state.has_window(window_id)
            || !self.state.native_services().drag_and_drop_enabled()
        {
            return;
        }
        self.state.events.publish(ClientEvent {
            name: "pam.drag.leave".to_owned(),
            payload: Value::Null,
            window_id: Some(window_id.to_owned()),
        });
    }

    pub fn dispatch_native_event(&self, name: &'static str, payload: Value) {
        self.state.events.publish(ClientEvent {
            name: name.to_owned(),
            payload: payload.clone(),
            window_id: None,
        });
        self.state.dispatch_php_event(name, payload);
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.watcher.take();
        self.state
            .scheduler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
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
    project: Project,
    public_root: PathBuf,
    origin: String,
    token: String,
    bootstrap: Arc<RwLock<Bootstrap>>,
    supervisor: Arc<Mutex<WorkerSupervisor>>,
    cancellations: Arc<Mutex<HashMap<RequestKey, CancellationToken>>>,
    events: EventHub,
    event_proxy: EventLoopProxy<HostEvent>,
    native: Arc<RwLock<Arc<NativeServices>>>,
    updater: Arc<RwLock<Updater>>,
    plugins: Arc<RwLock<Arc<PluginSupervisor>>>,
    scheduler: Arc<Mutex<Option<BackgroundScheduler>>>,
}

impl GatewayState {
    fn has_window(&self, window_id: &str) -> bool {
        self.bootstrap
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .windows
            .iter()
            .any(|window| window.id == window_id)
    }

    fn default_timeout_ms(&self) -> u64 {
        self.bootstrap
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .command_timeout_ms
    }

    fn window_entry(&self, window_id: &str) -> Option<String> {
        self.bootstrap
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .windows
            .iter()
            .find(|window| window.id == window_id)
            .map(|window| window.entry.clone())
    }

    fn process_response(&self, response: &ResponseEnvelope) {
        if !response.effects.is_empty() {
            let _ = self
                .event_proxy
                .send_event(HostEvent::ApplyEffects(response.effects.clone()));
        }
        for event in &response.events {
            self.events.publish(event.clone());
        }
    }

    fn native_services(&self) -> Arc<NativeServices> {
        self.native
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn updater(&self) -> Updater {
        self.updater
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn plugins(&self) -> Arc<PluginSupervisor> {
        self.plugins
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn replace_scheduler(
        &self,
        jobs: &[pam_desktop_protocol::BackgroundJobConfig],
    ) -> Result<(), String> {
        let scheduler =
            BackgroundScheduler::start(jobs, &self.supervisor, &self.events, &self.event_proxy)?;
        *self
            .scheduler
            .lock()
            .map_err(|_| "background scheduler lock is poisoned".to_owned())? = Some(scheduler);
        Ok(())
    }

    fn dispatch_php_event(&self, name: &'static str, payload: Value) {
        let state = self.clone();
        let _ = std::thread::Builder::new()
            .name(format!("pam-native-event-{}", name.replace('.', "-")))
            .spawn(move || {
                let timeout = Duration::from_millis(state.default_timeout_ms());
                let result = state
                    .supervisor
                    .lock()
                    .map_err(|_| WorkerRequestError::Crashed("worker lock is poisoned".to_owned()))
                    .and_then(|mut supervisor| {
                        supervisor.request(
                            EVENT_COMMAND,
                            MAIN_WINDOW_ID,
                            serde_json::json!({"name": name, "payload": payload}),
                            timeout,
                            &CancellationToken::default(),
                        )
                    });
                match result {
                    Ok(response) if response.status == ResponseStatus::Success => {
                        state.process_response(&response);
                    }
                    Ok(response)
                        if response
                            .error
                            .as_ref()
                            .is_some_and(|error| error.code == ErrorCode::UnknownEvent) => {}
                    Ok(response) => {
                        let message = response.error.map_or_else(
                            || "PHP worker rejected a native event.".to_owned(),
                            |error| error.message,
                        );
                        state.events.publish(ClientEvent {
                            name: "pam.shell.error".to_owned(),
                            payload: serde_json::json!({
                                "code": ErrorCode::HandlerFailed as u16,
                                "message": message,
                            }),
                            window_id: None,
                        });
                    }
                    Err(error) => {
                        state.events.publish(ClientEvent {
                            name: "pam.shell.error".to_owned(),
                            payload: serde_json::json!({
                                "code": ErrorCode::WorkerUnavailable as u16,
                                "message": error.to_string(),
                            }),
                            window_id: None,
                        });
                    }
                }
            });
    }

    fn reload(&self, kind: ChangeKind) {
        match kind {
            ChangeKind::Assets => {
                let _ = self.event_proxy.send_event(HostEvent::ReloadViews);
                self.events.publish(ClientEvent {
                    name: "pam.dev.reloaded".to_owned(),
                    payload: serde_json::json!({"kind": 1}),
                    window_id: None,
                });
            }
            ChangeKind::Runtime => {
                let result = self
                    .supervisor
                    .lock()
                    .map_err(|_| "PHP worker supervisor lock is poisoned".to_owned())
                    .and_then(|mut supervisor| supervisor.restart())
                    .and_then(|bootstrap| {
                        self.project.validate_bootstrap(&bootstrap)?;
                        let native =
                            NativeServices::prepare(self.project.root(), &bootstrap.capabilities)?;
                        let plugins =
                            PluginSupervisor::prepare(&self.project, &bootstrap.rust_plugins)?;
                        Ok((bootstrap, native, plugins))
                    });
                match result {
                    Ok((bootstrap, native, plugins)) => {
                        *self
                            .native
                            .write()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(native);
                        *self
                            .bootstrap
                            .write()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) = bootstrap.clone();
                        *self
                            .updater
                            .write()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) =
                            Updater::prepare(&bootstrap.manifest);
                        *self
                            .plugins
                            .write()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(plugins);
                        if let Err(error) = self.replace_scheduler(&bootstrap.background_jobs) {
                            eprintln!("pam-desktop: cannot reload background scheduler: {error}");
                        }
                        let _ = self
                            .event_proxy
                            .send_event(HostEvent::Reconfigure(Box::new(bootstrap)));
                        start_update_policy(self);
                        self.events.publish(ClientEvent {
                            name: "pam.dev.reloaded".to_owned(),
                            payload: serde_json::json!({"kind": 2}),
                            window_id: None,
                        });
                    }
                    Err(error) => {
                        eprintln!("pam-desktop: hot reload failed: {error}");
                        self.events.publish(ClientEvent {
                            name: "pam.dev.error".to_owned(),
                            payload: serde_json::json!({"message": error}),
                            window_id: None,
                        });
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RequestKey {
    window_id: String,
    request_id: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InvokeRequest {
    request_id: u64,
    command: String,
    window_id: String,
    timeout_ms: Option<u64>,
    #[serde(default)]
    payload: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EmitRequest {
    request_id: u64,
    name: String,
    window_id: String,
    timeout_ms: Option<u64>,
    #[serde(default)]
    payload: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelRequest {
    request_id: u64,
    window_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventPollRequest {
    after: u64,
    window_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateBridgeRequest {
    window_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginBridgeRequest {
    request_id: u64,
    window_id: String,
    plugin: String,
    command: String,
    timeout_ms: Option<u64>,
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
struct CancelResponse {
    cancelled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EventPollResponse {
    cursor: u64,
    events: Vec<PublishedEvent>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientError {
    code: u16,
    message: String,
}

async fn serve_main(State(state): State<GatewayState>) -> Response<Body> {
    serve_window_entry(&state, MAIN_WINDOW_ID).await
}

async fn serve_window(
    State(state): State<GatewayState>,
    AxumPath(window): AxumPath<String>,
) -> Response<Body> {
    serve_window_entry(&state, &window).await
}

async fn serve_window_entry(state: &GatewayState, window_id: &str) -> Response<Body> {
    let Some(entry) = state.window_entry(window_id) else {
        return plain_response(StatusCode::NOT_FOUND, "Not found");
    };
    let Ok(resolved) = state.project.resolve_entry(&entry) else {
        return plain_response(StatusCode::NOT_FOUND, "Not found");
    };
    let Ok(relative) = resolved.strip_prefix(&state.public_root) else {
        return plain_response(StatusCode::NOT_FOUND, "Not found");
    };
    serve_file(state, relative).await
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
    secure_response(
        StatusCode::OK,
        content_type(&resolved),
        Body::from(contents),
    )
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
        return unauthorized_response();
    }
    if validate_identifier(&request.command, "command").is_err() {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidMessage,
            "The command name is invalid.",
        );
    }
    execute_request(
        state,
        request.request_id,
        request.window_id,
        request.timeout_ms,
        request.command,
        request.payload,
    )
    .await
}

async fn emit(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<EmitRequest>,
) -> Response<Body> {
    if !authorized(&state, &headers) {
        return unauthorized_response();
    }
    if validate_identifier(&request.name, "event").is_err() {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidMessage,
            "The event name is invalid.",
        );
    }
    execute_request(
        state,
        request.request_id,
        request.window_id,
        request.timeout_ms,
        EVENT_COMMAND.to_owned(),
        serde_json::json!({
            "name": request.name,
            "payload": request.payload,
        }),
    )
    .await
}

async fn execute_request(
    state: GatewayState,
    request_id: u64,
    window_id: String,
    timeout_ms: Option<u64>,
    command: String,
    payload: Value,
) -> Response<Body> {
    if request_id == 0 || !state.has_window(&window_id) {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidMessage,
            "The bridge request identity is invalid.",
        );
    }
    let timeout_ms = timeout_ms.unwrap_or_else(|| state.default_timeout_ms());
    if !(MIN_COMMAND_TIMEOUT_MS..=MAX_COMMAND_TIMEOUT_MS).contains(&timeout_ms) {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidPayload,
            "The command timeout is outside the allowed range.",
        );
    }

    let key = RequestKey {
        window_id: window_id.clone(),
        request_id,
    };
    let cancellation = CancellationToken::default();
    {
        let mut cancellations = state
            .cancellations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if cancellations
            .insert(key.clone(), cancellation.clone())
            .is_some()
        {
            return client_failure(
                StatusCode::CONFLICT,
                ErrorCode::InvalidMessage,
                "The bridge request identifier is already active.",
            );
        }
    }

    let supervisor = state.supervisor.clone();
    let response = tokio::task::spawn_blocking(move || {
        let mut supervisor = supervisor
            .lock()
            .map_err(|_| WorkerRequestError::Crashed("worker lock is poisoned".to_owned()))?;
        supervisor.request(
            command,
            window_id,
            payload,
            Duration::from_millis(timeout_ms),
            &cancellation,
        )
    })
    .await;
    state
        .cancellations
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&key);

    let response = match response {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => return worker_error_response(error),
        Err(error) => {
            return client_failure(
                StatusCode::BAD_GATEWAY,
                ErrorCode::WorkerCrashed,
                &format!("The PHP worker task failed: {error}"),
            );
        }
    };
    state.process_response(&response);

    if response.status == ResponseStatus::Failure {
        let error = response.error.map_or(
            ClientError {
                code: ErrorCode::Internal as u16,
                message: "PHP worker returned an unspecified failure".to_owned(),
            },
            |error| ClientError {
                code: error.code as u16,
                message: error.message,
            },
        );
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

async fn cancel(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<CancelRequest>,
) -> Response<Body> {
    if !authorized(&state, &headers) {
        return unauthorized_response();
    }
    let key = RequestKey {
        window_id: request.window_id,
        request_id: request.request_id,
    };
    let cancellation = state
        .cancellations
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
        .cloned();
    if let Some(cancellation) = &cancellation {
        cancellation.cancel();
    }
    json_response(
        StatusCode::OK,
        &CancelResponse {
            cancelled: cancellation.is_some(),
        },
    )
}

async fn events(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<EventPollRequest>,
) -> Response<Body> {
    if !authorized(&state, &headers) {
        return unauthorized_response();
    }
    if !state.has_window(&request.window_id) {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidMessage,
            "The event window is invalid.",
        );
    }
    let events = state
        .events
        .poll(request.after, &request.window_id, EVENT_POLL_TIMEOUT)
        .await;
    let cursor = events
        .last()
        .map_or_else(|| state.events.latest_id(), |event| event.id);
    json_response(StatusCode::OK, &EventPollResponse { cursor, events })
}

async fn filesystem(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<FileRequest>,
) -> Response<Body> {
    if !authorized(&state, &headers) {
        return unauthorized_response();
    }
    if !state.has_window(&request.window_id) {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidMessage,
            "The filesystem source window is invalid.",
        );
    }
    let native = state.native_services();
    native_task(tokio::task::spawn_blocking(move || native.filesystem(&request)).await)
}

async fn clipboard(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<ClipboardRequest>,
) -> Response<Body> {
    if !authorized(&state, &headers) {
        return unauthorized_response();
    }
    if !state.has_window(&request.window_id) {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidMessage,
            "The clipboard source window is invalid.",
        );
    }
    let native = state.native_services();
    native_task(tokio::task::spawn_blocking(move || native.clipboard(&request)).await)
}

async fn notification(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<NotificationRequest>,
) -> Response<Body> {
    if !authorized(&state, &headers) {
        return unauthorized_response();
    }
    if !state.has_window(&request.window_id) {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidMessage,
            "The notification source window is invalid.",
        );
    }
    let native = state.native_services();
    native_task(tokio::task::spawn_blocking(move || native.notify(&request)).await)
}

async fn update_status(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<UpdateBridgeRequest>,
) -> Response<Body> {
    if let Some(response) = authorize_update_request(&state, &headers, &request) {
        return response;
    }
    native_success(serde_json::to_value(state.updater().snapshot()).unwrap_or(Value::Null))
}

async fn update_check(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<UpdateBridgeRequest>,
) -> Response<Body> {
    update_operation(state, headers, request, Updater::check).await
}

async fn update_download(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<UpdateBridgeRequest>,
) -> Response<Body> {
    update_operation(state, headers, request, Updater::download).await
}

async fn update_install(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<UpdateBridgeRequest>,
) -> Response<Body> {
    if let Some(response) = authorize_update_request(&state, &headers, &request) {
        return response;
    }
    let updater = state.updater();
    match tokio::task::spawn_blocking(move || updater.install()).await {
        Ok(Ok(snapshot)) => {
            publish_update_snapshot(&state.events, "pam.update.applying", &snapshot);
            let proxy = state.event_proxy.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(150)).await;
                let _ = proxy.send_event(HostEvent::Exit);
            });
            update_success(snapshot)
        }
        Ok(Err(error)) => update_failure(&error),
        Err(error) => update_failure(&UpdateError {
            code: ErrorCode::UpdateInstallFailed,
            message: format!("The update install task failed: {error}"),
        }),
    }
}

async fn update_operation(
    state: GatewayState,
    headers: HeaderMap,
    request: UpdateBridgeRequest,
    operation: fn(&Updater) -> Result<UpdateSnapshot, UpdateError>,
) -> Response<Body> {
    if let Some(response) = authorize_update_request(&state, &headers, &request) {
        return response;
    }
    let updater = state.updater();
    match tokio::task::spawn_blocking(move || operation(&updater)).await {
        Ok(Ok(snapshot)) => {
            publish_update_snapshot(&state.events, "pam.update.changed", &snapshot);
            update_success(snapshot)
        }
        Ok(Err(error)) => update_failure(&error),
        Err(error) => update_failure(&UpdateError {
            code: ErrorCode::NativeOperationFailed,
            message: format!("The update task failed: {error}"),
        }),
    }
}

fn authorize_update_request(
    state: &GatewayState,
    headers: &HeaderMap,
    request: &UpdateBridgeRequest,
) -> Option<Response<Body>> {
    if !authorized(state, headers) {
        return Some(unauthorized_response());
    }
    if !state.has_window(&request.window_id) {
        return Some(client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidMessage,
            "The update source window is invalid.",
        ));
    }
    None
}

fn update_success(snapshot: UpdateSnapshot) -> Response<Body> {
    match serde_json::to_value(snapshot) {
        Ok(value) => native_success(value),
        Err(error) => client_failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            &format!("Cannot encode update state: {error}"),
        ),
    }
}

fn update_failure(error: &UpdateError) -> Response<Body> {
    let status = match error.code {
        ErrorCode::UpdateDisabled => StatusCode::FORBIDDEN,
        ErrorCode::UpdateUnavailable => StatusCode::NOT_FOUND,
        ErrorCode::UpdateIntegrityFailed => StatusCode::BAD_GATEWAY,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    client_failure(status, error.code, &error.message)
}

fn start_update_policy(state: &GatewayState) {
    let updater = state.updater();
    let Some(policy) = updater.policy() else {
        return;
    };
    if policy == pam_desktop_protocol::UpdatePolicy::Manual {
        return;
    }
    let events = state.events.clone();
    let event_proxy = state.event_proxy.clone();
    let _ = std::thread::Builder::new()
        .name("pam-desktop-updater".to_owned())
        .spawn(move || match updater.check() {
            Ok(snapshot) => {
                publish_update_snapshot(&events, "pam.update.changed", &snapshot);
                if policy == pam_desktop_protocol::UpdatePolicy::Automatic
                    && snapshot.state == pam_desktop_protocol::UpdateState::Available
                {
                    match updater.download() {
                        Ok(snapshot) => {
                            publish_update_snapshot(&events, "pam.update.ready", &snapshot);
                            match updater.install() {
                                Ok(snapshot) => {
                                    publish_update_snapshot(
                                        &events,
                                        "pam.update.applying",
                                        &snapshot,
                                    );
                                    std::thread::sleep(Duration::from_millis(150));
                                    let _ = event_proxy.send_event(HostEvent::Exit);
                                }
                                Err(error) => publish_update_error(&events, &error),
                            }
                        }
                        Err(error) => publish_update_error(&events, &error),
                    }
                }
            }
            Err(error) => publish_update_error(&events, &error),
        });
}

fn publish_update_snapshot(events: &EventHub, name: &str, snapshot: &UpdateSnapshot) {
    events.publish(ClientEvent {
        name: name.to_owned(),
        payload: serde_json::to_value(snapshot).unwrap_or(Value::Null),
        window_id: None,
    });
}

fn publish_update_error(events: &EventHub, error: &UpdateError) {
    events.publish(ClientEvent {
        name: "pam.update.error".to_owned(),
        payload: serde_json::json!({
            "code": error.code as u16,
            "message": error.message,
        }),
        window_id: None,
    });
}

async fn plugin_invoke(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<PluginBridgeRequest>,
) -> Response<Body> {
    if !authorized(&state, &headers) {
        return unauthorized_response();
    }
    if request.request_id == 0
        || !state.has_window(&request.window_id)
        || validate_identifier(&request.plugin, "plugin").is_err()
        || validate_identifier(&request.command, "plugin command").is_err()
    {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidMessage,
            "The Rust plugin invocation identity is invalid.",
        );
    }
    let timeout_ms = request
        .timeout_ms
        .unwrap_or_else(|| state.default_timeout_ms());
    if !(MIN_COMMAND_TIMEOUT_MS..=MAX_COMMAND_TIMEOUT_MS).contains(&timeout_ms) {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidPayload,
            "The Rust plugin timeout is outside the allowed range.",
        );
    }

    let key = RequestKey {
        window_id: request.window_id,
        request_id: request.request_id,
    };
    let cancellation = CancellationToken::default();
    {
        let mut cancellations = state
            .cancellations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if cancellations
            .insert(key.clone(), cancellation.clone())
            .is_some()
        {
            return client_failure(
                StatusCode::CONFLICT,
                ErrorCode::InvalidMessage,
                "The bridge request identifier is already active.",
            );
        }
    }

    let plugins = state.plugins();
    let result = tokio::task::spawn_blocking(move || {
        plugins.invoke(
            &request.plugin,
            &request.command,
            request.payload,
            Some(Duration::from_millis(timeout_ms)),
            &cancellation,
        )
    })
    .await;
    state
        .cancellations
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&key);

    match result {
        Ok(Ok(invocation)) => {
            for event in invocation.events {
                state.events.publish(event);
            }
            native_success(invocation.payload)
        }
        Ok(Err(error)) => plugin_failure(&error),
        Err(error) => plugin_failure(&PluginError {
            code: ErrorCode::PluginFailed,
            message: format!("The Rust plugin task failed: {error}"),
        }),
    }
}

fn plugin_failure(error: &PluginError) -> Response<Body> {
    let status = match error.code {
        ErrorCode::UnknownCommand | ErrorCode::InvalidMessage | ErrorCode::InvalidPayload => {
            StatusCode::BAD_REQUEST
        }
        ErrorCode::PluginUnavailable => StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::RequestTimedOut | ErrorCode::RequestCancelled => StatusCode::REQUEST_TIMEOUT,
        _ => StatusCode::UNPROCESSABLE_ENTITY,
    };
    client_failure(status, error.code, &error.message)
}

async fn dialog(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<DialogBridgeRequest>,
) -> Response<Body> {
    if !authorized(&state, &headers) {
        return unauthorized_response();
    }
    if !state.has_window(&request.window_id) {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidMessage,
            "The dialog source window is invalid.",
        );
    }
    let native = state.native_services();
    let access = match native.validate_dialog(&request) {
        Ok(access) => access,
        Err(error) => return native_failure(error),
    };
    let kind = request.kind;
    let (reply, receiver) = oneshot::channel();
    let event = HostEvent::Dialog(DialogRequest {
        kind,
        title: request.title,
        file_name: request.file_name,
        filters: request.filters,
        reply,
    });
    if state.event_proxy.send_event(event).is_err() {
        return native_failure(NativeError {
            code: ErrorCode::NativeOperationFailed,
            message: "The native event loop is unavailable for this dialog.".to_owned(),
        });
    }
    let paths = match receiver.await {
        Ok(Ok(paths)) => paths,
        Ok(Err(message)) => {
            return native_failure(NativeError {
                code: ErrorCode::NativeOperationFailed,
                message,
            });
        }
        Err(_) => {
            return native_failure(NativeError {
                code: ErrorCode::NativeOperationFailed,
                message: "The native dialog closed without a result.".to_owned(),
            });
        }
    };
    let files = match native.grant_paths(paths, access) {
        Ok(files) => files,
        Err(error) => return native_failure(error),
    };
    let data = match kind {
        DialogKind::OpenFiles => serde_json::to_value(files),
        DialogKind::OpenFile | DialogKind::SaveFile | DialogKind::OpenDirectory => files
            .into_iter()
            .next()
            .map_or(Ok(Value::Null), serde_json::to_value),
    };
    match data {
        Ok(data) => native_success(data),
        Err(error) => native_failure(NativeError {
            code: ErrorCode::Internal,
            message: format!("Cannot encode the dialog result: {error}"),
        }),
    }
}

fn native_task(
    result: Result<Result<Value, NativeError>, tokio::task::JoinError>,
) -> Response<Body> {
    match result {
        Ok(Ok(data)) => native_success(data),
        Ok(Err(error)) => native_failure(error),
        Err(error) => native_failure(NativeError {
            code: ErrorCode::NativeOperationFailed,
            message: format!("The native operation task failed: {error}"),
        }),
    }
}

fn native_success(data: Value) -> Response<Body> {
    json_response(
        StatusCode::OK,
        &InvokeResponse {
            ok: true,
            data,
            error: None,
        },
    )
}

fn native_failure(error: NativeError) -> Response<Body> {
    let NativeError { code, message } = error;
    let status = match code {
        ErrorCode::CapabilityDisabled | ErrorCode::PermissionDenied | ErrorCode::InvalidGrant => {
            StatusCode::FORBIDDEN
        }
        ErrorCode::ResourceNotFound => StatusCode::NOT_FOUND,
        ErrorCode::ResourceTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
        ErrorCode::InvalidMessage | ErrorCode::InvalidPayload => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    client_failure(status, code, &message)
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

fn worker_error_response(error: WorkerRequestError) -> Response<Body> {
    match error {
        WorkerRequestError::TimedOut => client_failure(
            StatusCode::REQUEST_TIMEOUT,
            ErrorCode::RequestTimedOut,
            "The PHP command exceeded its deadline and the worker was restarted.",
        ),
        WorkerRequestError::Cancelled => client_failure(
            StatusCode::REQUEST_TIMEOUT,
            ErrorCode::RequestCancelled,
            "The PHP command was cancelled and the worker was restarted.",
        ),
        WorkerRequestError::Crashed(message) => client_failure(
            StatusCode::BAD_GATEWAY,
            ErrorCode::WorkerCrashed,
            &format!("The PHP worker crashed and was recovered: {message}"),
        ),
    }
}

fn unauthorized_response() -> Response<Body> {
    client_failure(
        StatusCode::FORBIDDEN,
        ErrorCode::Unauthorized,
        "The desktop bridge rejected this origin or token.",
    )
}

fn client_failure(status: StatusCode, code: ErrorCode, message: &str) -> Response<Body> {
    json_response(
        status,
        &InvokeResponse {
            ok: false,
            data: Value::Null,
            error: Some(ClientError {
                code: code as u16,
                message: message.to_owned(),
            }),
        },
    )
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

    const match = window.location.pathname.match(/^\/_pam\/window\/([A-Za-z][A-Za-z0-9._-]{0,63})$/);
    const windowId = match?.[1] ?? "main";
    const listeners = new Map();
    let nextRequestId = 1;
    let cursor = 0;
    let stopped = false;

    const headers = {
        "Content-Type": "application/json",
        "X-Pam-Bridge": token,
    };

    const dispatch = (event) => {
        const registered = listeners.get(event.name);
        if (!registered) return;
        for (const listener of [...registered]) {
            try {
                listener(event.payload);
            } catch (error) {
                console.error(`Pam event listener failed for ${event.name}`, error);
            }
        }
    };

    const cancelRemote = (requestId) => {
        void fetch("/_pam/cancel", {
            method: "POST",
            headers,
            body: JSON.stringify({ requestId, windowId }),
        }).catch(() => {});
    };

    const request = async (endpoint, body, options = {}) => {
        const requestId = nextRequestId++;
        const controller = new AbortController();
        const externalSignal = options.signal;
        let timedOut = false;
        const abort = () => {
            controller.abort();
            cancelRemote(requestId);
        };
        if (externalSignal?.aborted) {
            const error = new Error("Pam request was cancelled.");
            error.code = 11;
            throw error;
        }
        externalSignal?.addEventListener("abort", abort, { once: true });
        const timer = options.timeout == null
            ? null
            : setTimeout(() => {
                timedOut = true;
                abort();
            }, options.timeout);

        try {
            const response = await fetch(endpoint, {
                method: "POST",
                headers,
                signal: controller.signal,
                body: JSON.stringify({
                    requestId,
                    windowId,
                    timeoutMs: options.timeout ?? null,
                    ...body,
                }),
            });
            const envelope = await response.json();
            if (!envelope.ok) {
                const error = new Error(envelope.error?.message ?? "Pam request failed.");
                error.code = envelope.error?.code ?? 8;
                throw error;
            }
            return envelope.data;
        } catch (error) {
            if (error?.name === "AbortError") {
                const aborted = new Error(
                    timedOut ? "Pam request timed out." : "Pam request was cancelled.",
                );
                aborted.code = timedOut ? 10 : 11;
                throw aborted;
            }
            throw error;
        } finally {
            if (timer !== null) clearTimeout(timer);
            externalSignal?.removeEventListener("abort", abort);
        }
    };

    const invoke = (command, payload = null, options = {}) => {
        if (typeof command !== "string" || command.length === 0) {
            throw new TypeError("Pam command must be a non-empty string.");
        }
        return request("/_pam/invoke", { command, payload }, options);
    };

    const emit = (name, payload = null, options = {}) => {
        if (typeof name !== "string" || name.length === 0) {
            throw new TypeError("Pam event must have a non-empty name.");
        }
        return request("/_pam/emit", { name, payload }, options);
    };

    const normalizeTarget = (target) => {
        if (target === null || typeof target !== "object" || Array.isArray(target)) {
            throw new TypeError("Pam filesystem targets must be objects.");
        }
        const root = typeof target.root === "string" ? target.root : null;
        const grantId = typeof target.grantId === "string" ? target.grantId : null;
        if ((root === null) === (grantId === null)) {
            throw new TypeError("Pam filesystem targets require exactly one root or grantId.");
        }
        const path = target.path ?? "";
        if (typeof path !== "string") {
            throw new TypeError("Pam filesystem target paths must be strings.");
        }
        return { root, grantId, path };
    };

    const nativeOptions = (options) => ({
        signal: options?.signal,
        timeout: options?.timeout,
    });

    const filesystem = Object.freeze({
        readText: async (target, options = {}) => {
            const data = await request("/_pam/fs", {
                operation: 1,
                target: normalizeTarget(target),
                content: null,
            }, nativeOptions(options));
            return data.text;
        },
        writeText: async (target, content, options = {}) => {
            if (typeof content !== "string") {
                throw new TypeError("Pam filesystem text writes require a string.");
            }
            return request("/_pam/fs", {
                operation: 2,
                target: normalizeTarget(target),
                content,
            }, nativeOptions(options));
        },
        list: (target, options = {}) => request("/_pam/fs", {
            operation: 3,
            target: normalizeTarget(target),
            content: null,
        }, nativeOptions(options)),
        metadata: (target, options = {}) => request("/_pam/fs", {
            operation: 4,
            target: normalizeTarget(target),
            content: null,
        }, nativeOptions(options)),
        createDirectory: (target, options = {}) => request("/_pam/fs", {
            operation: 5,
            target: normalizeTarget(target),
            content: null,
        }, nativeOptions(options)),
    });

    const dialogRequest = (kind, options = {}) => {
        if (options === null || typeof options !== "object" || Array.isArray(options)) {
            throw new TypeError("Pam dialog options must be an object.");
        }
        return request("/_pam/dialog", {
            kind,
            title: options.title ?? null,
            fileName: options.fileName ?? null,
            filters: options.filters ?? [],
            access: options.access ?? null,
        });
    };

    const dialog = Object.freeze({
        openFile: (options = {}) => dialogRequest(1, options),
        openFiles: (options = {}) => dialogRequest(2, options),
        saveFile: (options = {}) => dialogRequest(3, options),
        openDirectory: (options = {}) => dialogRequest(4, options),
    });

    const clipboard = Object.freeze({
        readText: async (options = {}) => {
            const data = await request("/_pam/clipboard", {
                operation: 1,
                text: null,
            }, nativeOptions(options));
            return data.text;
        },
        writeText: (text, options = {}) => {
            if (typeof text !== "string") {
                throw new TypeError("Pam clipboard writes require a string.");
            }
            return request("/_pam/clipboard", {
                operation: 2,
                text,
            }, nativeOptions(options));
        },
        clear: (options = {}) => request("/_pam/clipboard", {
            operation: 3,
            text: null,
        }, nativeOptions(options)),
    });

    const notification = Object.freeze({
        show: (options) => {
            if (options === null || typeof options !== "object" || Array.isArray(options)) {
                throw new TypeError("Pam notifications require an options object.");
            }
            return request("/_pam/notification", {
                title: options.title,
                body: options.body ?? "",
                urgency: options.urgency ?? 2,
            }, nativeOptions(options));
        },
    });

    const updater = Object.freeze({
        status: () => request("/_pam/update/status", {}),
        check: () => request("/_pam/update/check", {}),
        download: () => request("/_pam/update/download", {}),
        install: () => request("/_pam/update/install", {}),
    });

    const plugins = Object.freeze({
        invoke: (plugin, command, payload = null, options = {}) => {
            if (typeof plugin !== "string" || plugin.length === 0) {
                throw new TypeError("Pam Rust plugin identifiers must be non-empty strings.");
            }
            if (typeof command !== "string" || command.length === 0) {
                throw new TypeError("Pam Rust plugin commands must be non-empty strings.");
            }
            return request("/_pam/plugin/invoke", {
                plugin,
                command,
                payload,
            }, nativeOptions(options));
        },
    });

    const on = (name, listener) => {
        if (typeof name !== "string" || typeof listener !== "function") {
            throw new TypeError("Pam event listeners require a name and function.");
        }
        const registered = listeners.get(name) ?? new Set();
        registered.add(listener);
        listeners.set(name, registered);
        return () => {
            registered.delete(listener);
            if (registered.size === 0) listeners.delete(name);
        };
    };

    const poll = async () => {
        while (!stopped) {
            try {
                const response = await fetch("/_pam/events", {
                    method: "POST",
                    headers,
                    body: JSON.stringify({ after: cursor, windowId }),
                });
                if (!response.ok) throw new Error(`Pam event poll failed with ${response.status}.`);
                const envelope = await response.json();
                cursor = envelope.cursor;
                for (const event of envelope.events) dispatch(event);
            } catch (error) {
                if (!stopped) {
                    console.warn("Pam event stream reconnecting.", error);
                    await new Promise((resolve) => setTimeout(resolve, 250));
                }
            }
        }
    };

    window.addEventListener("beforeunload", () => {
        stopped = true;
    }, { once: true });

    Object.defineProperty(window, "pam", {
        configurable: false,
        enumerable: true,
        writable: false,
        value: Object.freeze({
            clipboard,
            dialog,
            emit,
            fs: filesystem,
            invoke,
            notification,
            on,
            plugins,
            updater,
            windowId,
        }),
    });
    void poll();
})();
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_bridge_identifiers() {
        assert!(validate_identifier("greet", "command").is_ok());
        assert!(validate_identifier("window.set-title", "command").is_ok());
        assert!(validate_identifier("@pam/boot", "command").is_err());
        assert!(validate_identifier("../escape", "command").is_err());
        assert!(validate_identifier("", "command").is_err());
    }

    #[test]
    fn compares_bridge_tokens_without_early_content_exit() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"diff"));
        assert!(!constant_time_eq(b"short", b"longer"));
    }

    #[test]
    fn publishes_the_native_capability_surface_as_frozen_namespaces() {
        assert!(BRIDGE_SCRIPT.contains("const filesystem = Object.freeze({"));
        assert!(BRIDGE_SCRIPT.contains("const dialog = Object.freeze({"));
        assert!(BRIDGE_SCRIPT.contains("const clipboard = Object.freeze({"));
        assert!(BRIDGE_SCRIPT.contains("const notification = Object.freeze({"));
        assert!(BRIDGE_SCRIPT.contains("const updater = Object.freeze({"));
        assert!(BRIDGE_SCRIPT.contains("const plugins = Object.freeze({"));
        assert!(BRIDGE_SCRIPT.contains("value: Object.freeze({"));
        assert!(BRIDGE_SCRIPT.contains("fs: filesystem,"));
        assert!(BRIDGE_SCRIPT.contains("updater,"));
        assert!(BRIDGE_SCRIPT.contains("plugins,"));
    }
}
