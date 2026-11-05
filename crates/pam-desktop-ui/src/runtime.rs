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
use pam_desktop_wire::{decode_batch, NodeKind, PropKey};

use crate::backend::{
    Backend, BackendEvent, NodeId, PointerButton, PointerEvent, PointerPhase, WindowConfig, WindowMetrics, MAIN_WINDOW,
};
use crate::geometry::{Point, Size};
use crate::scene::{IdMap, Scene};
use crate::text::TextSource;
use crate::wiremap::WireMap;
use crate::worker::{Worker, WorkerCommand, WorkerMessage};

/// Event kinds of the wire protocol (`Pam\Desktop\EventKind`).
pub mod event_kind {
    pub const PRESS: u16 = 1;
    pub const CHANGE: u16 = 2;
    pub const BACK: u16 = 3;
    pub const LONG_PRESS: u16 = 5;
    pub const FOCUS: u16 = 6;
    pub const BLUR: u16 = 7;
    pub const SUBMIT: u16 = 8;
    pub const SCROLL: u16 = 9;
    pub const APP_STATE: u16 = 16;
    pub const DIMENSIONS: u16 = 17;
    pub const PRESS_IN: u16 = 28;
    pub const PRESS_OUT: u16 = 29;
}

pub const MODULE_SUCCESS: u8 = 1;
pub const MODULE_FAILURE: u8 = 2;

/// Work posted to the UI thread from other threads.
pub enum Inbound {
    /// A message from the worker process of the given generation.
    Worker(u64, WorkerMessage),
    ModuleResult { request: u64, status: u8, payload: Vec<u8> },
    /// Development: application sources changed, restart the worker.
    Reload,
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
    let block = source.measure_block();
    let max_width = if request.available_width.is_finite() && request.available_width > 0.0 {
        request.available_width
    } else {
        f32::INFINITY
    };
    // SAFETY: `backend` points at the runtime's backend for the duration of
    // the engine call that triggered this measurement.
    let metrics = unsafe { (*context.backend).measure_text(&block, max_width) };
    *result = PamTextMeasureResult {
        width: metrics.width,
        height: metrics.height,
        first_baseline: metrics.first_baseline,
        line_count: metrics.line_count,
    };
    1
}

#[derive(Clone, Copy, Debug)]
struct PressState {
    node: NodeId,
    started: Point,
}

pub struct Runtime<B: Backend> {
    pub backend: B,
    engine: Engine,
    scene: Scene,
    worker: Option<Worker>,
    worker_command: WorkerCommand,
    /// Bumped on every worker restart; messages of older workers are dropped.
    generation: u64,
    development: bool,
    inbox: Arc<Inbox>,
    events: Rc<EventQueue>,
    measure: Box<MeasureContext<B>>,
    batch: Vec<u8>,
    metrics: WindowMetrics,
    platform: &'static str,
    scroll_offsets: IdMap<Point>,
    pressed: Option<PressState>,
    started: Instant,
    draining: bool,
    first_frame: bool,
    quit_requested: bool,
    /// Set when the worker exited or the window closed: the host should quit.
    pub finished: bool,
}

