//! Glue between the PHP worker, the layout engine, the scene and a backend.
//!
//! Everything here runs on the UI thread. Background threads (the worker
//! reader, module workers) post into the [`Inbox`] and wake the UI thread;
//! backends push [`BackendEvent`]s into the [`EventQueue`]. One `drain` then
//! applies everything and flushes the scene and the worker once.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use pam_desktop_engine::{Engine, PamTextMeasureRequest, PamTextMeasureResult};
use pam_desktop_wire::decode_batch;

use crate::animator::{Animator, Owner, Timer, Timers};
use crate::backend::{Backend, BackendEvent, KeyEvent, NodeId, WindowConfig, WindowMetrics, MAIN_WINDOW};
use crate::context::{event_kind, HostCommand, OutEvent};
use crate::controls::Controls;
use crate::geometry::Size;
use crate::inputs::Inputs;
use crate::interaction::Interaction;
use crate::modules::Registry;
use crate::motion::Motion;
use crate::runtime_modules::Settings;
use crate::runtime_shell::Shell;
use crate::presenter::coverage::Coverage;
use crate::presenter::modal::Modals;
use crate::presenter::navigation::Navigation;
use crate::presenter::tabs::Tabs;
use crate::scene::{Action, Change, Scene};
use crate::scrolling::Scrolling;
use crate::text::TextSource;
use crate::wiremap::WireMap;
use crate::worker::{Worker, WorkerCommand, WorkerMessage};

pub const MODULE_SUCCESS: u8 = 1;
pub const MODULE_FAILURE: u8 = 2;

/// Work posted to the UI thread from other threads.
pub enum Inbound {
    /// A message from the worker process of the given generation.
    Worker(u64, WorkerMessage),
    /// The answer of a native module to the worker of `generation`.
    ModuleResult { generation: u64, request: u64, status: u8, payload: Vec<u8> },
    /// Development: application sources changed, restart the worker.
    Reload,
    /// An item of a node's context menu was chosen.
    ContextMenuAction { node: NodeId, id: String },
    /// A link arrived while the application runs (a second launch).
    OpenUrl(String),
    /// Progress or result of an image download.
    Image(crate::images::ImageInbound),
    /// Progress or result of a media cache download.
    Media(crate::media::MediaInbound),
}

/// Thread-safe queue into the UI thread.
pub struct Inbox {
    queue: Mutex<VecDeque<Inbound>>,
    wake_pending: AtomicBool,
    waker: Box<dyn Fn() + Send + Sync>,
}

impl Inbox {
    /// `waker` must schedule [`pump`] on the UI thread; it may be called from any thread.
    pub fn new(waker: impl Fn() + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self { queue: Mutex::new(VecDeque::new()), wake_pending: AtomicBool::new(false), waker: Box::new(waker) })
    }

    pub fn post(&self, item: Inbound) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.push_back(item);
        }
        if !self.wake_pending.swap(true, Ordering::AcqRel) {
            (self.waker)();
        }
    }

    fn take(&self) -> VecDeque<Inbound> {
        self.wake_pending.store(false, Ordering::Release);
        self.queue.lock().map(|mut queue| std::mem::take(&mut *queue)).unwrap_or_default()
    }
}

/// Events from the backend, pushed from native callbacks on the UI thread.
#[derive(Default)]
pub struct EventQueue {
    events: RefCell<VecDeque<BackendEvent>>,
}

impl EventQueue {
    #[must_use]
    pub fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    /// Queues an event and drains the runtime unless it is already draining.
    pub fn push(&self, event: BackendEvent) {
        self.events.borrow_mut().push_back(event);
        pump();
    }

    /// Queues an event without draining (the caller pumps later).
    pub fn push_deferred(&self, event: BackendEvent) {
        self.events.borrow_mut().push_back(event);
    }

    fn pop(&self) -> Option<BackendEvent> {
        self.events.borrow_mut().pop_front()
    }
}

thread_local! {
    static PUMP: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
}

/// Installs the UI thread's drain function (see [`Shared::install`]).
pub fn set_pump(pump: Option<Rc<dyn Fn()>>) {
    PUMP.with(|slot| *slot.borrow_mut() = pump);
}

/// Drains the runtime of the calling (UI) thread. Safe to call re-entrantly:
/// a drain already in progress picks the new work up.
pub fn pump() {
    let pump = PUMP.with(|slot| slot.borrow().clone());
    if let Some(pump) = pump {
        pump();
    }
}

/// How the application's worker and first window are configured.
pub struct RuntimeConfig {
    pub worker: WorkerCommand,
    pub window: WindowConfig,
    /// Operating system keyword exposed to style queries: `linux`, `macos`, `windows`.
    pub platform: &'static str,
    /// Development keeps the window open when PHP stops and reloads on changes.
    pub development: bool,
    /// Directories whose changes restart the worker (development only).
    pub watch: Vec<std::path::PathBuf>,
    pub directories: Directories,
    /// `version` and `build` of the application manifest.
    pub app_version: String,
    pub build_number: String,
    /// The URL the application was launched with, if any.
    pub launch_url: Option<String>,
    /// Modules provided by the host binary (network, database, files…).
    pub modules: Vec<(String, Box<dyn crate::modules::Module>)>,
    /// The HTTP stack of the host binary; without one, network features fail.
    pub http_client: Option<Arc<dyn crate::modules::fetch::HttpClient>>,
}

/// Where the application keeps its files.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Directories {
    pub data: std::path::PathBuf,
    pub cache: std::path::PathBuf,
    pub state: std::path::PathBuf,
    /// The root of the application bundle, where `asset://` paths resolve.
    pub assets: std::path::PathBuf,
}

