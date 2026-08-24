use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::Request;
use axum::extract::{Path as AxumPath, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, ORIGIN, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, Response, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use futures_util::StreamExt;
use getrandom::fill;
use pam_desktop_protocol::{
    Bootstrap, ClientEvent, CommandExecution, DialogKind, EVENT_COMMAND, ErrorCode, FileAccess,
    FileEntryKind, MAIN_WINDOW_ID, MAX_COMMAND_TIMEOUT_MS, MIN_COMMAND_TIMEOUT_MS,
    ResponseEnvelope, ResponseStatus, validate_identifier,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::sync::oneshot;
use tokio_util::io::ReaderStream;
use winit::event_loop::EventLoopProxy;

use crate::database::DatabaseRequest;
use crate::desktop_otlp::{
    CommandOutcome, DesktopOtlpExporter, OtlpCounters, TraceParent, epoch_nanos, parse_traceparent,
};
use crate::desktop_portal::DesktopPortalRequest;
use crate::dev_event::{self, EventCode};
use crate::diagnostic_session::DiagnosticSession;
use crate::event_hub::{EventHub, PublishedEvent};
use crate::file_watch::{FileWatchManager, FileWatchRequest};
use crate::host_event::HostEvent;
use crate::http_client::HttpRequest;
use crate::native::{
    ClipboardRequest, DialogBridgeRequest, DialogRequest, FileRequest, MAX_STREAM_BYTES,
    NativeError, NativeServices, NotificationRequest,
};
use crate::plugin::{PluginError, PluginSupervisor};
use crate::process_runner::ProcessRequest;
use crate::project::Project;
use crate::scheduler::BackgroundScheduler;
use crate::secret_store::SecretRequest;
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
    _diagnostic_session: Option<DiagnosticSession>,
}

impl Gateway {
    #[allow(
        clippy::too_many_lines,
        reason = "gateway startup keeps atomic service assembly and route registration together"
    )]
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
        let otlp = DesktopOtlpExporter::from_environment()?;
        let otlp_counters = otlp
            .as_ref()
            .map_or_else(OtlpCounters::default, |exporter| exporter.counters.clone());

        let parallel_workers =
            WorkerPool::prepare(&supervisor, requested_parallel_count(&bootstrap))?;
        let state = GatewayState {
            project: project.clone(),
            public_root,
            origin: url.trim_end_matches('/').to_owned(),
            token,
            bootstrap: Arc::new(RwLock::new(bootstrap)),
            supervisor: Arc::new(Mutex::new(supervisor)),
            parallel_workers: Arc::new(RwLock::new(Arc::new(parallel_workers))),
            cancellations: Arc::new(Mutex::new(HashMap::new())),
            events: EventHub::default(),
            event_proxy,
            native: Arc::new(RwLock::new(Arc::new(native))),
            updater: Arc::new(RwLock::new(updater)),
            plugins: Arc::new(RwLock::new(Arc::new(plugins))),
            scheduler: Arc::new(Mutex::new(None)),
            metrics: Arc::new(GatewayMetrics {
                otlp: otlp_counters,
                ..GatewayMetrics::default()
            }),
            otlp,
            file_watches: Arc::new(FileWatchManager::new()),
            development: watch,
        };
        state.replace_scheduler(&background_jobs)?;
        let router = Router::new()
            .route("/", get(serve_main))
            .route("/_pam/window/{window}", get(serve_window))
            .route("/_pam/bridge.js", get(serve_bridge))
            .route("/_pam/inspector", get(serve_inspector))
            .route("/_pam/inspector.css", get(serve_inspector_css))
            .route("/_pam/inspector.js", get(serve_inspector_js))
            .route("/_pam/invoke", post(invoke))
            .route("/_pam/emit", post(emit))
            .route("/_pam/cancel", post(cancel))
            .route("/_pam/events", post(events))
            .route("/_pam/fs", post(filesystem))
            .route("/_pam/fs/read-stream", post(filesystem_read_stream))
            .route("/_pam/fs/write-stream", post(filesystem_write_stream))
            .route("/_pam/dialog", post(dialog))
            .route("/_pam/clipboard", post(clipboard))
            .route("/_pam/notification", post(notification))
            .route("/_pam/database", post(database))
            .route("/_pam/system", post(system_information))
            .route("/_pam/http", post(http_request))
            .route("/_pam/secrets", post(secrets))
            .route("/_pam/process", post(process))
            .route("/_pam/fs/watch", post(file_watch))
            .route("/_pam/portal", post(desktop_portal))
            .route("/_pam/diagnostics", post(diagnostics))
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
        let diagnostic_session = if watch {
            let window_id = {
                let bootstrap = state
                    .bootstrap
                    .read()
                    .map_err(|_| "desktop bootstrap lock is poisoned".to_owned())?;
                bootstrap
                    .windows
                    .first()
                    .map(|window| window.id.clone())
                    .ok_or_else(|| "desktop project has no diagnostic source window".to_owned())?
            };
            Some(DiagnosticSession::create(
                project.root(),
                &url,
                &state.token,
                &window_id,
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
            _diagnostic_session: diagnostic_session,
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

    #[must_use]
    pub fn event_hub(&self) -> EventHub {
        self.state.events.clone()
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
    parallel_workers: Arc<RwLock<Arc<WorkerPool>>>,
    cancellations: Arc<Mutex<HashMap<RequestKey, CancellationToken>>>,
    events: EventHub,
    event_proxy: EventLoopProxy<HostEvent>,
    native: Arc<RwLock<Arc<NativeServices>>>,
    updater: Arc<RwLock<Updater>>,
    plugins: Arc<RwLock<Arc<PluginSupervisor>>>,
    scheduler: Arc<Mutex<Option<BackgroundScheduler>>>,
    metrics: Arc<GatewayMetrics>,
    otlp: Option<DesktopOtlpExporter>,
    file_watches: Arc<FileWatchManager>,
    development: bool,
}

impl GatewayState {
    fn command_execution(&self, name: &str) -> CommandExecution {
        self.bootstrap
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .commands
            .iter()
            .find(|command| command.name == name)
            .map_or(CommandExecution::Stateful, |command| command.execution)
    }

    fn parallel_workers(&self) -> Arc<WorkerPool> {
        self.parallel_workers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
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

    fn application_id(&self) -> String {
        self.bootstrap
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .manifest
            .identifier
            .clone()
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
        let reload_code = match kind {
            ChangeKind::Assets => 1,
            ChangeKind::Runtime => 2,
        };
        self.emit_reload_started(reload_code);
        match kind {
            ChangeKind::Assets => {
                let _ = self.event_proxy.send_event(HostEvent::ReloadViews);
                self.events.publish(ClientEvent {
                    name: "pam.dev.reloaded".to_owned(),
                    payload: serde_json::json!({"kind": 1}),
                    window_id: None,
                });
                dev_event::emit(
                    EventCode::ReloadSucceeded,
                    self.project.root(),
                    &serde_json::json!({"reloadCode": reload_code}),
                );
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
                        let parallel = self
                            .supervisor
                            .lock()
                            .map_err(|_| "PHP worker supervisor lock is poisoned".to_owned())
                            .and_then(|supervisor| {
                                WorkerPool::prepare(
                                    &supervisor,
                                    requested_parallel_count(&bootstrap),
                                )
                            })?;
                        Ok((bootstrap, native, plugins, parallel))
                    });
                self.finish_runtime_reload(result, reload_code);
            }
        }
    }

    fn emit_reload_started(&self, reload_code: u8) {
        dev_event::emit(
            EventCode::ChangeDetected,
            self.project.root(),
            &serde_json::json!({"reloadCode": reload_code}),
        );
        dev_event::emit(
            EventCode::ReloadStarted,
            self.project.root(),
            &serde_json::json!({"reloadCode": reload_code}),
        );
    }

    fn finish_runtime_reload(
        &self,
        result: Result<(Bootstrap, NativeServices, PluginSupervisor, WorkerPool), String>,
        reload_code: u8,
    ) {
        match result {
            Ok((bootstrap, native, plugins, parallel)) => {
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
                *self
                    .parallel_workers
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(parallel);
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
                dev_event::emit(
                    EventCode::ReloadSucceeded,
                    self.project.root(),
                    &serde_json::json!({"reloadCode": reload_code}),
                );
            }
            Err(error) => {
                dev_event::emit(
                    EventCode::ReloadFailed,
                    self.project.root(),
                    &serde_json::json!({"reloadCode": reload_code, "message": error}),
                );
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

struct WorkerPool {
    workers: Vec<Mutex<WorkerSupervisor>>,
    next: AtomicUsize,
}

#[derive(Default)]
struct GatewayMetrics {
    total_commands: std::sync::atomic::AtomicU64,
    failed_commands: std::sync::atomic::AtomicU64,
    active_commands: std::sync::atomic::AtomicU64,
    total_command_nanoseconds: std::sync::atomic::AtomicU64,
    otlp: OtlpCounters,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticsSnapshot {
    schema_version: u8,
    surface_code: u8,
    captured_at_unix_ms: u64,
    total_commands: u64,
    failed_commands: u64,
    active_commands: u64,
    average_command_microseconds: u64,
    primary_worker_generation: u64,
    parallel_workers: usize,
    event_cursor: u64,
    otlp_spans_exported: u64,
    otlp_spans_dropped: u64,
    otlp_export_errors: u64,
    otlp_spans_rejected: u64,
}

fn requested_parallel_count(bootstrap: &Bootstrap) -> u8 {
    if bootstrap
        .commands
        .iter()
        .any(|command| command.execution != CommandExecution::Stateful)
    {
        bootstrap.parallel_worker_count
    } else {
        0
    }
}

impl WorkerPool {
    fn prepare(supervisor: &WorkerSupervisor, count: u8) -> Result<Self, String> {
        let mut workers = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            workers.push(Mutex::new(supervisor.fork()?));
        }
        Ok(Self {
            workers,
            next: AtomicUsize::new(0),
        })
    }

    fn request(
        &self,
        command: String,
        window_id: String,
        payload: Value,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<ResponseEnvelope, WorkerRequestError> {
        if self.workers.is_empty() {
            return Err(WorkerRequestError::Crashed(
                "parallel worker pool is not configured".to_owned(),
            ));
        }
        let index = self.next.fetch_add(1, Ordering::Relaxed) % self.workers.len();
        self.workers[index]
            .lock()
            .map_err(|_| {
                WorkerRequestError::Crashed("parallel worker lock is poisoned".to_owned())
            })?
            .request(command, window_id, payload, timeout, cancellation)
    }

    fn len(&self) -> usize {
        self.workers.len()
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
    traceparent: Option<String>,
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StreamReadRequest {
    window_id: String,
    target: crate::native::FileTarget,
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

async fn serve_inspector(State(state): State<GatewayState>) -> Response<Body> {
    if !state.development {
        return secure_response(
            StatusCode::NOT_FOUND,
            "text/plain; charset=utf-8",
            Body::from("Not found"),
        );
    }
    secure_response(
        StatusCode::OK,
        "text/html; charset=utf-8",
        Body::from(INSPECTOR_HTML),
    )
}

async fn serve_inspector_css(State(state): State<GatewayState>) -> Response<Body> {
    if !state.development {
        return secure_response(
            StatusCode::NOT_FOUND,
            "text/plain; charset=utf-8",
            Body::from("Not found"),
        );
    }
    secure_response(
        StatusCode::OK,
        "text/css; charset=utf-8",
        Body::from(INSPECTOR_CSS),
    )
}

async fn serve_inspector_js(State(state): State<GatewayState>) -> Response<Body> {
    if !state.development {
        return secure_response(
            StatusCode::NOT_FOUND,
            "text/plain; charset=utf-8",
            Body::from("Not found"),
        );
    }
    secure_response(
        StatusCode::OK,
        "text/javascript; charset=utf-8",
        Body::from(INSPECTOR_JS),
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
    let parent = match request.traceparent.as_deref() {
        Some(value) => match parse_traceparent(value) {
            Some(parent) => Some(parent),
            None => {
                return client_failure(
                    StatusCode::BAD_REQUEST,
                    ErrorCode::InvalidMessage,
                    "The traceparent is not a valid W3C version 00 context.",
                );
            }
        },
        None => None,
    };
    execute_request(
        state,
        request.request_id,
        request.window_id,
        request.timeout_ms,
        request.command,
        request.payload,
        parent,
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
        None,
    )
    .await
}

#[allow(
    clippy::too_many_lines,
    reason = "request lifecycle keeps validation, cancellation, metrics and response normalization auditable"
)]
async fn execute_request(
    state: GatewayState,
    request_id: u64,
    window_id: String,
    timeout_ms: Option<u64>,
    command: String,
    payload: Value,
    trace_parent: Option<TraceParent>,
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

    let execution = state.command_execution(&command);
    let started = Instant::now();
    let started_unix_nano = state.otlp.as_ref().map(|_| epoch_nanos());
    let span_command = command.clone();
    state.metrics.total_commands.fetch_add(1, Ordering::Relaxed);
    state
        .metrics
        .active_commands
        .fetch_add(1, Ordering::Relaxed);
    let supervisor = state.supervisor.clone();
    let parallel_workers = state.parallel_workers();
    let response = tokio::task::spawn_blocking(move || match execution {
        CommandExecution::Stateful => supervisor
            .lock()
            .map_err(|_| WorkerRequestError::Crashed("worker lock is poisoned".to_owned()))?
            .request(
                command,
                window_id,
                payload,
                Duration::from_millis(timeout_ms),
                &cancellation,
            ),
        CommandExecution::Parallel | CommandExecution::Background => parallel_workers.request(
            command,
            window_id,
            payload,
            Duration::from_millis(timeout_ms),
            &cancellation,
        ),
    })
    .await;
    state
        .metrics
        .active_commands
        .fetch_sub(1, Ordering::Relaxed);
    state.metrics.total_command_nanoseconds.fetch_add(
        u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
        Ordering::Relaxed,
    );
    state
        .cancellations
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&key);

    let response = match response {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            state
                .metrics
                .failed_commands
                .fetch_add(1, Ordering::Relaxed);
            export_command_span(
                &state,
                span_command,
                execution,
                CommandOutcome::WorkerFailure,
                started_unix_nano,
                trace_parent,
            );
            return worker_error_response(error);
        }
        Err(error) => {
            state
                .metrics
                .failed_commands
                .fetch_add(1, Ordering::Relaxed);
            export_command_span(
                &state,
                span_command,
                execution,
                CommandOutcome::TaskFailure,
                started_unix_nano,
                trace_parent,
            );
            return client_failure(
                StatusCode::BAD_GATEWAY,
                ErrorCode::WorkerCrashed,
                &format!("The PHP worker task failed: {error}"),
            );
        }
    };
    state.process_response(&response);

    if response.status == ResponseStatus::Failure {
        state
            .metrics
            .failed_commands
            .fetch_add(1, Ordering::Relaxed);
        export_command_span(
            &state,
            span_command,
            execution,
            CommandOutcome::HandlerFailure,
            started_unix_nano,
            trace_parent,
        );
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

    export_command_span(
        &state,
        span_command,
        execution,
        CommandOutcome::Success,
        started_unix_nano,
        trace_parent,
    );

    json_response(
        StatusCode::OK,
        &InvokeResponse {
            ok: true,
            data: response.payload,
            error: None,
        },
    )
}

fn export_command_span(
    state: &GatewayState,
    command: String,
    execution: CommandExecution,
    outcome: CommandOutcome,
    start_unix_nano: Option<u64>,
    parent: Option<TraceParent>,
) {
    if let Some(exporter) = &state.otlp
        && let Some(start_unix_nano) = start_unix_nano
    {
        exporter.export(command, execution, outcome, start_unix_nano, parent);
    }
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

async fn filesystem_read_stream(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<StreamReadRequest>,
) -> Response<Body> {
    if !authorized(&state, &headers) {
        return unauthorized_response();
    }
    if !state.has_window(&request.window_id) {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidMessage,
            "The streaming source window is invalid.",
        );
    }
    let native = state.native_services();
    let opened =
        tokio::task::spawn_blocking(move || native.open_read_stream(&request.target)).await;
    let (file, bytes) = match opened {
        Ok(Ok(opened)) => opened,
        Ok(Err(error)) => return native_failure(error),
        Err(error) => {
            return native_failure(NativeError {
                code: ErrorCode::NativeOperationFailed,
                message: format!("The streaming read task failed: {error}"),
            });
        }
    };
    let stream = ReaderStream::with_capacity(tokio::fs::File::from_std(file), 64 * 1024);
    let mut response = secure_response(
        StatusCode::OK,
        "application/octet-stream",
        Body::from_stream(stream),
    );
    if let Ok(length) = HeaderValue::from_str(&bytes.to_string()) {
        response
            .headers_mut()
            .insert(axum::http::header::CONTENT_LENGTH, length);
    }
    response
}

#[allow(
    clippy::too_many_lines,
    reason = "stream validation and bounded backpressure handling remain together for security review"
)]
async fn filesystem_write_stream(
    State(state): State<GatewayState>,
    request: Request,
) -> Response<Body> {
    let (parts, body) = request.into_parts();
    if !authorized(&state, &parts.headers) {
        return unauthorized_response();
    }
    let Some(window_id) = parts
        .headers
        .get("x-pam-window")
        .and_then(|value| value.to_str().ok())
    else {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidMessage,
            "The streaming write window is missing.",
        );
    };
    if !state.has_window(window_id) {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidMessage,
            "The streaming write window is invalid.",
        );
    }
    let Some(encoded_target) = parts
        .headers
        .get("x-pam-stream-target")
        .and_then(|value| value.to_str().ok())
    else {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidPayload,
            "The streaming write target is missing.",
        );
    };
    let Some(target) = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded_target)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<crate::native::FileTarget>(&bytes).ok())
    else {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidPayload,
            "The streaming write target is malformed.",
        );
    };
    if parts
        .headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|bytes| bytes > MAX_STREAM_BYTES)
    {
        return client_failure(
            StatusCode::PAYLOAD_TOO_LARGE,
            ErrorCode::ResourceTooLarge,
            "The streaming write exceeds the four-gibibyte limit.",
        );
    }
    let native = state.native_services();
    let opened = tokio::task::spawn_blocking(move || native.open_write_stream(&target)).await;
    let file = match opened {
        Ok(Ok(file)) => file,
        Ok(Err(error)) => return native_failure(error),
        Err(error) => {
            return native_failure(NativeError {
                code: ErrorCode::NativeOperationFailed,
                message: format!("The streaming write task failed: {error}"),
            });
        }
    };
    let mut file = tokio::fs::File::from_std(file);
    let mut stream = body.into_data_stream();
    let mut written = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                return native_failure(NativeError {
                    code: ErrorCode::NativeOperationFailed,
                    message: format!("Cannot read the streaming request body: {error}"),
                });
            }
        };
        written = written.saturating_add(chunk.len() as u64);
        if written > MAX_STREAM_BYTES {
            return client_failure(
                StatusCode::PAYLOAD_TOO_LARGE,
                ErrorCode::ResourceTooLarge,
                "The streaming write exceeds the four-gibibyte limit.",
            );
        }
        if let Err(error) = file.write_all(&chunk).await {
            return native_failure(NativeError {
                code: ErrorCode::NativeOperationFailed,
                message: format!("Cannot write the streaming destination: {error}"),
            });
        }
    }
    if let Err(error) = file.flush().await {
        return native_failure(NativeError {
            code: ErrorCode::NativeOperationFailed,
            message: format!("Cannot flush the streaming destination: {error}"),
        });
    }
    native_success(serde_json::json!({"bytesWritten": written}))
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

