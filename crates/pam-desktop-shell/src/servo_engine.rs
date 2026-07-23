use std::cell::{Cell, RefCell};
use std::rc::Rc;

use euclid::Scale;
use pam_desktop_protocol::{Effect, EffectKind, WindowConfig, WindowTheme};
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
use winit::window::{Theme, Window};

use crate::gateway::Gateway;
use crate::host_event::HostEvent;
use crate::runtime::DesktopRuntime;

pub fn run(runtime: DesktopRuntime) -> Result<(), String> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let (project, worker, bootstrap) = runtime.into_parts();
    let entry = project.resolve_entry(&bootstrap.entry)?;
    let event_loop = EventLoop::with_user_event()
        .build()
        .map_err(|error| format!("cannot create desktop event loop: {error}"))?;
    let gateway = Gateway::start(&project, &entry, worker, event_loop.create_proxy())?;
    let url = Url::parse(gateway.url())
        .map_err(|error| format!("cannot parse desktop gateway URL: {error}"))?;
    let mut application = Application::new(&event_loop, bootstrap.window, url, gateway);

    event_loop
        .run_app(&mut application)
        .map_err(|error| format!("desktop event loop failed: {error}"))
}

struct AppState {
    window: Window,
    servo: Servo,
    rendering_context: Rc<WindowRenderingContext>,
    webviews: RefCell<Vec<WebView>>,
    mouse_position: Cell<DevicePoint>,
    modifiers: Cell<ModifiersState>,
    allowed_origin: String,
    _gateway: Gateway,
}

impl AppState {
    fn with_webview(&self, operation: impl FnOnce(&WebView)) {
        if let Some(webview) = self.webviews.borrow().last() {
            operation(webview);
        }
    }

    fn apply_effects(&self, event_loop: &ActiveEventLoop, effects: Vec<Effect>) {
        for effect in effects {
            match effect.kind {
                EffectKind::SetWindowTitle => {
                    if let Some(title) = effect
                        .payload
                        .get("title")
                        .and_then(serde_json::Value::as_str)
                    {
                        self.window.set_title(title);
                    }
                }
                EffectKind::SetWindowVisible => {
                    if let Some(visible) = effect
                        .payload
                        .get("visible")
                        .and_then(serde_json::Value::as_bool)
                    {
                        self.window.set_visible(visible);
                    }
                }
                EffectKind::CloseWindow => event_loop.exit(),
            }
        }
    }

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
        self.with_webview(|webview| {
            webview.notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
                action,
                button,
                self.mouse_position.get().into(),
            )));
        });
    }

    fn handle_mouse_move(&self, position: PhysicalPosition<f64>) {
        let point = DevicePoint::new(engine_float(position.x), engine_float(position.y));
        self.mouse_position.set(point);
        self.with_webview(|webview| {
            webview.notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(point.into())));
        });
    }

    fn handle_wheel(&self, delta: MouseScrollDelta) {
        let (x, y) = match delta {
            MouseScrollDelta::LineDelta(x, y) => (f64::from(x) * 76.0, f64::from(y) * 76.0),
            MouseScrollDelta::PixelDelta(delta) => (delta.x, delta.y),
        };
        self.with_webview(|webview| {
            webview.notify_input_event(InputEvent::Wheel(WheelEvent::new(
                WheelDelta {
                    x,
                    y,
                    z: 0.0,
                    mode: WheelMode::DeltaPixel,
                },
                self.mouse_position.get().into(),
            )));
        });
    }
}

impl WebViewDelegate for AppState {
    fn notify_new_frame_ready(&self, _webview: WebView) {
        self.window.request_redraw();
    }