struct MeasureContext<B: Backend> {
    backend: *mut B,
}

unsafe extern "C" fn measure_trampoline<B: Backend>(
    context: *mut c_void,
    request: *const PamTextMeasureRequest,
    result: *mut PamTextMeasureResult,
) -> i32 {
    // SAFETY: the engine passes back the context installed by `Runtime` and
    // valid request/result pointers for the duration of the call.
    let (context, request, result) = unsafe { (&*context.cast::<MeasureContext<B>>(), &*request, &mut *result) };
    if context.backend.is_null() {
        return 0;
    }
    // SAFETY: the engine guarantees the lengths describe live UTF-8 buffers.
    let text = |pointer: *const u8, length: usize| -> &str {
        if pointer.is_null() || length == 0 {
            ""
        } else {
            unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(pointer, length)) }
        }
    };
    let source = TextSource {
        text: text(request.text, request.text_length),
        spans: text(request.spans, request.spans_length),
        family: text(request.font_family, request.font_family_length),
        features: text(request.font_features, request.font_features_length),
        font_size: request.font_size,
        font_scale: request.font_scale,
        letter_spacing: request.letter_spacing,
        line_height: request.line_height,
        weight: request.font_weight,
        italic: request.italic != 0,
        transform: i64::from(request.text_transform),
        max_lines: request.max_lines,
    };
    // An empty paragraph (an input without text) still occupies one line box.
    let empty = source.text.is_empty();
    let source = if empty { TextSource { text: " ", spans: "", ..source } } else { source };
    let block = source.measure_block();
    let max_width = if request.available_width.is_finite() && request.available_width > 0.0 {
        request.available_width
    } else {
        f32::INFINITY
    };
    // SAFETY: `backend` points at the runtime's backend for the duration of
    // the engine call that triggered this measurement.
    let mut metrics = unsafe { (*context.backend).measure_text(&block, max_width) };
    if empty {
        metrics.width = 0.0;
    }
    *result = PamTextMeasureResult {
        width: metrics.width,
        height: metrics.height,
        first_baseline: metrics.first_baseline,
        line_count: metrics.line_count,
    };
    1
}

pub struct Runtime<B: Backend> {
    pub backend: B,
    pub(crate) engine: Engine,
    pub(crate) scene: Scene,
    pub(crate) worker: Option<Worker>,
    worker_command: WorkerCommand,
    /// Bumped on every worker restart; messages of older workers are dropped.
    pub(crate) generation: u64,
    development: bool,
    pub(crate) inbox: Arc<Inbox>,
    events: Rc<EventQueue>,
    measure: Box<MeasureContext<B>>,
    batch: Vec<u8>,
    pub(crate) metrics: WindowMetrics,
    platform: &'static str,
    pub(crate) animator: Animator,
    pub(crate) timers: Timers,
    navigation: Navigation,
    tabs: Tabs,
    modals: Modals,
    coverage: Coverage,
    pub(crate) interaction: Interaction,
    pub(crate) inputs: Inputs,
    motion: Motion,
    pub(crate) controls: Controls,
    scrolling: Scrolling,
    images: crate::images::Images,
    drawings: crate::drawing::Drawings,
    media: crate::media::Media,
    web_views: crate::webview::WebViews,
    pub(crate) selection: crate::selection::TextSelection,
    pub(crate) modules: Registry,
    pub(crate) directories: Directories,
    pub(crate) shell: Shell,
    linking: crate::modules::system::Linking,
    pub(crate) settings: Settings,
    /// What the operating system reports, before the app's own preference.
    pub(crate) system_dark: bool,
    pub(crate) system_font_scale: f32,
    pub(crate) app_version: String,
    pub(crate) build_number: String,
    /// Whether the main window has keyboard focus (the app is "active").
    pub(crate) active: bool,
    /// Events for PHP produced while handling the current work.
    pub(crate) outbox: Vec<OutEvent>,
    pub(crate) commands: Vec<HostCommand>,
    pub(crate) started: Instant,
    draining: bool,
    quit_requested: bool,
    /// The deadline last handed to the backend (avoids re-arming every drain).
    armed_wake: Option<u64>,
    /// Set when the worker exited or the window closed: the host should quit.
    pub finished: bool,
}