async fn database(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<DatabaseRequest>,
) -> Response<Body> {
    if !authorized(&state, &headers) {
        return unauthorized_response();
    }
    if !state.has_window(&request.window_id) {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidMessage,
            "The database source window is invalid.",
        );
    }
    let native = state.native_services();
    native_task(tokio::task::spawn_blocking(move || native.database(&request)).await)
}

async fn system_information(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<UpdateBridgeRequest>,
) -> Response<Body> {
    if !authorized(&state, &headers) {
        return unauthorized_response();
    }
    if !state.has_window(&request.window_id) {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidMessage,
            "The system information source window is invalid.",
        );
    }
    let native = state.native_services();
    native_task(tokio::task::spawn_blocking(move || native.system_information()).await)
}

async fn http_request(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<HttpRequest>,
) -> Response<Body> {
    if !authorized(&state, &headers) {
        return unauthorized_response();
    }
    if !state.has_window(&request.window_id) {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidMessage,
            "The native HTTP source window is invalid.",
        );
    }
    let native = state.native_services();
    native_task(tokio::task::spawn_blocking(move || native.http(&request)).await)
}

async fn secrets(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<SecretRequest>,
) -> Response<Body> {
    if !authorized(&state, &headers) {
        return unauthorized_response();
    }
    if !state.has_window(&request.window_id) {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidMessage,
            "The secret source window is invalid.",
        );
    }
    if !state.native_services().secrets_enabled() {
        return native_failure(NativeError::disabled("secrets"));
    }
    match crate::secret_store::execute(&state.application_id(), &request).await {
        Ok(response) => match serde_json::to_value(response) {
            Ok(value) => native_success(value),
            Err(error) => native_failure(NativeError::native("Cannot encode secret result", error)),
        },
        Err(error) => native_failure(error),
    }
}

