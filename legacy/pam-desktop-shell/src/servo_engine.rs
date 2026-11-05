use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::Arc;

use euclid::Scale;
use pam_desktop_protocol::{
    Bootstrap, Effect, EffectKind, MAIN_WINDOW_ID, RenderBackend, TaskbarProgressState,
    WindowConfig, WindowRole, WindowTheme, WorkstationConfig,
};
use servo::{
    Code, DeviceIntRect, DeviceIntSize, DevicePoint, EventLoopWaker, InputEvent, Key, KeyState,
    KeyboardEvent, Location, Modifiers, MouseButton as ServoMouseButton, MouseButtonAction,
    MouseButtonEvent, MouseLeftViewportEvent, MouseMoveEvent, NamedKey, NavigationRequest,
    Preferences, RenderingContext, Servo, ServoBuilder, SoftwareRenderingContext, WebView,
    WebViewBuilder, WebViewDelegate, WheelDelta, WheelEvent, WheelMode, WindowRenderingContext,
};
use tracing::warn;
use url::Url;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{
    Key as WinitKey, KeyCode, ModifiersState, NamedKey as WinitNamedKey, PhysicalKey,
};
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawWindowHandle};
use winit::window::{Fullscreen, Theme, UserAttentionType, Window, WindowId, WindowLevel};

use crate::dev_event::{self, EventCode};
use crate::gateway::Gateway;
use crate::host_event::HostEvent;
use crate::lifecycle::InstanceGuard;
use crate::native::show_dialog;
use crate::native_shell::NativeShell;
use crate::runtime::DesktopRuntime;
use crate::window_state::{MonitorGeometry, WindowStateStore};

pub fn run(
    runtime: DesktopRuntime,
    watch: bool,
    mut instance: InstanceGuard,
    initial_arguments: Vec<String>,
) -> Result<(), String> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let cached_bootstrap = runtime.bootstrap().clone();
    let crash_reports_installed = cached_bootstrap.workstation.crash_reports;
    if crash_reports_installed {
        crate::crash_report::install(&cached_bootstrap.manifest.identifier)?;
    }
    let event_loop = EventLoop::with_user_event()
        .build()
        .map_err(|error| format!("cannot create desktop event loop: {error}"))?;
    NativeShell::install_event_handlers(&event_loop.create_proxy());
    let (project, supervisor, bootstrap, startup_snapshot_hit) = runtime.into_parts()?;
    if bootstrap.workstation.crash_reports && !crash_reports_installed {
        crate::crash_report::install(&bootstrap.manifest.identifier)?;
    }
    let project_root = project.root().to_path_buf();
    if watch {
        dev_event::emit(
            EventCode::SessionStarting,
            &project_root,
            &serde_json::json!({}),
        );
    }
    let gateway = Gateway::start(
        &project,
        supervisor,
        bootstrap.clone(),
        event_loop.create_proxy(),
        watch,
        startup_snapshot_hit,
    )?;
    if watch {
        dev_event::emit(
            EventCode::SessionReady,
            &project_root,
            &serde_json::json!({
                "gatewayUrl": gateway.url(),
                "startupSnapshotHit": startup_snapshot_hit,
            }),
        );
    }
    let events = gateway.event_hub();
    let quick_actions = bootstrap
        .shell
        .quick_actions
        .iter()
        .map(|action| action.id.clone())
        .collect::<std::collections::HashSet<_>>();
    publish_activation(
        &events,
        &quick_actions,
        initial_arguments,
        "pam.lifecycle.opened",
    );
    instance.listen(move |activation| {
        publish_activation(
            &events,
            &quick_actions,
            activation.arguments,
            "pam.lifecycle.second-instance",
        );
    })?;
    let mut application = Application::new(&event_loop, bootstrap, gateway);

    let result = event_loop
        .run_app(&mut application)
        .map_err(|error| format!("desktop event loop failed: {error}"));
    if watch {
        dev_event::emit(
            EventCode::SessionStopped,
            &project_root,
            &serde_json::json!({}),
        );
    }
    result?;
    drop(instance);
    Ok(())
}

fn publish_activation(
    events: &crate::event_hub::EventHub,
    allowed_quick_actions: &std::collections::HashSet<String>,
    arguments: Vec<String>,
    lifecycle_event: &str,
) {
    let (quick_actions, remaining) = split_activation(allowed_quick_actions, arguments);
    for id in quick_actions {
        events.publish(pam_desktop_protocol::ClientEvent {
            name: "pam.quick-action.selected".to_owned(),
            payload: serde_json::json!({"id": id}),
            window_id: None,
        });
    }
    if !remaining.is_empty() {
        events.publish(pam_desktop_protocol::ClientEvent {
            name: lifecycle_event.to_owned(),
            payload: serde_json::json!({"arguments": remaining}),
            window_id: None,
        });
    }
}