    fn notify_animating_changed(&self, _webview: WebView, _animating: bool) {
        self.window.request_redraw();
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
    window: WindowConfig,
    url: Url,
    gateway: Option<Gateway>,
}

impl Application {
    fn new(
        event_loop: &EventLoop<HostEvent>,
        window: WindowConfig,
        url: Url,
        gateway: Gateway,
    ) -> Self {
        Self::Initial(Box::new(InitialState {
            waker: Waker(event_loop.create_proxy()),
            window,
            url,
            gateway: Some(gateway),
        }))
    }
}

impl ApplicationHandler<HostEvent> for Application {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let Self::Initial(initial) = self else {
            return;
        };
        let attributes = Window::default_attributes()
            .with_title(initial.window.title.clone())
            .with_inner_size(LogicalSize::new(
                initial.window.width,
                initial.window.height,
            ))
            .with_min_inner_size(LogicalSize::new(
                initial.window.min_width,
                initial.window.min_height,
            ))
            .with_resizable(initial.window.resizable)
            .with_visible(initial.window.visible)
            .with_theme(window_theme(initial.window.theme));
        let window = event_loop
            .create_window(attributes)
            .expect("validated window configuration should create a window");
        let display_handle = event_loop
            .display_handle()
            .expect("desktop event loop should expose its display");
        let window_handle = window
            .window_handle()
            .expect("desktop window should expose its handle");
        let rendering_context = Rc::new(
            WindowRenderingContext::new(display_handle, window_handle, window.inner_size())
                .expect("Servo should create a rendering context for the desktop window"),
        );
        let _ = rendering_context.make_current();
        let servo = ServoBuilder::default()
            .event_loop_waker(Box::new(initial.waker.clone()))
            .build();
        servo.setup_logging();
        let gateway = initial
            .gateway
            .take()
            .expect("desktop gateway should only be consumed once");
        let allowed_origin = origin(&initial.url);

        let state = Rc::new(AppState {
            window,
            servo,
            rendering_context,
            webviews: RefCell::new(Vec::new()),
            mouse_position: Cell::new(DevicePoint::default()),
            modifiers: Cell::new(ModifiersState::empty()),
            allowed_origin,
            _gateway: gateway,
        });
        let webview = WebViewBuilder::new(&state.servo, state.rendering_context.clone())
            .url(initial.url.clone())
            .hidpi_scale_factor(Scale::new(engine_float(state.window.scale_factor())))
            .delegate(state.clone())
            .build();
        webview.focus();
        webview.show();
        state.webviews.borrow_mut().push(webview);
        *self = Self::Running(state);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: HostEvent) {
        let Self::Running(state) = self else {
            return;
        };
        match event {
            HostEvent::ServoWake => state.servo.spin_event_loop(),
            HostEvent::ApplyEffects(effects) => state.apply_effects(event_loop, effects),
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Self::Running(state) = self else {
            return;
        };
        state.servo.spin_event_loop();

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => {
                state.with_webview(WebView::paint);
                state.rendering_context.present();
            }
            WindowEvent::Resized(size) => state.with_webview(|webview| webview.resize(size)),
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                state.with_webview(|webview| {
                    webview.set_hidpi_scale_factor(Scale::new(engine_float(scale_factor)));
                    webview.resize(state.window.inner_size());
                });
            }
            WindowEvent::Focused(true) => state.with_webview(WebView::focus),
            WindowEvent::Focused(false) => state.with_webview(WebView::blur),
            WindowEvent::CursorMoved { position, .. } => state.handle_mouse_move(position),
            WindowEvent::CursorLeft { .. } => {
                state.with_webview(|webview| {
                    webview.notify_input_event(InputEvent::MouseLeftViewport(
                        MouseLeftViewportEvent::default(),
                    ));
                });
            }
            WindowEvent::MouseInput {
                state: button_state,
                button,
                ..
            } => state.handle_mouse_button(button, button_state),
            WindowEvent::MouseWheel { delta, .. } => state.handle_wheel(delta),
            WindowEvent::ModifiersChanged(modifiers) => state.modifiers.set(modifiers.state()),
            WindowEvent::KeyboardInput { event, .. } => {
                state.with_webview(|webview| {
                    webview.notify_input_event(InputEvent::Keyboard(keyboard_event(
                        &event,
                        state.modifiers.get(),
                    )));
                });
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Self::Running(state) = self else {
            return;
        };
        let animating = state
            .webviews
            .borrow()
            .last()
            .is_some_and(WebView::animating);
        if animating {
            state.servo.spin_event_loop();
            state.window.request_redraw();
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
    // Servo's device geometry is f32 while Winit reports coordinates and
    // scale factors as f64. Window-sized values are safely representable.
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