async fn process(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<ProcessRequest>,
) -> Response<Body> {
    if !authorized(&state, &headers) {
        return unauthorized_response();
    }
    if !state.has_window(&request.window_id) {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidMessage,
            "The process source window is invalid.",
        );
    }
    let native = state.native_services();
    native_task(tokio::task::spawn_blocking(move || native.process(&request)).await)
}

async fn file_watch(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<FileWatchRequest>,
) -> Response<Body> {
    if !authorized(&state, &headers) {
        return unauthorized_response();
    }
    if !state.has_window(&request.window_id) {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidMessage,
            "The file watch source window is invalid.",
        );
    }
    let native = state.native_services();
    match state
        .file_watches
        .dispatch(&native, &state.events, &request)
    {
        Ok(()) => native_success(Value::Null),
        Err(error) => native_failure(error),
    }
}

async fn desktop_portal(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<DesktopPortalRequest>,
) -> Response<Body> {
    if !authorized(&state, &headers) {
        return unauthorized_response();
    }
    if !state.has_window(&request.window_id) {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidMessage,
            "The desktop portal source window is invalid.",
        );
    }
    let native = state.native_services();
    match crate::desktop_portal::execute(&native, &request).await {
        Ok(value) => native_success(value),
        Err(error) => native_failure(error),
    }
}