fn split_activation(
    allowed_quick_actions: &std::collections::HashSet<String>,
    arguments: Vec<String>,
) -> (Vec<String>, Vec<String>) {
    let mut quick_actions = Vec::new();
    let mut remaining = Vec::new();
    for argument in arguments {
        if let Some(id) = argument.strip_prefix("--pam-quick-action=") {
            if allowed_quick_actions.contains(id) {
                quick_actions.push(id.to_owned());
            }
        } else {
            remaining.push(argument);
        }
    }
    (quick_actions, remaining)
}

struct DesktopWindow {
    id: String,
    role: WindowRole,
    parent: Option<String>,
    webview: WebView,
    rendering_context: Rc<dyn RenderingContext>,
    software_presenter: Option<RefCell<SoftwarePresenter>>,
    window: Arc<Window>,
    accessibility: RefCell<accesskit_winit::Adapter>,
    mouse_position: Cell<DevicePoint>,
    modifiers: Cell<ModifiersState>,
}

struct SoftwarePresenter {
    context: Rc<SoftwareRenderingContext>,
    surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
    previous: Vec<u32>,
    dirty_regions: bool,
}

impl SoftwarePresenter {
    fn new(
        window: Arc<Window>,
        context: Rc<SoftwareRenderingContext>,
        dirty_regions: bool,
    ) -> Result<Self, String> {
        let display = softbuffer::Context::new(window.clone())
            .map_err(|error| format!("cannot create software display context: {error}"))?;
        let surface = softbuffer::Surface::new(&display, window)
            .map_err(|error| format!("cannot create software window surface: {error}"))?;
        Ok(Self {
            context,
            surface,
            previous: Vec::new(),
            dirty_regions,
        })
    }

    fn present(&mut self) -> Result<(), String> {
        let size = self.context.size();
        let width =
            NonZeroU32::new(size.width).ok_or_else(|| "software frame width is zero".to_owned())?;
        let height = NonZeroU32::new(size.height)
            .ok_or_else(|| "software frame height is zero".to_owned())?;
        let rectangle = DeviceIntRect::from_size(DeviceIntSize::new(
            i32::try_from(size.width).map_err(|_| "software frame is too wide".to_owned())?,
            i32::try_from(size.height).map_err(|_| "software frame is too tall".to_owned())?,
        ));
        let image = self
            .context
            .read_to_image(rectangle)
            .ok_or_else(|| "Servo did not expose the software frame".to_owned())?;
        let pixels = image
            .as_raw()
            .chunks_exact(4)
            .map(|rgba| (u32::from(rgba[0]) << 16) | (u32::from(rgba[1]) << 8) | u32::from(rgba[2]))
            .collect::<Vec<_>>();
        let damage = self
            .dirty_regions
            .then(|| changed_bounds(&self.previous, &pixels, size.width, size.height))
            .flatten();
        self.surface
            .resize(width, height)
            .map_err(|error| format!("cannot resize software window surface: {error}"))?;
        let mut buffer = self
            .surface
            .buffer_mut()
            .map_err(|error| format!("cannot map software window buffer: {error}"))?;
        buffer.copy_from_slice(&pixels);
        self.previous = pixels;
        if let Some((x, y, damage_width, damage_height)) = damage {
            buffer
                .present_with_damage(&[softbuffer::Rect {
                    x,
                    y,
                    width: NonZeroU32::new(damage_width).expect("damage width is nonzero"),
                    height: NonZeroU32::new(damage_height).expect("damage height is nonzero"),
                }])
                .map_err(|error| format!("cannot present dirty software frame: {error}"))
        } else {
            buffer
                .present()
                .map_err(|error| format!("cannot present software frame: {error}"))
        }
    }
}

fn changed_bounds(
    previous: &[u32],
    current: &[u32],
    width: u32,
    height: u32,
) -> Option<(u32, u32, u32, u32)> {
    if previous.len() != current.len() || previous.is_empty() {
        return None;
    }
    let width_usize = width as usize;
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0;
    let mut max_y = 0;
    for (index, (before, after)) in previous.iter().zip(current).enumerate() {
        if before == after {
            continue;
        }
        let x = (index % width_usize) as u32;
        let y = (index / width_usize) as u32;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    (min_x <= max_x).then_some((min_x, min_y, max_x - min_x + 1, max_y - min_y + 1))
}

impl DesktopWindow {
    fn handle_mouse_button(&self, button: MouseButton, state: ElementState) {
        let button = match button {
            MouseButton::Left => ServoMouseButton::Left,
            MouseButton::Right => ServoMouseButton::Right,
            MouseButton::Middle => ServoMouseButton::Middle,
            MouseButton::Back => ServoMouseButton::Back,
            MouseButton::Forward => ServoMouseButton::Forward,
            MouseButton::Other(value) => ServoMouseButton::Other(value),
        };
        let action = match state {
            ElementState::Pressed => MouseButtonAction::Down,
            ElementState::Released => MouseButtonAction::Up,
        };
        self.webview
            .notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
                action,
                button,
                self.mouse_position.get().into(),
            )));
    }

    fn handle_mouse_move(&self, position: PhysicalPosition<f64>) {
        let point = DevicePoint::new(engine_float(position.x), engine_float(position.y));
        self.mouse_position.set(point);
        self.webview
            .notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(point.into())));
    }

    fn handle_wheel(&self, delta: MouseScrollDelta) {
        let (x, y) = match delta {
            MouseScrollDelta::LineDelta(x, y) => (f64::from(x) * 76.0, f64::from(y) * 76.0),
            MouseScrollDelta::PixelDelta(delta) => (delta.x, delta.y),
        };
        self.webview
            .notify_input_event(InputEvent::Wheel(WheelEvent::new(
                WheelDelta {
                    x,
                    y,
                    z: 0.0,
                    mode: WheelMode::DeltaPixel,
                },
                self.mouse_position.get().into(),
            )));
    }
}