/// Borrows the controller context out of a runtime's fields.
#[macro_export]
#[doc(hidden)]
macro_rules! ctx {
    ($runtime:expr) => {
        $crate::context::Ctx {
            scene: &mut $runtime.scene,
            animator: &mut $runtime.animator,
            timers: &mut $runtime.timers,
            events: &mut $runtime.outbox,
            commands: &mut $runtime.commands,
            now_ms: u64::try_from($runtime.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            viewport: $runtime.metrics.size,
            reduced_motion: $runtime.metrics.reduced_motion,
        }
    };
}

impl<B: Backend> Runtime<B> {
    pub fn new(mut backend: B, inbox: Arc<Inbox>, events: Rc<EventQueue>, config: RuntimeConfig) -> std::io::Result<Self> {
        backend.open_window(MAIN_WINDOW, &config.window);
        let system = backend.window_metrics(MAIN_WINDOW);
        let settings = Settings::load(&config.directories.state);
        backend.set_appearance(settings.forced_dark());
        // The window as the application sees it: its own appearance and text
        // scale preferences layered over the system's.
        let metrics = WindowMetrics {
            dark: settings.forced_dark().unwrap_or(system.dark),
            font_scale: settings.effective_font_scale(system.font_scale),
            ..system
        };
        let mut measure = Box::new(MeasureContext::<B> { backend: std::ptr::null_mut() });
        let engine = new_engine(&metrics, measure.as_mut());

        let mut scene = Scene::new();
        scene.environment.scale = metrics.scale;
        scene.environment.font_scale = metrics.font_scale;

        let mut modules = Registry::default();
        modules.register("timers", crate::modules::timers::Timers::new());
        modules.register("storage", crate::modules::storage::Storage::new(&config.directories.data));
        modules.register("device", crate::modules::system::Device::default());
        modules.register("permissions", crate::modules::system::Permissions);
        let linking = crate::modules::system::Linking::new(config.launch_url.clone());
        modules.register("linking", linking.clone());
        modules.register("cache", crate::modules::system::Cache::new(&config.directories.cache));
        modules.register("background", crate::modules::system::Background::default());
        modules.register("toast", crate::modules::system::Toast);
        if let Some(client) = &config.http_client {
            let directories = &config.directories;
            let files_root = crate::modules::files::sandbox_root(&directories.data);
            modules.register("files", crate::modules::files::Files::new(&directories.data, &directories.assets, Arc::clone(client)));
            modules.register("http", crate::modules::http::Http::new(&files_root, Arc::clone(client), config.development));
            modules.register("image", crate::modules::image::ImagePrefetch::new(&directories.cache, Arc::clone(client)));
        }
        let directories = config.directories.clone();
        let http_client = config.http_client.clone();
        let media = crate::media::Media::new(crate::media::MediaEnvironment {
            cache_root: directories.cache.join("pam-media-v1"),
            files_root: crate::modules::files::sandbox_root(&directories.data),
            assets: directories.assets.clone(),
            client: http_client.clone(),
            inbox: Arc::clone(&inbox),
        });
        let images = crate::images::Images::new(crate::images::ImageEnvironment {
            cache_root: crate::modules::image::cache_root(&directories.cache),
            files_root: crate::modules::files::sandbox_root(&directories.data),
            assets: directories.assets.clone(),
            client: http_client.clone(),
            inbox: Arc::clone(&inbox),
            allow_insecure: config.development,
        });
        for (name, module) in config.modules {
            modules.register_boxed(&name, module);
        }

        let mut command = config.worker;
        command.environment.push(("PAM_DESKTOP_PLATFORM".into(), config.platform.into()));
        command.environment.push(("PAM_RENDERER_CONTRACT".into(), renderer_contract_json()));
        command.environment.push((
            "PAM_DESKTOP_MODE".into(),
            if config.development { "development" } else { "production" }.into(),
        ));
        if let Some(url) = &config.launch_url {
            command.environment.push(("PAM_DESKTOP_LAUNCH_URL".into(), url.clone()));
        }
        let worker = spawn_worker(&command, &metrics, system.dark, settings.appearance_mode, &inbox, 1)?;
        if config.development && !config.watch.is_empty() {
            crate::watch::watch(config.watch, Arc::clone(&inbox));
        }

        Ok(Self {
            backend,
            engine,
            scene,
            worker: Some(worker),
            worker_command: command,
            generation: 1,
            development: config.development,
            inbox,
            events,
            measure,
            batch: Vec::with_capacity(64 * 1024),
            metrics,
            platform: config.platform,
            animator: Animator::new(),
            timers: Timers::default(),
            navigation: Navigation::default(),
            tabs: Tabs::default(),
            modals: Modals::default(),
            coverage: Coverage::default(),
            interaction: Interaction::new(metrics.dark),
            inputs: Inputs::default(),
            motion: Motion::default(),
            controls: Controls::new(metrics.dark),
            scrolling: Scrolling::default(),
            images,
            drawings: crate::drawing::Drawings::default(),
            media,
            web_views: crate::webview::WebViews::default(),
            selection: crate::selection::TextSelection::default(),
            modules,
            directories,
            shell: Shell::default(),
            linking,
            settings,
            system_dark: system.dark,
            system_font_scale: system.font_scale,
            app_version: config.app_version,
            build_number: config.build_number,
            active: true,
            outbox: Vec::new(),
            commands: Vec::new(),
            started: Instant::now(),
            draining: false,
            quit_requested: false,
            armed_wake: None,
            finished: false,
        })
    }

    /// The queue other threads use to reach the UI thread.
    #[must_use]
    pub fn inbox(&self) -> Arc<Inbox> {
        Arc::clone(&self.inbox)
    }

    #[must_use]
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    #[must_use]
    pub fn metrics(&self) -> WindowMetrics {
        self.metrics
    }

    pub(crate) fn now_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Applies all queued work, then flushes the scene and the worker once.
    pub fn drain(&mut self) {
        if self.draining {
            return;
        }
        self.draining = true;
        loop {
            let mut progressed = false;
            while let Some(event) = self.events.pop() {
                self.handle_backend_event(event);
                self.settle();
                progressed = true;
            }
            let inbound = self.inbox.take();
            for item in inbound {
                self.handle_inbound(item);
                self.settle();
                progressed = true;
            }
            if !progressed {
                break;
            }
        }
        {
            let mut ctx = ctx!(self);
            self.interaction.flush(&mut ctx);
            self.scrolling.flush(&mut ctx);
        }
        self.scene.flush(&mut self.backend, MAIN_WINDOW);
        for command in std::mem::take(&mut self.commands) {
            match command {
                HostCommand::Focus(id) => self.backend.focus(id),
                HostCommand::ScrollTo { id, offset, animated } => self.backend.scroll_to(id, offset, animated),
                HostCommand::InputConfig { id, config } => self.backend.set_input_config(id, &config),
                HostCommand::InputText { id, text, selection } => self.backend.set_input_text(id, &text, selection),
                HostCommand::InputSelection { id, start, end } => self.backend.set_input_selection(id, start, end),
                HostCommand::BeginWindowDrag => self.backend.begin_window_drag(MAIN_WINDOW),
                HostCommand::LoadImage { key, source, max_dimension } => self.backend.load_image(key, source, max_dimension),
                HostCommand::ReleaseImage(key) => self.backend.release_image(key),
                HostCommand::SetImage { id, content } => self.backend.set_image(id, content.as_ref()),
                HostCommand::SetDrawing { id, commands } => self.backend.set_drawing(id, &commands),
                HostCommand::SetTextSelection { id, range } => self.backend.set_text_selection(id, range),
                HostCommand::SetMedia { id, state } => self.backend.set_media(id, &state),
                HostCommand::SetWebView { id, state } => self.backend.set_web_view(id, &state),
            }
        }
        if let Some(worker) = &mut self.worker {
            for event in self.outbox.drain(..) {
                if std::env::var_os("PAM_DESKTOP_DEBUG").is_some() {
                    eprintln!("event node={} kind={} payload={:?}", event.node, event.kind, String::from_utf8_lossy(&event.payload));
                }
                worker.send_event(event.node, event.kind, &event.payload);
            }
            worker.flush();
        } else {
            self.outbox.clear();
        }
        if !self.animator.is_idle() {
            self.backend.request_frame();
        }
        let deadline = self.timers.next_deadline();
        if deadline != self.armed_wake {
            self.armed_wake = deadline;
            if let Some(deadline) = deadline {
                let delay = deadline.saturating_sub(self.now_ms()).min(u64::from(u32::MAX)) as u32;
                self.backend.schedule_wake(delay);
            }
        }
        self.draining = false;
        if self.finished && !self.quit_requested {
            self.quit_requested = true;
            self.worker = None;
            self.backend.quit();
        }
    }

    /// Lets every controller react to what changed in the scene, until the
    /// scene is quiet (controllers add chrome and overrides of their own).
    fn settle(&mut self) {
        for _ in 0..8 {
            let changes = self.scene.take_changes();
            if changes.is_empty() {
                break;
            }
            let mut ctx = ctx!(self);
            for change in changes {
                if let Change::Removed(id, _) = change {
                    ctx.animator.forget(id);
                }
                self.navigation.on_change(&mut ctx, change);
                self.tabs.on_change(&mut ctx, change);
                self.modals.on_change(&mut ctx, change);
                self.coverage.on_change(&mut ctx, change);
                self.interaction.on_change(&mut ctx, change);
                self.inputs.on_change(&mut ctx, change);
                self.motion.on_change(&mut ctx, change);
                self.controls.on_change(&mut ctx, change);
                self.scrolling.on_change(&mut ctx, change);
                self.images.on_change(&mut ctx, change);
                self.drawings.on_change(&mut ctx, change);
                self.media.on_change(&mut ctx, change);
                self.web_views.on_change(&mut ctx, change);
                self.selection.on_change(&mut ctx, change);
            }
            self.navigation.after_commit(&mut ctx);
            self.tabs.after_commit(&mut ctx);
            self.modals.after_commit(&mut ctx);
            self.inputs.after_commit(&mut ctx);
            self.motion.after_commit(&mut ctx);
            self.controls.after_commit(&mut ctx);
            self.scrolling.after_commit(&mut ctx);
            self.images.after_commit(&mut ctx);
            self.drawings.after_commit(&mut ctx);
            for (id, size) in self.images.take_natural_sizes() {
                self.drawings.set_natural_size(&mut ctx, id, size);
            }
            self.web_views.after_commit(&mut ctx);
            self.media.after_commit(&mut ctx);
            for (id, hidden) in self.media.take_poster_changes() {
                self.images.set_suppressed(&mut ctx, id, hidden);
            }
            self.coverage.invalidate();
            self.inputs.retry_autofocus();
        }
        let mut ctx = ctx!(self);
        self.coverage.evaluate(&mut ctx);
    }

    fn handle_inbound(&mut self, item: Inbound) {
        match item {
            Inbound::Worker(generation, _) if generation != self.generation => {}
            Inbound::Worker(_, WorkerMessage::Commit(frame)) => self.commit(&frame),
            Inbound::Worker(_, WorkerMessage::Call { request, module, method, payload }) => {
                self.call_module(request, &module, &method, &payload);
            }
            Inbound::Worker(_, WorkerMessage::CallTyped { request, operation, payload }) => {
                self.call_typed(request, operation, &payload);
            }
            Inbound::Worker(_, WorkerMessage::Error(message)) => {
                eprintln!("[pam-desktop] {}", describe_php_error(&message));
            }
            Inbound::Worker(_, WorkerMessage::Ready | WorkerMessage::Idle) => {}
            Inbound::Worker(_, WorkerMessage::Exited(_)) => {
                self.worker = None;
                if self.development {
                    // Keep the last frame on screen; the next change restarts PHP.
                    eprintln!("[pam-desktop] the PHP worker stopped; waiting for a change to restart it");
                } else {
                    self.finished = true;
                }
            }
            Inbound::ModuleResult { generation, request, status, payload } => {
                // Results asked by a worker that was reloaded away are stale.
                if generation == self.generation {
                    if let Some(worker) = &mut self.worker {
                        worker.send_module_result(request, status, &payload);
                    }
                }
            }
            Inbound::Reload => self.reload(),
            Inbound::ContextMenuAction { node, id } => self.context_menu_chosen(node, &id),
            Inbound::OpenUrl(url) => self.linking.deliver(&url),
            Inbound::Image(message) => {
                let mut ctx = ctx!(self);
                self.images.on_inbound(&mut ctx, message);
            }
            Inbound::Media(message) => {
                let mut ctx = ctx!(self);
                self.media.on_inbound(&mut ctx, message);
            }
        }
    }

    /// Restarts the PHP worker keeping the window: component state is
    /// checkpointed, and the first frame of the new worker is diffed against
    /// the tree that is already mounted.
    pub fn reload(&mut self) {
        if let Some(mut worker) = self.worker.take() {
            worker.checkpoint();
            worker.shutdown();
        }
        self.generation += 1;
        self.outbox.clear();
        self.modules.reset();
        self.shell.reset();
        match spawn_worker(&self.worker_command, &self.metrics, self.system_dark, self.settings.appearance_mode, &self.inbox, self.generation) {
            Ok(worker) => self.worker = Some(worker),
            Err(error) => eprintln!("[pam-desktop] could not restart the PHP worker: {error}"),
        }
    }

    fn with_engine<T>(&mut self, call: impl FnOnce(&mut Engine, &mut Vec<u8>) -> T) -> T {
        self.measure.backend = std::ptr::from_mut(&mut self.backend);
        self.batch.clear();
        let result = call(&mut self.engine, &mut self.batch);
        self.measure.backend = std::ptr::null_mut();
        result
    }

    fn apply_batch(&mut self) -> bool {
        if self.batch.is_empty() {
            return true;
        }
        match decode_batch(&self.batch) {
            Ok(mutations) => {
                self.scene.apply(mutations);
                true
            }
            Err(error) => {
                eprintln!("[pam-desktop] mutation batch rejected: {error}");
                false
            }
        }
    }

    fn commit(&mut self, frame: &[u8]) {
        let committed = self.with_engine(|engine, batch| engine.commit_into(frame, batch));
        let applied = match committed {
            Ok(()) => self.apply_batch(),
            Err(error) => {
                eprintln!("[pam-desktop] frame rejected: {error}");
                false
            }
        };
        if !applied {
            self.resynchronize();
        }
    }

    /// Drops the mounted tree and asks PHP for a complete frame.
    fn resynchronize(&mut self) {
        self.scene.clear(&mut self.backend);
        self.engine = new_engine(&self.metrics, self.measure.as_mut());
        if let Some(worker) = &mut self.worker {
            worker.resynchronize();
        }
    }

    pub(crate) fn relayout(&mut self) {
        let (width, height, font_scale) = (self.metrics.size.width, self.metrics.size.height, self.metrics.font_scale);
        let result = self.with_engine(|engine, batch| engine.relayout_with_metrics_into(width, height, font_scale, batch));
        match result {
            Ok(()) => {
                self.apply_batch();
            }
            Err(error) => eprintln!("[pam-desktop] relayout failed: {error}"),
        }
    }

    pub(crate) fn send_event(&mut self, node: NodeId, kind: u16, payload: &[u8]) {
        self.outbox.push(OutEvent { node, kind, payload: payload.to_vec() });
    }

    pub(crate) fn send_dimensions(&mut self) {
        let metrics = self.metrics;
        let payload = WireMap::new()
            .with("width", metrics.size.width)
            .with("height", metrics.size.height)
            .with("density", metrics.scale)
            .with("appearance", if metrics.dark { 2_i64 } else { 1_i64 })
            .with("appearanceMode", i64::from(self.settings.appearance_mode))
            .with("systemAppearance", if self.system_dark { 2_i64 } else { 1_i64 })
            .with("fontScale", metrics.font_scale)
            .with("safeAreaTop", 0.0_f64)
            .with("safeAreaRight", 0.0_f64)
            .with("safeAreaBottom", 0.0_f64)
            .with("safeAreaLeft", 0.0_f64)
            .with("refreshRate", metrics.refresh_rate)
            .with("reducedMotion", metrics.reduced_motion)
            .with("deviceType", "desktop")
            .with("pointer", "fine")
            .with("inputMode", "mouse")
            .with("displayMode", "window")
            .encode();
        self.send_event(0, event_kind::DIMENSIONS, &payload);
    }

    fn handle_backend_event(&mut self, event: BackendEvent) {
        match event {
            BackendEvent::Pointer(pointer) => {
                if pointer.window != MAIN_WINDOW {
                    return;
                }
                if std::env::var_os("PAM_DESKTOP_DEBUG").is_some() && pointer.phase != crate::backend::PointerPhase::Move {
                    eprintln!("pointer {:?} at {:?} hit {:?}", pointer.phase, pointer.position, self.scene.hit_test(pointer.position));
                }
                if self.select_text(&pointer) {
                    return;
                }
                {
                    // A freehand canvas owns the pointer while a stroke is drawn.
                    let mut ctx = ctx!(self);
                    if self.drawings.on_pointer(&mut ctx, pointer) {
                        return;
                    }
                }
                let action = {
                    let mut ctx = ctx!(self);
                    if pointer.phase == crate::backend::PointerPhase::Up {
                        self.inputs.flush_before_press(&mut ctx);
                    }
                    self.interaction.on_pointer(&mut ctx, pointer)
                };
                if let Some(action) = action {
                    self.run_action(action);
                }
                if pointer.phase == crate::backend::PointerPhase::Up && pointer.button == crate::backend::PointerButton::Secondary {
                    self.open_context_menu(pointer.position);
                }
            }
            BackendEvent::Key(key) => self.handle_key(&key),
            BackendEvent::BackRequested { window } => {
                if window == MAIN_WINDOW {
                    self.go_back();
                }
            }
            BackendEvent::FocusChanged { id, focused } => {
                let mut ctx = ctx!(self);
                self.inputs.on_focus(&mut ctx, id, focused);
                self.interaction.on_focus(&mut ctx, id, focused);
            }
            BackendEvent::Input { id, event } => {
                let mut ctx = ctx!(self);
                self.inputs.on_event(&mut ctx, id, event);
            }
            BackendEvent::WindowResized { window, size, scale } if window == MAIN_WINDOW => {
                let changed = self.metrics.size != size || self.metrics.scale != scale;
                if !changed {
                    return;
                }
                let scale_changed = self.metrics.scale != scale;
                self.metrics.size = size;
                self.metrics.scale = scale;
                self.scene.environment.scale = scale;
                if scale_changed {
                    self.scene.invalidate_all();
                }
                let _ = self.engine.set_viewport(size.width, size.height);
                self.relayout();
                self.send_dimensions();
            }
            BackendEvent::WindowResized { .. } => {}
            BackendEvent::WindowCloseRequested { window } => {
                if window == MAIN_WINDOW {
                    self.close_requested();
                }
            }
            BackendEvent::MenuAction { id } => self.shell.emit("menu", &id, "{}"),
            BackendEvent::Shortcut { id } => self.shell.emit("shortcut", &id, "{}"),
            BackendEvent::TrayActivated => self.shell.emit("tray", "activate", "{}"),
            BackendEvent::NotificationActivated { id } => self.shell.emit("notification", &id, "{}"),
            BackendEvent::FilesDropped { window, position, paths } => {
                if window == MAIN_WINDOW {
                    self.files_dropped(position, &paths);
                }
            }
            BackendEvent::WindowStateChanged { window, state } => {
                if window == MAIN_WINDOW {
                    self.window_state_changed(state);
                }
            }
            BackendEvent::WindowFocusChanged { window, focused } => {
                if window == MAIN_WINDOW {
                    self.active = focused;
                    let state = if focused { b"1" } else { b"2" };
                    self.send_event(0, event_kind::APP_STATE, state);
                }
            }
            BackendEvent::AppearanceChanged { dark } => {
                self.system_dark = dark;
                self.apply_appearance();
            }
            BackendEvent::Frame { .. } => self.tick(),
            BackendEvent::Wake => {
                self.armed_wake = None;
                let due = self.timers.take_due(self.now_ms());
                let mut ctx = ctx!(self);
                for timer in due {
                    match timer {
                        Timer::InputDebounce(id) => self.inputs.on_timer(&mut ctx, id),
                        Timer::WebViewTimeout(id) => self.web_views.on_timer(&mut ctx, id),
                        Timer::MediaProgress(id) | Timer::MediaRelease(id) => {
                            let position = self.backend.media_position(id);
                            self.media.on_timer(&mut ctx, timer, position);
                            for (id, hidden) in self.media.take_poster_changes() {
                                self.images.set_suppressed(&mut ctx, id, hidden);
                            }
                        }
                        Timer::ScrollRest(_) | Timer::ListWindow(_) | Timer::RefreshFallback(_) => self.scrolling.on_timer(&mut ctx, timer),
                        _ => self.interaction.on_timer(&mut ctx, timer),
                    }
                }
            }
            BackendEvent::ScrollChanged { id, offset } => {
                let mut ctx = ctx!(self);
                self.scrolling.on_scroll(&mut ctx, id, offset);
                // Players that start when scrolled into view follow the offset.
                self.media.on_scroll();
                self.media.evaluate(&mut ctx);
            }
            // Gestures, text selection and drag and drop are wired below as
            // their controllers land.
            BackendEvent::Wheel { .. } | BackendEvent::Magnify { .. } | BackendEvent::Rotate { .. } | BackendEvent::TextDropped { .. } | BackendEvent::DragFinished { .. } => {}
            BackendEvent::WebView { id, event } => {
                let mut ctx = ctx!(self);
                self.web_views.on_event(&mut ctx, id, event);
            }
            BackendEvent::Media { id, event } => {
                let mut ctx = ctx!(self);
                self.media.on_event(&mut ctx, id, event);
                for (id, hidden) in self.media.take_poster_changes() {
                    self.images.set_suppressed(&mut ctx, id, hidden);
                }
            }
            BackendEvent::AccessibilityAction { id, name } => {
                if name == "activate" {
                    let action = {
                        let mut ctx = ctx!(self);
                        crate::interaction::Interaction::activate(&mut ctx, id)
                    };
                    if let Some(action) = action {
                        self.run_action(action);
                    }
                } else {
                    let mut ctx = ctx!(self);
                    ctx.emit_if(id, pam_desktop_wire::PropKey::OnAccessibilityAction, event_kind::ACCESSIBILITY_ACTION, name.into_bytes());
                }
            }
            BackendEvent::ImageLoaded { key, size } => {
                let mut ctx = ctx!(self);
                self.images.loaded(&mut ctx, key, size);
                for (id, size) in self.images.take_natural_sizes() {
                    self.drawings.set_natural_size(&mut ctx, id, size);
                }
            }
            BackendEvent::ImageFailed { key, message } => {
                let mut ctx = ctx!(self);
                self.images.failed(&mut ctx, key, &message);
                for (id, size) in self.images.take_natural_sizes() {
                    self.drawings.set_natural_size(&mut ctx, id, size);
                }
            }
        }
    }

    /// One display frame: every running animation advances.
    fn tick(&mut self) {
        let now = self.now_ms();
        let samples = self.animator.tick(now);
        let mut ctx = ctx!(self);
        for sample in samples {
            match sample.owner {
                Owner::Navigation(id) => {
                    self.navigation.on_sample(&mut ctx, id, sample.value, sample.finished);
                    if sample.finished {
                        self.coverage.invalidate();
                    }
                }
                Owner::ModalBackdrop(_) | Owner::ModalContent(_) => {
                    self.modals.on_sample(&mut ctx, sample.owner, sample.value, sample.finished);
                }
                Owner::Press(id) => self.interaction.on_press_sample(&mut ctx, id, sample.value),
                Owner::Control(id) => self.controls.on_sample(&mut ctx, id, sample.value, sample.finished),
                Owner::Scroll(id) => self.scrolling.on_sample(&mut ctx, id, sample.value),
                Owner::Image(id) => self.images.on_sample(&mut ctx, id, sample.value),
                Owner::Transition(..) | Owner::Motion(..) => self.motion.on_sample(&mut ctx, sample.owner, sample.value, sample.finished),
            }
        }
    }

    fn run_action(&mut self, action: Action) {
        let mut ctx = ctx!(self);
        match action {
            Action::TabSelect { host, index } => {
                self.tabs.select(&mut ctx, host, index);
                self.coverage.invalidate();
            }
            Action::ModalBackdrop { modal } => self.modals.backdrop_pressed(&mut ctx, modal),
            Action::Toggle { id } => self.controls.toggle(&mut ctx, id),
            Action::NavigationBack { .. } => {
                drop(ctx);
                self.go_back();
            }
        }
    }

    /// A back request: a presented modal takes it, otherwise PHP navigates.
    fn go_back(&mut self) {
        let consumed = {
            let mut ctx = ctx!(self);
            self.modals.request_close_top(&mut ctx)
        };
        if !consumed {
            self.send_event(0, event_kind::BACK, b"");
        }
    }

    /// Routes a pointer event to the text selection. `true` means it was
    /// consumed.
    fn select_text(&mut self, pointer: &crate::backend::PointerEvent) -> bool {
        use crate::backend::{PointerButton, PointerPhase};
        use crate::selection::TextSelection;
        if pointer.button != PointerButton::Primary && pointer.phase != PointerPhase::Move {
            return false;
        }
        let index_at = |runtime: &Self, id: NodeId| {
            let origin = runtime.scene.window_origin(id);
            runtime.backend.text_index_at(id, crate::geometry::Point::new(pointer.position.x - origin.x, pointer.position.y - origin.y))
        };
        match pointer.phase {
            PointerPhase::Down => {
                let target = self.scene.hit_test(pointer.position).and_then(|hit| TextSelection::target(&self.scene, hit));
                let index = target.and_then(|id| index_at(self, id));
                let mut ctx = ctx!(self);
                match (target, index) {
                    (Some(id), Some(index)) => {
                        self.selection.begin(&mut ctx, id, index, pointer.click_count, pointer.modifiers.shift);
                        true
                    }
                    _ => {
                        // A press anywhere else drops the selection.
                        self.selection.clear(&mut ctx);
                        false
                    }
                }
            }
            PointerPhase::Move if self.selection.is_dragging() => {
                if let Some(index) = self.selection.node().and_then(|id| index_at(self, id)) {
                    let mut ctx = ctx!(self);
                    self.selection.extend(&mut ctx, index);
                }
                true
            }
            PointerPhase::Up | PointerPhase::Cancel if self.selection.is_dragging() => {
                self.selection.end();
                true
            }
            _ => false,
        }
    }

    /// Copy, select all and escape for the selected text. `true` means the
    /// key was consumed.
    fn selection_key(&mut self, key: &KeyEvent) -> bool {
        if self.selection.node().is_none() {
            return false;
        }
        let command = if cfg!(target_os = "macos") { key.modifiers.meta } else { key.modifiers.control };
        match key.key.to_ascii_lowercase().as_str() {
            "c" if command => match self.selection.text() {
                Some(text) => {
                    self.backend.clipboard_set_text(&text);
                    true
                }
                None => false,
            },
            "a" if command => {
                let mut ctx = ctx!(self);
                self.selection.select_all(&mut ctx);
                true
            }
            "escape" if self.selection.text().is_some() => {
                let mut ctx = ctx!(self);
                self.selection.clear(&mut ctx);
                true
            }
            _ => false,
        }
    }

    fn handle_key(&mut self, key: &KeyEvent) {
        if key.window != MAIN_WINDOW {
            return;
        }
        if key.pressed && !key.repeat && self.selection_key(key) {
            return;
        }
        if key.pressed && !key.repeat {
            let back = match key.key.as_str() {
                // The conventional "back" chords of each system.
                "ArrowLeft" => key.modifiers.alt && !cfg!(target_os = "macos"),
                "[" => key.modifiers.meta && cfg!(target_os = "macos"),
                "BrowserBack" => true,
                _ => false,
            };
            if back {
                return self.go_back();
            }
            if let Some(id) = self.shell.shortcut_for(&key.key, key.modifiers) {
                return self.shell.emit("shortcut", &id, "{}");
            }
            {
                let mut ctx = ctx!(self);
                if self.scrolling.on_key(&mut ctx, key) {
                    return;
                }
            }
            if key.key == "Escape" {
                let mut ctx = ctx!(self);
                if self.drawings.cancel(&mut ctx) {
                    return;
                }
                if self.modals.request_close_top(&mut ctx) {
                    return;
                }
            }
        }
        let action = {
            let mut ctx = ctx!(self);
            self.interaction.report_key(&mut ctx, key);
            self.interaction.on_key(&mut ctx, key)
        };
        if let Some(action) = action {
            self.run_action(action);
        }
    }

    /// Milliseconds since the runtime started (diagnostics).
    #[must_use]
    pub fn uptime_ms(&self) -> u64 {
        self.now_ms()
    }

    #[must_use]
    pub fn platform(&self) -> &'static str {
        self.platform
    }
}