async fn diagnostics(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<UpdateBridgeRequest>,
) -> Response<Body> {
    if !authorized(&state, &headers) {
        return unauthorized_response();
    }
    if !state.has_window(&request.window_id) {
        return client_failure(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidMessage,
            "The diagnostics source window is invalid.",
        );
    }
    let total = state.metrics.total_commands.load(Ordering::Relaxed);
    let nanos = state
        .metrics
        .total_command_nanoseconds
        .load(Ordering::Relaxed);
    let snapshot = DiagnosticsSnapshot {
        schema_version: 1,
        surface_code: 3,
        captured_at_unix_ms: u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        )
        .unwrap_or(u64::MAX),
        total_commands: total,
        failed_commands: state.metrics.failed_commands.load(Ordering::Relaxed),
        active_commands: state.metrics.active_commands.load(Ordering::Relaxed),
        average_command_microseconds: nanos.checked_div(total).unwrap_or(0) / 1_000,
        primary_worker_generation: state
            .supervisor
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .generation(),
        parallel_workers: state.parallel_workers().len(),
        event_cursor: state.events.latest_id(),
        otlp_spans_exported: state.metrics.otlp.exported.load(Ordering::Relaxed),
        otlp_spans_dropped: state.metrics.otlp.dropped.load(Ordering::Relaxed),
        otlp_export_errors: state.metrics.otlp.errors.load(Ordering::Relaxed),
        otlp_spans_rejected: state.metrics.otlp.rejected.load(Ordering::Relaxed),
    };
    native_success(serde_json::to_value(snapshot).unwrap_or(Value::Null))
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
            "default-src 'self'; base-uri 'none'; connect-src 'self'; font-src 'self'; frame-src 'none'; img-src 'self' data:; object-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'",
        ),
    );
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    );
    response
}

