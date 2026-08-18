use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use euclid::Scale;
use pam_desktop_protocol::{
    Bootstrap, Effect, EffectKind, MAIN_WINDOW_ID, WindowConfig, WindowTheme,
};
use servo::{
    Code, DevicePoint, EventLoopWaker, InputEvent, Key, KeyState, KeyboardEvent, Location,
    Modifiers, MouseButton as ServoMouseButton, MouseButtonAction, MouseButtonEvent,
    MouseLeftViewportEvent, MouseMoveEvent, NamedKey, NavigationRequest, RenderingContext, Servo,
    ServoBuilder, WebView, WebViewBuilder, WebViewDelegate, WheelDelta, WheelEvent, WheelMode,
    WindowRenderingContext,
};
use tracing::warn;
use url::Url;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{
    Key as WinitKey, KeyCode, ModifiersState, NamedKey as WinitNamedKey, PhysicalKey,
};
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::{Fullscreen, Theme, Window, WindowId, WindowLevel};

use crate::dev_event::{self, EventCode};
use crate::gateway::Gateway;
use crate::host_event::HostEvent;
use crate::lifecycle::InstanceGuard;
use crate::native::show_dialog;
use crate::native_shell::NativeShell;
use crate::runtime::DesktopRuntime;

pub fn run(
    runtime: DesktopRuntime,
    watch: bool,
    mut instance: InstanceGuard,
    initial_arguments: Vec<String>,
) -> Result<(), String> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let (project, supervisor, bootstrap) = runtime.into_parts();
    let project_root = project.root().to_path_buf();
    if watch {
        dev_event::emit(
            EventCode::SessionStarting,
            &project_root,
            &serde_json::json!({}),
        );
    }
    let event_loop = EventLoop::with_user_event()
        .build()
        .map_err(|error| format!("cannot create desktop event loop: {error}"))?;
    NativeShell::install_event_handlers(&event_loop.create_proxy());
    let gateway = Gateway::start(
        &project,
        supervisor,
        bootstrap.clone(),
        event_loop.create_proxy(),
        watch,
    )?;
    if watch {
        dev_event::emit(
            EventCode::SessionReady,
            &project_root,
            &serde_json::json!({"gatewayUrl": gateway.url()}),
        );
    }
    let events = gateway.event_hub();
    if !initial_arguments.is_empty() {
        events.publish(pam_desktop_protocol::ClientEvent {
            name: "pam.lifecycle.opened".to_owned(),
            payload: serde_json::json!({"arguments": initial_arguments}),
            window_id: None,
        });
    }
    instance.listen(move |activation| {
        events.publish(pam_desktop_protocol::ClientEvent {
            name: "pam.lifecycle.second-instance".to_owned(),
            payload: serde_json::json!({"arguments": activation.arguments}),
            window_id: None,
        });
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

struct DesktopWindow {
    id: String,
    webview: WebView,
    rendering_context: Rc<WindowRenderingContext>,
    window: Window,
    mouse_position: Cell<DevicePoint>,
    modifiers: Cell<ModifiersState>,
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
}

impl AppState {
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

        let mut next = HashMap::with_capacity(bootstrap.windows.len());
        for config in &bootstrap.windows {
            let desktop_window = self.create_window(event_loop, config)?;
            next.insert(desktop_window.window.id(), desktop_window);
        }
        self.windows.replace(next);
        Ok(())
    }

    fn create_window(
        self: &Rc<Self>,
        event_loop: &ActiveEventLoop,
        config: &WindowConfig,
    ) -> Result<DesktopWindow, String> {
        let attributes = Window::default_attributes()
            .with_title(config.title.clone())
            .with_inner_size(LogicalSize::new(config.width, config.height))
            .with_min_inner_size(LogicalSize::new(config.min_width, config.min_height))
            .with_resizable(config.resizable)
            .with_visible(config.visible)
            .with_theme(window_theme(config.theme))
            .with_decorations(config.decorated)
            .with_transparent(config.transparent)
            .with_window_level(if config.always_on_top {
                WindowLevel::AlwaysOnTop
            } else {
                WindowLevel::Normal
            })
            .with_maximized(config.maximized)
            .with_fullscreen(config.fullscreen.then(|| Fullscreen::Borderless(None)));
        let window = event_loop
            .create_window(attributes)
            .map_err(|error| format!("cannot create window {:?}: {error}", config.id))?;
        let display_handle = event_loop
            .display_handle()
            .map_err(|error| format!("desktop event loop has no display handle: {error}"))?;
        let window_handle = window
            .window_handle()
            .map_err(|error| format!("window {:?} has no native handle: {error}", config.id))?;
        let rendering_context = Rc::new(
            WindowRenderingContext::new(display_handle, window_handle, window.inner_size())
                .map_err(|error| {
                    format!(
                        "Servo cannot create the rendering context for {:?}: {error:?}",
                        config.id,
                    )
                })?,
        );
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
        } else {
            webview.hide();
        }
        if config.id == MAIN_WINDOW_ID {
            webview.focus();
        }

        Ok(DesktopWindow {
            id: config.id.clone(),
            webview,
            rendering_context,
            window,
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
                    if effect.window_id == MAIN_WINDOW_ID || self.windows.borrow().len() == 1 {
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
                EffectKind::SetMenuItemEnabled
                | EffectKind::SetMenuItemChecked
                | EffectKind::SetTrayVisible => {
                    unreachable!("native shell effects are handled before window effects")
                }
            }
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
            && self.native_shell.borrow().close_behavior()
                == pam_desktop_protocol::TrayCloseBehavior::Hide
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
        let servo = ServoBuilder::default()
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
        let state = Rc::new(AppState {
            servo,
            windows: RefCell::new(HashMap::new()),
            allowed_origin,
            gateway,
            native_shell: RefCell::new(native_shell),
            event_proxy,
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
            HostEvent::ApplyEffects(effects) => state.apply_effects(event_loop, effects),
            HostEvent::ReloadViews => state.reload_views(),
            HostEvent::Reconfigure(bootstrap) => {
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
            HostEvent::Exit => event_loop.exit(),
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
            state.close_requested(event_loop, window_id);
            return;
        }
        let windows = state.windows.borrow();
        let Some(window) = windows.get(&window_id) else {
            return;
        };
        match event {
            WindowEvent::RedrawRequested => {
                window.webview.paint();
                window.rendering_context.present();
            }
            WindowEvent::Resized(size) => window.webview.resize(size),
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                window
                    .webview
                    .set_hidpi_scale_factor(Scale::new(engine_float(scale_factor)));
                window.webview.resize(window.window.inner_size());
            }
            WindowEvent::Focused(true) => window.webview.focus(),
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

fn origin(url: &Url) -> String {
    format!(
        "{}://{}:{}",
        url.scheme(),
        url.host_str().unwrap_or_default(),
        url.port_or_known_default().unwrap_or_default()
    )
}