/// An engine configured for a desktop window, measuring text with the backend.
fn new_engine<B: Backend>(metrics: &WindowMetrics, measure: &mut MeasureContext<B>) -> Engine {
    let mut engine = Engine::new();
    let _ = engine.set_viewport(metrics.size.width, metrics.size.height);
    let _ = engine.set_text_scale(metrics.font_scale);
    let _ = engine.set_refresh_rate(f64::from(metrics.refresh_rate));
    // SAFETY: the context is heap allocated, owned by the runtime and outlives
    // the engine; its backend pointer is only set around engine calls.
    unsafe {
        engine.set_text_measurer(Some((
            measure_trampoline::<B>,
            std::ptr::from_mut::<MeasureContext<B>>(measure).cast::<c_void>(),
        )));
    }
    engine
}

/// A one-paragraph description of an error report from PHP
/// (`PAMERR1\n<json>` or a plain `Class: message`).
fn describe_php_error(report: &str) -> String {
    let Some(json) = report.strip_prefix("PAMERR1\n") else { return format!("PHP error: {report}") };
    let Ok(error) = serde_json::from_str::<serde_json::Value>(json) else { return format!("PHP error: {json}") };
    let text = |key: &str| error.get(key).and_then(serde_json::Value::as_str).unwrap_or("");
    let mut description = format!(
        "PHP {} in {} phase: {}\n    at {}:{}",
        text("type"),
        text("phase"),
        text("message"),
        text("file"),
        error.get("line").and_then(serde_json::Value::as_i64).unwrap_or(0),
    );
    if let Some(frames) = error.get("frames").and_then(serde_json::Value::as_array) {
        for frame in frames.iter().take(6) {
            let field = |key: &str| frame.get(key).and_then(serde_json::Value::as_str).unwrap_or("");
            description.push_str(&format!(
                "\n    {} ({}:{})",
                field("call"),
                field("file"),
                frame.get("line").and_then(serde_json::Value::as_i64).unwrap_or(0),
            ));
        }
    }
    description
}