impl<B: Backend> Runtime<B> {
    pub fn new(mut backend: B, inbox: Arc<Inbox>, events: Rc<EventQueue>, config: RuntimeConfig) -> std::io::Result<Self> {
        backend.open_window(MAIN_WINDOW, &config.window);
        let metrics = backend.window_metrics(MAIN_WINDOW);
        let mut engine = Engine::new();
        let _ = engine.set_viewport(metrics.size.width, metrics.size.height);
        let _ = engine.set_text_scale(metrics.font_scale);
        let _ = engine.set_refresh_rate(f64::from(metrics.refresh_rate));

        let mut measure = Box::new(MeasureContext::<B> { backend: std::ptr::null_mut() });
        // SAFETY: the context is heap allocated, owned by this runtime and
        // outlives the engine; its backend pointer is only set around engine calls.
        unsafe {
            engine.set_text_measurer(Some((
                measure_trampoline::<B>,
                std::ptr::from_mut::<MeasureContext<B>>(measure.as_mut()).cast::<c_void>(),
            )));
        }

        let mut scene = Scene::new();
        scene.environment.scale = metrics.scale;
        scene.environment.font_scale = metrics.font_scale;

        let mut command = config.worker;
        command.environment.push(("PAM_DESKTOP_PLATFORM".into(), config.platform.into()));
        command.environment.push(("PAM_RENDERER_CONTRACT".into(), renderer_contract_json()));
        command.environment.push((
            "PAM_DESKTOP_MODE".into(),
            if config.development { "development" } else { "production" }.into(),
        ));
        let worker = spawn_worker(&command, &metrics, &inbox, 1)?;
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
            scroll_offsets: IdMap::default(),
            pressed: None,
            started: Instant::now(),
            draining: false,
            first_frame: true,
            quit_requested: false,
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
                progressed = true;
            }
            let inbound = self.inbox.take();
            for item in inbound {
                self.handle_inbound(item);
                progressed = true;
            }
            if !progressed {
                break;
            }
        }
        self.scene.flush(&mut self.backend, MAIN_WINDOW);
        if let Some(worker) = &mut self.worker {
            worker.flush();
        }
        self.draining = false;
        if self.finished && !self.quit_requested {
            self.quit_requested = true;
            self.worker = None;
            self.backend.quit();
        }
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
                eprintln!("[pam-desktop] PHP error: {message}");
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
            Inbound::ModuleResult { request, status, payload } => {
                if let Some(worker) = &mut self.worker {
                    worker.send_module_result(request, status, &payload);
                }
            }
            Inbound::Reload => self.reload(),
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
        self.pressed = None;
        match spawn_worker(&self.worker_command, &self.metrics, &self.inbox, self.generation) {
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
        if applied {
            self.first_frame = false;
        } else {
            self.resynchronize();
        }
    }

    /// Drops the mounted tree and asks PHP for a complete frame.
    fn resynchronize(&mut self) {
        self.scene.clear(&mut self.backend);
        self.engine = {
            let mut engine = Engine::new();
            let _ = engine.set_viewport(self.metrics.size.width, self.metrics.size.height);
            let _ = engine.set_text_scale(self.metrics.font_scale);
            let _ = engine.set_refresh_rate(f64::from(self.metrics.refresh_rate));
            // SAFETY: same contract as in `new`.
            unsafe {
                engine.set_text_measurer(Some((
                    measure_trampoline::<B>,
                    std::ptr::from_mut::<MeasureContext<B>>(self.measure.as_mut()).cast::<c_void>(),
                )));
            }
            engine
        };
        if let Some(worker) = &mut self.worker {
            worker.resynchronize();
        }
    }

    fn relayout(&mut self) {
        let (width, height, font_scale) = (self.metrics.size.width, self.metrics.size.height, self.metrics.font_scale);
        let result = self.with_engine(|engine, batch| engine.relayout_with_metrics_into(width, height, font_scale, batch));
        match result {
            Ok(()) => {
                self.apply_batch();
            }
            Err(error) => eprintln!("[pam-desktop] relayout failed: {error}"),
        }
    }

    fn send_event(&mut self, node: NodeId, kind: u16, payload: &[u8]) {
        if let Some(worker) = &mut self.worker {
            worker.send_event(node, kind, payload);
        }
    }

    fn send_dimensions(&mut self) {
        let metrics = self.metrics;
        let payload = WireMap::new()
            .with("width", metrics.size.width)
            .with("height", metrics.size.height)
            .with("density", metrics.scale)
            .with("appearance", if metrics.dark { 2_i64 } else { 1_i64 })
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
            BackendEvent::Pointer(pointer) => self.handle_pointer(pointer),
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
                    if let Some(worker) = &mut self.worker {
                        worker.shutdown();
                    }
                    self.finished = true;
                }
            }
            BackendEvent::WindowFocusChanged { window, focused } => {
                if window == MAIN_WINDOW {
                    let state = if focused { b"1" } else { b"2" };
                    self.send_event(0, event_kind::APP_STATE, state);
                }
            }
            BackendEvent::AppearanceChanged { dark } => {
                if self.metrics.dark != dark {
                    self.metrics.dark = dark;
                    self.send_dimensions();
                }
            }
            BackendEvent::Frame { .. } => {}
            BackendEvent::ScrollChanged { id, offset } => {
                self.scroll_offsets.insert(id, offset);
            }
            BackendEvent::ImageLoaded { .. } | BackendEvent::ImageFailed { .. } => {}
        }
    }

    // ---- pointer -------------------------------------------------------

    /// Deepest node under `point` (window coordinates), front to back.
    #[must_use]
    pub fn hit_test(&self, point: Point) -> Option<NodeId> {
        self.hit_test_node(self.scene.root(), point)
    }

    fn hit_test_node(&self, id: NodeId, point: Point) -> Option<NodeId> {
        let node = self.scene.node(id)?;
        if !node.layer.visible || node.props().integer(PropKey::PointerEvents) == Some(2) {
            return None;
        }
        let inside = node.frame.contains(point);
        let clips = node.decoration.clip || node.class == crate::backend::ViewClass::Scroll;
        if !inside && clips {
            return None;
        }
        // Scrolled content is laid out unscrolled: move the point into it.
        let child_point = match self.scroll_offsets.get(&id) {
            Some(offset) => point.offset(offset.x, offset.y),
            None => point,
        };
        let mut ordered: Vec<(f32, u32, NodeId)> = node
            .children
            .iter()
            .filter_map(|child| {
                let child_node = self.scene.node(*child)?;
                Some((child_node.props().number_or(PropKey::ZIndex, 0.0), child_node.index, *child))
            })
            .collect();
        ordered.sort_by(|left, right| right.0.total_cmp(&left.0).then(right.1.cmp(&left.1)));
        for (_, _, child) in ordered {
            if let Some(hit) = self.hit_test_node(child, child_point) {
                return Some(hit);
            }
        }
        inside.then_some(id)
    }

    /// Nearest node at or above `id` that reacts to presses.
    fn press_target(&self, id: NodeId) -> Option<NodeId> {
        let mut current = id;
        loop {
            let node = self.scene.node(current)?;
            let pressable = matches!(node.kind, NodeKind::Button | NodeKind::Pressable)
                || node.handles(PropKey::OnPress)
                || node.handles(PropKey::OnLongPress)
                || node.handles(PropKey::OnPressIn)
                || node.handles(PropKey::OnPressOut);
            if pressable {
                return node.props().flag_or(PropKey::Enabled, true).then_some(current);
            }
            if node.parent == 0 {
                return None;
            }
            current = node.parent;
        }
    }

    fn press_payload(&self, node: NodeId, position: Point, time_ms: u64) -> Vec<u8> {
        let origin = self.scene.node(node).map_or(Point::ZERO, |node| node.frame.origin());
        WireMap::new()
            .with("x", position.x - origin.x)
            .with("y", position.y - origin.y)
            .with("pageX", position.x)
            .with("pageY", position.y)
            .with("timestamp", i64::try_from(time_ms).unwrap_or(0))
            .with("pointerId", 0_i64)
            .encode()
    }

    fn handle_pointer(&mut self, pointer: PointerEvent) {
        if pointer.window != MAIN_WINDOW {
            return;
        }
        match pointer.phase {
            PointerPhase::Down if pointer.button == PointerButton::Primary => {
                let target = self.hit_test(pointer.position).and_then(|hit| self.press_target(hit));
                if let Some(node) = target {
                    self.pressed = Some(PressState { node, started: pointer.position });
                    if self.scene.node(node).is_some_and(|node| node.handles(PropKey::OnPressIn)) {
                        let payload = self.press_payload(node, pointer.position, pointer.time_ms);
                        self.send_event(node, event_kind::PRESS_IN, &payload);
                    }
                }
            }
            PointerPhase::Up if pointer.button == PointerButton::Primary => {
                if let Some(press) = self.pressed.take() {
                    let Some(node) = self.scene.node(press.node) else { return };
                    let handles_out = node.handles(PropKey::OnPressOut);
                    let handles_press = node.handles(PropKey::OnPress);
                    let payload = self.press_payload(press.node, pointer.position, pointer.time_ms);
                    if handles_out {
                        self.send_event(press.node, event_kind::PRESS_OUT, &payload);
                    }
                    let still_over = self
                        .hit_test(pointer.position)
                        .and_then(|hit| self.press_target(hit))
                        .is_some_and(|target| target == press.node);
                    if handles_press && still_over && press.started.distance(pointer.position) < 10_000.0 {
                        self.send_event(press.node, event_kind::PRESS, &payload);
                    }
                }
            }
            PointerPhase::Cancel | PointerPhase::Leave => {
                self.pressed = None;
            }
            _ => {}
        }
    }

    // ---- native modules ------------------------------------------------

    fn call_module(&mut self, request: u64, module: &str, method: &str, _payload: &[u8]) {
        // Unknown modules fail fast so application callbacks never hang.
        let message = format!("Native module {module}.{method} is not available on this platform.");
        if let Some(worker) = &mut self.worker {
            worker.send_module_result(request, MODULE_FAILURE, message.as_bytes());
        }
    }

    fn call_typed(&mut self, request: u64, operation: u16, _payload: &[u8]) {
        let message = format!("Native operation {operation} is not available on this platform.");
        if let Some(worker) = &mut self.worker {
            worker.send_module_result(request, MODULE_FAILURE, message.as_bytes());
        }
    }

    /// Milliseconds since the runtime started (diagnostics).
    #[must_use]
    pub fn uptime_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    #[must_use]
    pub fn platform(&self) -> &'static str {
        self.platform
    }
}

/// Starts a worker that sees the window exactly as it is now.
fn spawn_worker(
    command: &WorkerCommand,
    metrics: &WindowMetrics,
    inbox: &Arc<Inbox>,
    generation: u64,
) -> std::io::Result<Worker> {
    let mut command = command.clone();
    command.environment.push(("PAM_BOOT_METRICS".into(), boot_metrics_json(metrics)));
    command.environment.push(("PAM_SYSTEM_DARK".into(), if metrics.dark { "1" } else { "0" }.into()));
    command.environment.push(("PAM_SYSTEM_APPEARANCE".into(), if metrics.dark { "dark" } else { "light" }.into()));
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

/// Highest extension identifiers implemented (see `PropKey` / `EventKind`).
const EXTENSION_PROPERTY_CEILING: u16 = 1011;
const EXTENSION_EVENT_CEILING: u16 = 1000;

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