const INSPECTOR_HTML: &str = r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Pam Desktop Inspector</title><link rel="stylesheet" href="/_pam/inspector.css"><script defer src="/_pam/bridge.js"></script><script defer src="/_pam/inspector.js"></script></head>
<body><a class="skip" href="#metrics">Skip to metrics</a><header><div><p class="eyebrow">PAM DESKTOP · DEVELOPMENT</p><h1>Runtime Inspector</h1><p id="status" role="status" aria-live="polite">Connecting to the native host…</p></div><button id="refresh" type="button">Refresh now</button></header><main id="metrics" tabindex="-1"><section class="grid" aria-label="Runtime metrics"><article><span>Total commands</span><strong id="total">—</strong></article><article><span>Active now</span><strong id="active">—</strong></article><article><span>Failures</span><strong id="failed">—</strong></article><article><span>Average host time</span><strong id="average">—</strong></article><article><span>Worker generation</span><strong id="generation">—</strong></article><article><span>Parallel workers</span><strong id="workers">—</strong></article><article><span>Event cursor</span><strong id="cursor">—</strong></article></section><section class="note"><h2>Privacy boundary</h2><p>This inspector reads bounded aggregate counters only. Payloads, paths, SQL, secrets and bridge credentials are never collected.</p></section></main></body></html>"##;