/// Starts a worker that sees the window exactly as it is now.
fn spawn_worker(
    command: &WorkerCommand,
    metrics: &WindowMetrics,
    system_dark: bool,
    appearance_mode: u8,
    inbox: &Arc<Inbox>,
    generation: u64,
) -> std::io::Result<Worker> {
    let mut command = command.clone();
    command.environment.push(("PAM_BOOT_METRICS".into(), boot_metrics_json(metrics)));
    command.environment.push(("PAM_APPEARANCE_MODE".into(), appearance_mode.to_string()));
    command.environment.push(("PAM_SYSTEM_APPEARANCE".into(), if system_dark { "2" } else { "1" }.into()));
    command.environment.push(("PAM_SYSTEM_DARK".into(), if metrics.dark { "1" } else { "0" }.into()));
    let inbox = Arc::clone(inbox);
    Worker::spawn(&command, move |message| inbox.post(Inbound::Worker(generation, message)))
}

fn boot_metrics_json(metrics: &WindowMetrics) -> String {
    format!(
        concat!(
            "{{\"width\":{},\"height\":{},\"density\":{},\"fontScale\":{},",
            "\"safeAreaTop\":0,\"safeAreaRight\":0,\"safeAreaBottom\":0,\"safeAreaLeft\":0,",
            "\"refreshRate\":{},\"reducedMotion\":{},\"deviceType\":\"desktop\",",
            "\"pointer\":\"fine\",\"inputMode\":\"mouse\",\"displayMode\":\"window\"}}"
        ),
        metrics.size.width, metrics.size.height, metrics.scale, metrics.font_scale, metrics.refresh_rate, metrics.reduced_motion,
    )
}