struct AppState {
    servo: Servo,
    windows: RefCell<HashMap<WindowId, DesktopWindow>>,
    allowed_origin: String,
    gateway: Gateway,
    native_shell: RefCell<NativeShell>,
    event_proxy: winit::event_loop::EventLoopProxy<HostEvent>,
    workstation: RefCell<WorkstationConfig>,
    application_id: String,
    window_states: WindowStateStore,
}

impl AppState {
    fn create_rendering_context(
        &self,
        event_loop: &ActiveEventLoop,
        window: Arc<Window>,
        window_id: &str,
    ) -> Result<(Rc<dyn RenderingContext>, Option<RefCell<SoftwarePresenter>>), String> {
        let workstation = self.workstation.borrow();
        let backend = workstation.render_backend;
        let dirty_regions = workstation.dirty_regions;
        drop(workstation);
        if backend != RenderBackend::Software {
            let gpu = (|| {
                let display_handle = event_loop.display_handle().map_err(|error| {
                    format!("desktop event loop has no display handle: {error}")
                })?;
                let window_handle = window.window_handle().map_err(|error| {
                    format!("window {window_id:?} has no native handle: {error}")
                })?;
                WindowRenderingContext::new(display_handle, window_handle, window.inner_size())
                    .map(Rc::new)
                    .map_err(|error| format!("GPU context creation failed: {error:?}"))
            })();
            match gpu {
                Ok(context) => {
                    let rendering_context: Rc<dyn RenderingContext> = context;
                    return Ok((rendering_context, None));
                }
                Err(error) if backend == RenderBackend::Gpu => {
                    return Err(format!(
                        "Servo cannot create the required GPU context for {window_id:?}: {error}"
                    ));
                }
                Err(error) => warn!(
                    window = window_id,
                    %error,
                    "GPU initialization failed; activating certified software fallback"
                ),
            }
        }
        let context = Rc::new(SoftwareRenderingContext::new(window.inner_size()).map_err(
            |error| {
                format!("cannot create software rendering context for {window_id:?}: {error:?}")
            },
        )?);
        let presenter = SoftwarePresenter::new(window, context.clone(), dirty_regions)?;
        let rendering_context: Rc<dyn RenderingContext> = context;
        Ok((rendering_context, Some(RefCell::new(presenter))))
    }