const INSPECTOR_CSS: &str = r":root{color-scheme:dark;--bg:#0f172a;--surface:#172033;--surface-2:#1e293b;--text:#f8fafc;--muted:#cbd5e1;--border:#475569;--accent:#22c55e;--danger:#fb7185;--ring:#86efac;font-family:Inter,ui-sans-serif,system-ui,sans-serif}*{box-sizing:border-box}body{margin:0;min-height:100vh;background:radial-gradient(circle at 80% 0,#1e293b 0,transparent 38%),var(--bg);color:var(--text);padding:clamp(20px,4vw,56px)}header,main{max-width:1120px;margin-inline:auto}header{display:flex;align-items:end;justify-content:space-between;gap:24px;margin-bottom:32px}.eyebrow{color:var(--accent);font:600 12px/1.5 ui-monospace,monospace;letter-spacing:.12em;margin:0 0 8px}h1{font-size:clamp(30px,5vw,52px);line-height:1.05;letter-spacing:-.04em;margin:0}#status{color:var(--muted);margin:12px 0 0;line-height:1.5}button{min-height:44px;padding:0 18px;border:1px solid var(--border);border-radius:10px;background:var(--surface-2);color:var(--text);font:600 14px/1 system-ui;cursor:pointer;transition:background-color .18s ease,border-color .18s ease}button:hover{background:#334155;border-color:#64748b}button:focus-visible,.skip:focus-visible{outline:3px solid var(--ring);outline-offset:3px}.grid{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:12px}.grid article,.note{border:1px solid var(--border);border-radius:14px;background:color-mix(in srgb,var(--surface) 94%,transparent);box-shadow:0 14px 40px #02061740}.grid article{min-height:132px;padding:20px;display:flex;flex-direction:column;justify-content:space-between}.grid span{color:var(--muted);font-size:13px;line-height:1.4}.grid strong{font:650 clamp(24px,4vw,38px)/1 ui-monospace,monospace;font-variant-numeric:tabular-nums}.grid article[data-error=true] strong{color:var(--danger)}.note{padding:22px;margin-top:12px}.note h2{font-size:16px;margin:0 0 8px}.note p{max-width:72ch;color:var(--muted);line-height:1.65;margin:0}.skip{position:fixed;left:16px;top:16px;transform:translateY(-160%);background:var(--text);color:var(--bg);padding:12px 16px;border-radius:8px;z-index:10}.skip:focus{transform:none}@media(max-width:820px){.grid{grid-template-columns:repeat(2,minmax(0,1fr))}}@media(max-width:520px){body{padding:20px}header{align-items:stretch;flex-direction:column}.grid{grid-template-columns:1fr}.grid article{min-height:112px}}@media(prefers-reduced-motion:reduce){*,*::before,*::after{scroll-behavior:auto!important;transition:none!important}}";