/// The property and event ceilings this renderer implements.
fn renderer_contract_json() -> String {
    use pam_desktop_wire::{ABI_VERSION, CORE_EVENT_LIMIT, CORE_PROPERTY_LIMIT, MINIMUM_PROTOCOL_VERSION, PROTOCOL_VERSION};
    format!(
        "{{\"abi\":{ABI_VERSION},\"events\":[{CORE_EVENT_LIMIT},{}],\"properties\":[{CORE_PROPERTY_LIMIT},{}],\"protocol\":[{MINIMUM_PROTOCOL_VERSION},{PROTOCOL_VERSION}]}}",
        EXTENSION_EVENT_CEILING, EXTENSION_PROPERTY_CEILING,
    )
}

/// Highest extension identifiers implemented (the desktop table included).
const EXTENSION_PROPERTY_CEILING: u16 = pam_desktop_wire::DESKTOP_PROPERTY_CEILING;
const EXTENSION_EVENT_CEILING: u16 = pam_desktop_wire::DESKTOP_EVENT_CEILING;

/// A runtime shared with native callbacks on the UI thread.
pub struct Shared<B: Backend> {
    runtime: RefCell<Runtime<B>>,
}

impl<B: Backend + 'static> Shared<B> {
    /// Wraps the runtime and installs it as this thread's pump target.
    pub fn install(runtime: Runtime<B>) -> Rc<Self> {
        let shared = Rc::new(Self { runtime: RefCell::new(runtime) });
        let weak = Rc::downgrade(&shared);
        set_pump(Some(Rc::new(move || {
            if let Some(shared) = weak.upgrade() {
                if let Ok(mut runtime) = shared.runtime.try_borrow_mut() {
                    runtime.drain();
                }
            }
        })));
        shared
    }

    pub fn with<T>(&self, call: impl FnOnce(&mut Runtime<B>) -> T) -> Option<T> {
        self.runtime.try_borrow_mut().ok().map(|mut runtime| call(&mut runtime))
    }

    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.runtime.try_borrow().map(|runtime| runtime.finished).unwrap_or(false)
    }
}

/// Default size used before the native window reports its real one.
#[must_use]
pub fn default_window_size() -> Size {
    Size::new(960.0, 640.0)
}