    fn configure(
        self: &Rc<Self>,
        event_loop: &ActiveEventLoop,
        bootstrap: &Bootstrap,
    ) -> Result<(), String> {
        // Surfman creates an EGL connection for every window rendering context.
        // On some GLVND/NVIDIA stacks, creating replacement contexts before
        // dropping the old ones lets the old connection's eglTerminate invalidate
        // the newly-created contexts because both resolve to the same EGLDisplay.
        // Fully release the previous generation before initializing the next one.
        let previous = self.windows.replace(HashMap::new());
        drop(previous);

        self.windows.borrow_mut().reserve(bootstrap.windows.len());
        let mut pending = bootstrap.windows.iter().collect::<Vec<_>>();
        while !pending.is_empty() {
            let before = pending.len();
            let mut failure = None;
            pending.retain(|config| {
                let parent = self
                    .workstation
                    .borrow()
                    .windows
                    .get(&config.id)
                    .and_then(|profile| profile.parent.as_deref())
                    .map(str::to_owned);
                if parent.is_some_and(|parent| {
                    !self
                        .windows
                        .borrow()
                        .values()
                        .any(|window| window.id == parent)
                }) {
                    return true;
                }
                match self.create_window(event_loop, config) {
                    Ok(desktop_window) => {
                        self.windows
                            .borrow_mut()
                            .insert(desktop_window.window.id(), desktop_window);
                        false
                    }
                    Err(error) => {
                        failure = Some(error);
                        true
                    }
                }
            });
            if let Some(error) = failure {
                return Err(error);
            }
            if pending.len() == before {
                return Err(format!(
                    "cannot resolve native parent ordering for windows: {}",
                    pending
                        .iter()
                        .map(|window| window.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                ));
            }
        }
        Ok(())
    }

    fn create_window(
        self: &Rc<Self>,
        event_loop: &ActiveEventLoop,
        config: &WindowConfig,
    ) -> Result<DesktopWindow, String> {
        let workstation = self.workstation.borrow();
        let profile = workstation.windows.get(&config.id);
        let monitors = monitor_geometries(event_loop);
        let restored = profile
            .filter(|profile| workstation.workspace_restore && profile.restore)
            .and_then(|profile| {
                self.window_states
                    .restore(&config.id, &monitors, profile.remember_monitor)
            });
        let role = profile.map_or(WindowRole::Primary, |profile| profile.role);
        let elevated = config.always_on_top
            || matches!(
                role,
                WindowRole::Popover | WindowRole::Panel | WindowRole::Palette
            );
        let chrome = config.decorated && !matches!(role, WindowRole::Popover | WindowRole::Panel);
        let mut attributes = Window::default_attributes()
            .with_title(config.title.clone())
            .with_inner_size(LogicalSize::new(config.width, config.height))
            .with_min_inner_size(LogicalSize::new(config.min_width, config.min_height))
            .with_resizable(config.resizable && role != WindowRole::Popover)
            // AccessKit must attach before a native window is ever visible.
            .with_visible(false)
            .with_theme(window_theme(config.theme))
            .with_decorations(chrome)
            .with_transparent(config.transparent)
            .with_window_level(if elevated {
                WindowLevel::AlwaysOnTop
            } else {
                WindowLevel::Normal
            })
            .with_maximized(
                restored
                    .as_ref()
                    .map_or(config.maximized, |state| state.maximized),
            )
            .with_fullscreen(
                restored
                    .as_ref()
                    .map_or(config.fullscreen, |state| state.fullscreen)
                    .then(|| Fullscreen::Borderless(None)),
            );
        let parent = profile.and_then(|profile| profile.parent.clone());
        if let Some(state) = restored {
            attributes = attributes
                .with_inner_size(PhysicalSize::new(state.width, state.height))
                .with_position(PhysicalPosition::new(state.x, state.y));
        }
        drop(workstation);
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(|error| format!("cannot create window {:?}: {error}", config.id))?,
        );
        let accessibility = accesskit_winit::Adapter::with_event_loop_proxy(
            event_loop,
            &window,
            self.event_proxy.clone(),
        );
        let (rendering_context, software_presenter) =
            self.create_rendering_context(event_loop, window.clone(), &config.id)?;
        rendering_context.make_current().map_err(|error| {
            format!(
                "Servo cannot activate the rendering context for {:?}: {error:?}",
                config.id,
            )
        })?;
        let url = Url::parse(&self.gateway.window_url(&config.id)?)
            .map_err(|error| format!("cannot create URL for window {:?}: {error}", config.id))?;
        let webview = WebViewBuilder::new(&self.servo, rendering_context.clone())
            .url(url)
            .hidpi_scale_factor(Scale::new(engine_float(window.scale_factor())))
            .delegate(self.clone())
            .build();
        if config.visible {
            webview.show();
            window.set_visible(true);
        } else {
            webview.hide();
        }
        if config.id == MAIN_WINDOW_ID {
            webview.focus();
        }

        Ok(DesktopWindow {
            id: config.id.clone(),
            role,
            parent,
            webview,
            rendering_context,
            software_presenter,
            window,
            accessibility: RefCell::new(accessibility),
            mouse_position: Cell::new(DevicePoint::default()),
            modifiers: Cell::new(ModifiersState::empty()),
        })
    }

    fn window_id_for_webview(&self, webview: &WebView) -> Option<WindowId> {
        self.windows
            .borrow()
            .iter()
            .find_map(|(window_id, window)| (&window.webview == webview).then_some(*window_id))
    }

    fn with_webview_window(&self, webview: &WebView, operation: impl FnOnce(&DesktopWindow)) {
        let Some(window_id) = self.window_id_for_webview(webview) else {
            return;
        };
        if let Some(window) = self.windows.borrow().get(&window_id) {
            operation(window);
        }
    }

    fn target_window_id(&self, application_id: &str) -> Option<WindowId> {
        self.windows
            .borrow()
            .iter()
            .find_map(|(window_id, window)| (window.id == application_id).then_some(*window_id))
    }