const INSPECTOR_JS: &str = r##"(() => {"use strict";const ids={total:"totalCommands",active:"activeCommands",failed:"failedCommands",average:"averageCommandMicroseconds",generation:"primaryWorkerGeneration",workers:"parallelWorkers",cursor:"eventCursor"};const status=document.querySelector("#status");const refresh=document.querySelector("#refresh");let pending=false;const render=async()=>{if(pending)return;pending=true;refresh.disabled=true;try{const data=await window.pam.diagnostics.snapshot();for(const [id,key] of Object.entries(ids)){const node=document.getElementById(id);const value=data[key];node.textContent=id==="average"?`${Number(value).toLocaleString()} µs`:Number(value).toLocaleString();if(id==="failed")node.parentElement.dataset.error=String(value>0)}status.textContent=`Live · updated ${new Date().toLocaleTimeString()}`}catch(error){status.textContent=`Inspector unavailable · ${error.message}`}finally{pending=false;refresh.disabled=false}};refresh.addEventListener("click",render);render();setInterval(render,1000)})();"##;

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
            const metadata = options.includeRequestMetadata === true
                ? { requestId, timeoutMs: options.timeout ?? null }
                : {};
            const response = await fetch(endpoint, {
                method: "POST",
                headers,
                signal: controller.signal,
                body: JSON.stringify({
                    windowId,
                    ...metadata,
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
        if (options?.traceparent != null && typeof options.traceparent !== "string") {
            throw new TypeError("Pam traceparent must be a string.");
        }
        return request("/_pam/invoke", {
            command,
            payload,
            traceparent: options?.traceparent ?? null,
        }, { ...options, includeRequestMetadata: true });
    };

    const emit = (name, payload = null, options = {}) => {
        if (typeof name !== "string" || name.length === 0) {
            throw new TypeError("Pam event must have a non-empty name.");
        }
        return request("/_pam/emit", { name, payload }, {
            ...options,
            includeRequestMetadata: true,
        });
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

    const encodeStreamTarget = (target) => {
        const bytes = new TextEncoder().encode(JSON.stringify(normalizeTarget(target)));
        let binary = "";
        for (const byte of bytes) binary += String.fromCharCode(byte);
        return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
    };

    const nativeEnvelope = async (response) => {
        const envelope = await response.json();
        if (!envelope.ok) {
            const error = new Error(envelope.error?.message ?? "Pam native request failed.");
            error.code = envelope.error?.code ?? 8;
            throw error;
        }
        return envelope.data;
    };

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
        openRead: async (target, options = {}) => {
            const response = await fetch("/_pam/fs/read-stream", {
                method: "POST",
                headers,
                signal: options.signal,
                body: JSON.stringify({ windowId, target: normalizeTarget(target) }),
            });
            if (!response.ok) return nativeEnvelope(response);
            return Object.freeze({
                size: Number(response.headers.get("content-length") ?? 0),
                stream: response.body,
            });
        },
        writeStream: async (target, source, options = {}) => {
            if (!(source instanceof Blob) && !(source instanceof ArrayBuffer) && !ArrayBuffer.isView(source)
                && !(source instanceof ReadableStream)) {
                throw new TypeError("Pam streaming writes require a Blob, ArrayBuffer, typed array, or ReadableStream.");
            }
            const streamHeaders = {
                ...headers,
                "X-Pam-Window": windowId,
                "X-Pam-Stream-Target": encodeStreamTarget(target),
                "Content-Type": "application/octet-stream",
            };
            const init = {
                method: "POST",
                headers: streamHeaders,
                signal: options.signal,
                body: source,
            };
            if (source instanceof ReadableStream) init.duplex = "half";
            return nativeEnvelope(await fetch("/_pam/fs/write-stream", init));
        },
        watch: async (watchId, target, options = {}) => {
            await request("/_pam/fs/watch", { operation: 1, watchId, target }, nativeOptions(options));
            return Object.freeze({
                close: () => request("/_pam/fs/watch", { operation: 2, watchId }, nativeOptions({})),
            });
        },
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

    const normalizeDatabaseParameters = (parameters) => {
        if (!Array.isArray(parameters)) {
            throw new TypeError("Pam database parameters must be an array.");
        }
        return parameters;
    };

    const database = Object.freeze({
        query: async (name, sql, parameters = [], options = {}) => {
            if (typeof name !== "string" || typeof sql !== "string") {
                throw new TypeError("Pam database queries require a database name and SQL string.");
            }
            const data = await request("/_pam/database", {
                database: name,
                operation: 1,
                sql,
                parameters: normalizeDatabaseParameters(parameters),
                statements: [],
            }, nativeOptions(options));
            return data.rows;
        },
        execute: (name, sql, parameters = [], options = {}) => request("/_pam/database", {
            database: name,
            operation: 2,
            sql,
            parameters: normalizeDatabaseParameters(parameters),
            statements: [],
        }, nativeOptions(options)),
        transaction: (name, statements, options = {}) => {
            if (!Array.isArray(statements)) {
                throw new TypeError("Pam database transactions require an array of statements.");
            }
            return request("/_pam/database", {
                database: name,
                operation: 3,
                sql: "",
                parameters: [],
                statements,
            }, nativeOptions(options));
        },
    });

    const system = Object.freeze({
        snapshot: (options = {}) => request("/_pam/system", {}, nativeOptions(options)),
    });

    const http = Object.freeze({
        request: (origin, options = {}) => {
            if (typeof origin !== "string" || origin.length === 0) {
                throw new TypeError("Pam HTTP requests require a declared origin name.");
            }
            if (options === null || typeof options !== "object" || Array.isArray(options)) {
                throw new TypeError("Pam HTTP request options must be an object.");
            }
            return request("/_pam/http", {
                origin,
                method: options.method ?? 1,
                path: options.path ?? "/",
                headers: options.headers ?? {},
                body: options.body ?? "",
                bodyEncoding: options.bodyEncoding ?? 1,
                traceparent: options.traceparent ?? null,
            }, nativeOptions(options));
        },
    });

    const diagnostics = Object.freeze({
        snapshot: (options = {}) => request("/_pam/diagnostics", {}, nativeOptions(options)),
    });

    const secrets = Object.freeze({
        get: (key, options = {}) => request("/_pam/secrets", {
            operation: 1,
            key,
        }, nativeOptions(options)).then((result) => result.value),
        set: (key, value, options = {}) => request("/_pam/secrets", {
            operation: 2,
            key,
            value,
        }, nativeOptions(options)),
        delete: (key, options = {}) => request("/_pam/secrets", {
            operation: 3,
            key,
        }, nativeOptions(options)),
    });

    const process = Object.freeze({
        run: (command, options = {}) => {
            if (typeof command !== "string" || command.length === 0) {
                throw new TypeError("Pam process.run requires an authorized command name.");
            }
            return request("/_pam/process", {
                command,
                arguments: options.arguments ?? [],
                stdin: options.stdin ?? "",
                timeoutMs: options.timeout ?? 30_000,
            }, nativeOptions(options));
        },
    });

    const portal = Object.freeze({
        open: (url, options = {}) => request("/_pam/portal", { operation: 1, url }, nativeOptions(options)),
        screenshot: (options = {}) => request("/_pam/portal", { operation: 2 }, nativeOptions(options)),
        printPdf: (target, options = {}) => request("/_pam/portal", {
            operation: 3,
            target: normalizeTarget(target),
            title: options.title ?? "Pam Desktop",
        }, nativeOptions(options)),
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
            }, { ...nativeOptions(options), includeRequestMetadata: true });
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
            apiVersion: 1,
            clipboard,
            database,
            diagnostics,
            dialog,
            emit,
            fs: filesystem,
            http,
            invoke,
            notification,
            on,
            plugins,
            portal,
            process,
            secrets,
            system,
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
    fn bridge_exposes_trace_context_only_on_authenticated_invocations() {
        assert!(BRIDGE_SCRIPT.contains("traceparent: options?.traceparent ?? null"));
        assert!(BRIDGE_SCRIPT.contains("X-Pam-Bridge\": token"));
        assert!(
            parse_traceparent("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01").is_some()
        );
        assert!(parse_traceparent("forged").is_none());
    }

    #[test]
    fn bridge_keeps_command_metadata_out_of_strict_native_requests() {
        assert!(BRIDGE_SCRIPT.contains("const metadata = options.includeRequestMetadata === true"));
        assert!(BRIDGE_SCRIPT.contains("...options, includeRequestMetadata: true"));
        assert!(BRIDGE_SCRIPT.contains("}, nativeOptions(options));"));
    }

    #[test]
    fn diagnostics_use_the_cross_host_snapshot_envelope() {
        let snapshot = DiagnosticsSnapshot {
            schema_version: 1,
            surface_code: 3,
            captured_at_unix_ms: 42,
            total_commands: 0,
            failed_commands: 0,
            active_commands: 0,
            average_command_microseconds: 0,
            primary_worker_generation: 1,
            parallel_workers: 0,
            event_cursor: 0,
            otlp_spans_exported: 0,
            otlp_spans_dropped: 0,
            otlp_export_errors: 0,
            otlp_spans_rejected: 0,
        };
        let value = serde_json::to_value(snapshot).unwrap();
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["surfaceCode"], 3);
        assert_eq!(value["capturedAtUnixMs"], 42);
    }

    #[test]
    fn publishes_the_native_capability_surface_as_frozen_namespaces() {
        assert!(BRIDGE_SCRIPT.contains("const filesystem = Object.freeze({"));
        assert!(BRIDGE_SCRIPT.contains("const dialog = Object.freeze({"));
        assert!(BRIDGE_SCRIPT.contains("const clipboard = Object.freeze({"));
        assert!(BRIDGE_SCRIPT.contains("const notification = Object.freeze({"));
        assert!(BRIDGE_SCRIPT.contains("const database = Object.freeze({"));
        assert!(BRIDGE_SCRIPT.contains("const system = Object.freeze({"));
        assert!(BRIDGE_SCRIPT.contains("const http = Object.freeze({"));
        assert!(BRIDGE_SCRIPT.contains("const secrets = Object.freeze({"));
        assert!(BRIDGE_SCRIPT.contains("const process = Object.freeze({"));
        assert!(BRIDGE_SCRIPT.contains("const portal = Object.freeze({"));
        assert!(BRIDGE_SCRIPT.contains("const diagnostics = Object.freeze({"));
        assert!(BRIDGE_SCRIPT.contains("const updater = Object.freeze({"));
        assert!(BRIDGE_SCRIPT.contains("const plugins = Object.freeze({"));
        assert!(BRIDGE_SCRIPT.contains("value: Object.freeze({"));
        assert!(BRIDGE_SCRIPT.contains("apiVersion: 1,"));
        assert!(BRIDGE_SCRIPT.contains("fs: filesystem,"));
        assert!(BRIDGE_SCRIPT.contains("openRead: async (target, options = {})"));
        assert!(BRIDGE_SCRIPT.contains("writeStream: async (target, source, options = {})"));
        assert!(BRIDGE_SCRIPT.contains("updater,"));
        assert!(BRIDGE_SCRIPT.contains("plugins,"));
    }
}