    fn apply_effects(&self, event_loop: &ActiveEventLoop, effects: Vec<Effect>) {
        for effect in effects {
            match self.native_shell.borrow_mut().apply_effect(&effect) {
                Ok(true) => continue,
                Ok(false) => {}
                Err(error) => {
                    warn!(?error, "cannot apply native shell effect");
                    continue;
                }
            }
            if effect.kind == EffectKind::QuitApplication {
                if self.workstation.borrow().background_agent
                    && let Err(error) = crate::background_agent::stop(&self.application_id)
                {
                    warn!(?error, "cannot stop desktop background agent");
                }
                event_loop.exit();
                return;
            }
            let Some(window_id) = self.target_window_id(&effect.window_id) else {
                warn!(
                    window = effect.window_id,
                    "ignored effect for unknown window"
                );
                continue;
            };
            match effect.kind {
                EffectKind::SetWindowTitle => {
                    if let Some(title) = effect
                        .payload
                        .get("title")
                        .and_then(serde_json::Value::as_str)
                        && let Some(window) = self.windows.borrow().get(&window_id)
                    {
                        window.window.set_title(title);
                    }
                }
                EffectKind::SetWindowVisible => {
                    if let Some(visible) = effect
                        .payload
                        .get("visible")
                        .and_then(serde_json::Value::as_bool)
                        && let Some(window) = self.windows.borrow().get(&window_id)
                    {
                        window.window.set_visible(visible);
                        if visible {
                            window.webview.show();
                        } else {
                            window.webview.hide();
                        }
                    }
                }
                EffectKind::CloseWindow => {
                    if (effect.window_id == MAIN_WINDOW_ID || self.windows.borrow().len() == 1)
                        && self.keep_alive_without_windows()
                    {
                        if let Some(window) = self.windows.borrow().get(&window_id) {
                            window.window.set_visible(false);
                            window.webview.hide();
                        }
                    } else if effect.window_id == MAIN_WINDOW_ID || self.windows.borrow().len() == 1
                    {
                        event_loop.exit();
                    } else {
                        self.windows.borrow_mut().remove(&window_id);
                    }
                }
                EffectKind::FocusWindow => {
                    if let Some(window) = self.windows.borrow().get(&window_id) {
                        window.window.set_visible(true);
                        window.webview.show();
                        window.webview.focus();
                        window.window.focus_window();
                    }
                }
                EffectKind::SetWindowFullscreen => {
                    if let Some(fullscreen) = effect
                        .payload
                        .get("fullscreen")
                        .and_then(serde_json::Value::as_bool)
                        && let Some(window) = self.windows.borrow().get(&window_id)
                    {
                        window
                            .window
                            .set_fullscreen(fullscreen.then(|| Fullscreen::Borderless(None)));
                    }
                }
                EffectKind::SetWindowMaximized => {
                    if let Some(maximized) = effect
                        .payload
                        .get("maximized")
                        .and_then(serde_json::Value::as_bool)
                        && let Some(window) = self.windows.borrow().get(&window_id)
                    {
                        window.window.set_maximized(maximized);
                    }
                }
                EffectKind::SetWindowAlwaysOnTop => {
                    if let Some(always_on_top) = effect
                        .payload
                        .get("alwaysOnTop")
                        .and_then(serde_json::Value::as_bool)
                        && let Some(window) = self.windows.borrow().get(&window_id)
                    {
                        window.window.set_window_level(if always_on_top {
                            WindowLevel::AlwaysOnTop
                        } else {
                            WindowLevel::Normal
                        });
                    }
                }
                EffectKind::SetWindowAttention => {
                    let active = effect
                        .payload
                        .get("active")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(true);
                    let critical = effect
                        .payload
                        .get("critical")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    if let Some(window) = self.windows.borrow().get(&window_id) {
                        window
                            .window
                            .request_user_attention(active.then_some(if critical {
                                UserAttentionType::Critical
                            } else {
                                UserAttentionType::Informational
                            }));
                    }
                }
                EffectKind::SetApplicationBadge => {
                    let visible = effect
                        .payload
                        .get("visible")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    let count = effect
                        .payload
                        .get("count")
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|count| u32::try_from(count).ok());
                    if let Some(window) = self.windows.borrow().get(&window_id)
                        && let Err(error) = pam_desktop_platform::set_badge(
                            Self::platform_window_handle(&window.window),
                            visible.then_some(count.unwrap_or(0)),
                        )
                    {
                        warn!(?error, "cannot apply application badge");
                    }
                }
                EffectKind::SetTaskbarProgress => {
                    let progress = effect
                        .payload
                        .get("progress")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let state = effect
                        .payload
                        .get("state")
                        .cloned()
                        .and_then(|value| {
                            serde_json::from_value::<TaskbarProgressState>(value).ok()
                        })
                        .unwrap_or_default();
                    if let Some(window) = self.windows.borrow().get(&window_id)
                        && let Ok(state) =
                            pam_desktop_platform::ProgressState::try_from(state as u8)
                        && let Err(error) = pam_desktop_platform::set_progress(
                            Self::platform_window_handle(&window.window),
                            progress,
                            state,
                        )
                    {
                        warn!(?error, "cannot apply taskbar progress");
                    }
                }
                EffectKind::SetMenuItemEnabled
                | EffectKind::SetMenuItemChecked
                | EffectKind::SetTrayVisible
                | EffectKind::QuitApplication => {
                    unreachable!("native shell effects are handled before window effects")
                }
            }
        }
    }

    fn platform_window_handle(window: &Window) -> isize {
        let Ok(handle) = window.window_handle() else {
            return 0;
        };
        match handle.as_raw() {
            RawWindowHandle::Win32(handle) => handle.hwnd.get(),
            RawWindowHandle::AppKit(_) => 1,
            _ => 0,
        }
    }

    fn reload_views(&self) {
        for window in self.windows.borrow().values() {
            window.webview.reload();
        }
    }

    fn close_requested(&self, event_loop: &ActiveEventLoop, window_id: WindowId) {
        if self
            .windows
            .borrow()
            .get(&window_id)
            .is_some_and(|window| window.id == MAIN_WINDOW_ID)
            && (self.native_shell.borrow().close_behavior()
                == pam_desktop_protocol::TrayCloseBehavior::Hide
                || self.keep_alive_without_windows())
        {
            if let Some(window) = self.windows.borrow().get(&window_id) {
                window.window.set_visible(false);
                window.webview.hide();
            }
            return;
        }
        let close_application =
            self.windows.borrow().get(&window_id).is_none_or(|window| {
                window.id == MAIN_WINDOW_ID || self.windows.borrow().len() == 1
            });
        if close_application {
            event_loop.exit();
        } else {
            // Keep Servo's WebView and EGL/surfman context alive for the
            // process lifetime. Dropping and recreating a secondary window can
            // invalidate the shared EGL display on GLVND/NVIDIA stacks.
            if let Some(window) = self.windows.borrow().get(&window_id) {
                window.webview.blur();
                window.webview.hide();
                window.window.set_visible(false);
            }
        }
    }

    fn keep_alive_without_windows(&self) -> bool {
        let profile = self.workstation.borrow();
        profile.persistent_services && !profile.background_agent
    }

    fn focus_modal_child(&self, parent_id: &str) -> bool {
        let windows = self.windows.borrow();
        let Some(modal) = windows.values().find(|window| {
            window.role == WindowRole::Modal
                && window.parent.as_deref() == Some(parent_id)
                && window.window.is_visible() == Some(true)
        }) else {
            return false;
        };
        modal.window.focus_window();
        modal.webview.focus();
        true
    }

    fn record_window_state(&self, window: &DesktopWindow) {
        let workstation = self.workstation.borrow();
        let Some(profile) = workstation.windows.get(&window.id) else {
            return;
        };
        if !workstation.workspace_restore || !profile.restore {
            return;
        }
        let Some(monitor) = window.window.current_monitor() else {
            return;
        };
        let Ok(position) = window.window.outer_position() else {
            return;
        };
        let Some(geometry) = monitor_geometry(&monitor) else {
            return;
        };
        let size = window.window.inner_size();
        if let Err(error) = self.window_states.record(
            &window.id,
            &geometry,
            position.x,
            position.y,
            size.width,
            size.height,
            window.window.is_maximized(),
            window.window.fullscreen().is_some(),
        ) {
            warn!(
                window = window.id,
                ?error,
                "cannot persist window restoration state"
            );
        }
    }
}

impl WebViewDelegate for AppState {
    fn notify_new_frame_ready(&self, webview: WebView) {
        self.with_webview_window(&webview, |window| window.window.request_redraw());
    }

    fn notify_animating_changed(&self, webview: WebView, _animating: bool) {
        self.with_webview_window(&webview, |window| window.window.request_redraw());
    }

    fn request_navigation(&self, _webview: WebView, request: NavigationRequest) {
        if origin(&request.url) == self.allowed_origin {
            request.allow();
        } else {
            request.deny();
        }
    }

    fn notify_accessibility_tree_update(
        &self,
        webview: WebView,
        tree_update: servo::accesskit::TreeUpdate,
    ) {
        self.with_webview_window(&webview, |window| {
            window
                .accessibility
                .borrow_mut()
                .update_if_active(|| tree_update);
        });
    }
}

enum Application {
    Initial(Box<InitialState>),
    Running(Rc<AppState>),
}

struct InitialState {
    waker: Waker,
    bootstrap: Bootstrap,
    gateway: Option<Gateway>,
}

impl Application {
    fn new(event_loop: &EventLoop<HostEvent>, bootstrap: Bootstrap, gateway: Gateway) -> Self {
        Self::Initial(Box::new(InitialState {
            waker: Waker(event_loop.create_proxy()),
            bootstrap,
            gateway: Some(gateway),
        }))
    }
}

impl ApplicationHandler<HostEvent> for Application {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let Self::Initial(initial) = self else {
            return;
        };
        let mut preferences = Preferences::default();
        preferences.accessibility_enabled = true;
        let servo = ServoBuilder::default()
            .preferences(preferences)
            .event_loop_waker(Box::new(initial.waker.clone()))
            .build();
        servo.setup_logging();
        let gateway = initial
            .gateway
            .take()
            .expect("desktop gateway should only be consumed once");
        let allowed_origin = gateway.url().trim_end_matches('/').to_owned();
        let event_proxy = initial.waker.0.clone();
        let native_shell = match NativeShell::prepare(&initial.bootstrap, event_proxy.clone()) {
            Ok(shell) => shell,
            Err(error) => {
                eprintln!("pam-desktop: cannot configure native shell: {error}");
                event_loop.exit();
                return;
            }
        };
        let window_states = match WindowStateStore::open(&initial.bootstrap.manifest.identifier) {
            Ok(store) => store,
            Err(error) => {
                eprintln!("pam-desktop: {error}");
                event_loop.exit();
                return;
            }
        };
        let state = Rc::new(AppState {
            servo,
            windows: RefCell::new(HashMap::new()),
            allowed_origin,
            gateway,
            native_shell: RefCell::new(native_shell),
            event_proxy,
            workstation: RefCell::new(initial.bootstrap.workstation.clone()),
            application_id: initial.bootstrap.manifest.identifier.clone(),
            window_states,
        });
        if let Err(error) = state.configure(event_loop, &initial.bootstrap) {
            eprintln!("pam-desktop: {error}");
            event_loop.exit();
            return;
        }
        *self = Self::Running(state);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: HostEvent) {
        let Self::Running(state) = self else {
            return;
        };
        match event {
            HostEvent::ServoWake => state.servo.spin_event_loop(),
            HostEvent::Accessibility(event) => {
                let windows = state.windows.borrow();
                if let Some(window) = windows.get(&event.window_id) {
                    match event.window_event {
                        accesskit_winit::WindowEvent::InitialTreeRequested => {
                            let _ = window.webview.set_accessibility_active(true);
                        }
                        accesskit_winit::WindowEvent::AccessibilityDeactivated => {
                            let _ = window.webview.set_accessibility_active(false);
                        }
                        accesskit_winit::WindowEvent::ActionRequested(request) => {
                            warn!(
                                ?request,
                                "Servo 0.5 does not expose accessibility action forwarding"
                            );
                        }
                    }
                }
            }
            HostEvent::ApplyEffects(effects) => state.apply_effects(event_loop, effects),
            HostEvent::ReloadViews => state.reload_views(),
            HostEvent::Reconfigure(bootstrap) => {
                state.workstation.replace(bootstrap.workstation.clone());
                state.native_shell.replace(NativeShell::empty());
                match NativeShell::prepare(&bootstrap, state.event_proxy.clone()) {
                    Ok(shell) => {
                        state.native_shell.replace(shell);
                    }
                    Err(error) => {
                        eprintln!("pam-desktop: cannot reload native shell: {error}");
                    }
                }
                if let Err(error) = state.configure(event_loop, &bootstrap) {
                    eprintln!("pam-desktop: cannot apply hot reload: {error}");
                }
            }
            HostEvent::Dialog(request) => show_dialog(request),
            HostEvent::Shell(event) => {
                if let Some((name, payload)) = state.native_shell.borrow().dispatch(event) {
                    state.gateway.dispatch_native_event(name, payload);
                }
            }
            HostEvent::Exit => {
                if state.workstation.borrow().background_agent
                    && let Err(error) = crate::background_agent::stop(&state.application_id)
                {
                    warn!(?error, "cannot stop desktop background agent");
                }
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Self::Running(state) = self else {
            return;
        };
        state.servo.spin_event_loop();

        if matches!(event, WindowEvent::CloseRequested) {
            if let Some(window) = state.windows.borrow().get(&window_id) {
                state.record_window_state(window);
            }
            state.close_requested(event_loop, window_id);
            return;
        }
        let windows = state.windows.borrow();
        let Some(window) = windows.get(&window_id) else {
            return;
        };
        window
            .accessibility
            .borrow_mut()
            .process_event(&window.window, &event);
        match event {
            WindowEvent::RedrawRequested => {
                window.webview.paint();
                if let Some(presenter) = &window.software_presenter
                    && let Err(error) = presenter.borrow_mut().present()
                {
                    warn!(window = window.id, %error, "cannot present software-rendered frame");
                }
                window.rendering_context.present();
            }
            WindowEvent::Moved(_) => state.record_window_state(window),
            WindowEvent::Resized(size) => {
                window.webview.resize(size);
                state.record_window_state(window);
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                window
                    .webview
                    .set_hidpi_scale_factor(Scale::new(engine_float(scale_factor)));
                window.webview.resize(window.window.inner_size());
                state.record_window_state(window);
            }
            WindowEvent::Focused(true) => {
                if !state.focus_modal_child(&window.id) {
                    window.webview.focus();
                }
            }
            WindowEvent::Focused(false) => window.webview.blur(),
            WindowEvent::CursorMoved { position, .. } => window.handle_mouse_move(position),
            WindowEvent::CursorLeft { .. } => {
                window
                    .webview
                    .notify_input_event(InputEvent::MouseLeftViewport(
                        MouseLeftViewportEvent::default(),
                    ));
            }
            WindowEvent::MouseInput {
                state: button_state,
                button,
                ..
            } => window.handle_mouse_button(button, button_state),
            WindowEvent::MouseWheel { delta, .. } => window.handle_wheel(delta),
            WindowEvent::ModifiersChanged(modifiers) => window.modifiers.set(modifiers.state()),
            WindowEvent::KeyboardInput { event, .. } => {
                window
                    .webview
                    .notify_input_event(InputEvent::Keyboard(keyboard_event(
                        &event,
                        window.modifiers.get(),
                    )));
            }
            WindowEvent::HoveredFile(path) => state.gateway.drag_hover(&window.id, &path),
            WindowEvent::DroppedFile(path) => state.gateway.drag_drop(&window.id, &path),
            WindowEvent::HoveredFileCancelled => state.gateway.drag_leave(&window.id),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Self::Running(state) = self else {
            return;
        };
        let windows = state.windows.borrow();
        let animating = windows.values().any(|window| window.webview.animating());
        if animating {
            state.servo.spin_event_loop();
            for window in windows.values().filter(|window| window.webview.animating()) {
                window.window.request_redraw();
            }
            event_loop.set_control_flow(ControlFlow::Poll);
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separates_declared_quick_actions_from_normal_activation_arguments() {
        let allowed = std::collections::HashSet::from(["compose".to_owned()]);
        let (actions, remaining) = split_activation(
            &allowed,
            vec![
                "--pam-quick-action=compose".to_owned(),
                "--pam-quick-action=forged".to_owned(),
                "pam://open/item".to_owned(),
            ],
        );
        assert_eq!(actions, ["compose"]);
        assert_eq!(remaining, ["pam://open/item"]);
    }
}

#[derive(Clone)]
struct Waker(winit::event_loop::EventLoopProxy<HostEvent>);

impl EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        if let Err(error) = self.0.send_event(HostEvent::ServoWake) {
            warn!(?error, "failed to wake Pam Desktop event loop");
        }
    }
}

fn keyboard_event(event: &winit::event::KeyEvent, modifiers: ModifiersState) -> KeyboardEvent {
    let key = match &event.logical_key {
        WinitKey::Character(character) => Key::Character(character.to_string()),
        WinitKey::Named(named) => Key::Named(named_key(*named)),
        WinitKey::Unidentified(_) | WinitKey::Dead(_) => Key::Named(NamedKey::Unidentified),
    };
    let code = match event.physical_key {
        PhysicalKey::Code(KeyCode::Tab) => Code::Tab,
        PhysicalKey::Code(KeyCode::Enter | KeyCode::NumpadEnter) => Code::Enter,
        PhysicalKey::Code(KeyCode::Space) => Code::Space,
        PhysicalKey::Code(_) | PhysicalKey::Unidentified(_) => Code::Unidentified,
    };
    let state = match event.state {
        ElementState::Pressed => KeyState::Down,
        ElementState::Released => KeyState::Up,
    };
    let mut servo_modifiers = Modifiers::empty();
    servo_modifiers.set(Modifiers::CONTROL, modifiers.control_key());
    servo_modifiers.set(Modifiers::SHIFT, modifiers.shift_key());
    servo_modifiers.set(Modifiers::ALT, modifiers.alt_key());
    servo_modifiers.set(Modifiers::META, modifiers.super_key());

    KeyboardEvent::new_without_event(
        state,
        key,
        code,
        Location::Standard,
        servo_modifiers,
        event.repeat,
        false,
    )
}

fn named_key(key: WinitNamedKey) -> NamedKey {
    match key {
        WinitNamedKey::Tab => NamedKey::Tab,
        WinitNamedKey::Enter => NamedKey::Enter,
        WinitNamedKey::Escape => NamedKey::Escape,
        WinitNamedKey::Backspace => NamedKey::Backspace,
        WinitNamedKey::Delete => NamedKey::Delete,
        WinitNamedKey::Home => NamedKey::Home,
        WinitNamedKey::End => NamedKey::End,
        WinitNamedKey::PageUp => NamedKey::PageUp,
        WinitNamedKey::PageDown => NamedKey::PageDown,
        WinitNamedKey::ArrowUp => NamedKey::ArrowUp,
        WinitNamedKey::ArrowRight => NamedKey::ArrowRight,
        WinitNamedKey::ArrowDown => NamedKey::ArrowDown,
        WinitNamedKey::ArrowLeft => NamedKey::ArrowLeft,
        WinitNamedKey::Shift => NamedKey::Shift,
        WinitNamedKey::Control => NamedKey::Control,
        WinitNamedKey::Alt => NamedKey::Alt,
        WinitNamedKey::Meta => NamedKey::Meta,
        WinitNamedKey::CapsLock => NamedKey::CapsLock,
        _ => NamedKey::Unidentified,
    }
}

#[allow(clippy::cast_possible_truncation)]
fn engine_float(value: f64) -> f32 {
    value as f32
}

fn window_theme(theme: WindowTheme) -> Option<Theme> {
    match theme {
        WindowTheme::System => None,
        WindowTheme::Light => Some(Theme::Light),
        WindowTheme::Dark => Some(Theme::Dark),
    }
}

fn monitor_geometries(event_loop: &ActiveEventLoop) -> Vec<MonitorGeometry> {
    let mut monitors = Vec::new();
    if let Some(primary) = event_loop.primary_monitor()
        && let Some(geometry) = monitor_geometry(&primary)
    {
        monitors.push(geometry);
    }
    for monitor in event_loop.available_monitors() {
        if let Some(geometry) = monitor_geometry(&monitor)
            && !monitors.iter().any(|known| known.name == geometry.name)
        {
            monitors.push(geometry);
        }
    }
    monitors
}

fn monitor_geometry(monitor: &winit::monitor::MonitorHandle) -> Option<MonitorGeometry> {
    let position = monitor.position();
    let size = monitor.size();
    Some(MonitorGeometry {
        name: monitor.name()?,
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
        scale: monitor.scale_factor(),
    })
}

fn origin(url: &Url) -> String {
    format!(
        "{}://{}:{}",
        url.scheme(),
        url.host_str().unwrap_or_default(),
        url.port_or_known_default().unwrap_or_default()
    )
}
