//! `Backend` implementation on GTK 4.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::process::ExitCode;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib, graphene, pango};
use pam_desktop_ui::backend::{
    Backend, BackendEvent, ImageContent, ImageKey, ImageSource, KeyEvent, Modifiers, NodeId, PointerButton, PointerEvent,
    PointerPhase, ScrollConfig, TextContent, ViewClass,
    WindowConfig, WindowId, WindowMetrics,
};
use pam_desktop_ui::geometry::{Point, Rect, Size};
use pam_desktop_ui::runtime::{self, EventQueue, Inbox, Runtime, RuntimeConfig, Shared};
use pam_desktop_ui::scene::IdMap;
use pam_desktop_ui::style::{Decoration, LayerState};
use pam_desktop_ui::text::{TextBlock, TextMetrics};

use crate::text;
use crate::view::{self, ImageState, PamView};

struct ViewEntry {
    /// The widget placed in the parent.
    widget: gtk::Widget,
    /// Where child node widgets are added.
    container: PamView,
    class: ViewClass,
}

struct WindowEntry {
    window: gtk::ApplicationWindow,
    host: PamView,
    size: Rc<Cell<Size>>,
    presented: bool,
    root: Option<NodeId>,
    overlays: Vec<NodeId>,
}

pub struct GtkBackend {
    application: gtk::Application,
    events: Rc<EventQueue>,
    views: IdMap<ViewEntry>,
    windows: HashMap<WindowId, WindowEntry>,
    measure_layout: Option<pango::Layout>,
    frame_requested: Rc<Cell<bool>>,
    textures: Rc<RefCell<HashMap<ImageKey, gdk::Texture>>>,
    wake_source: Option<glib::SourceId>,
    /// Set when the pending wake already ran (its source id is then stale).
    wake_fired: Rc<Cell<bool>>,
}

impl GtkBackend {
    #[must_use]
    pub fn new(application: gtk::Application, events: Rc<EventQueue>) -> Self {
        Self {
            application,
            events,
            views: IdMap::default(),
            windows: HashMap::new(),
            measure_layout: None,
            frame_requested: Rc::new(Cell::new(false)),
            textures: Rc::new(RefCell::new(HashMap::new())),
            wake_source: None,
            wake_fired: Rc::new(Cell::new(false)),
        }
    }

    fn settings_font_scale() -> f32 {
        // GNOME's text scaling factor is published through the Xft DPI.
        gtk::Settings::default()
            .map(|settings| settings.gtk_xft_dpi())
            .filter(|dpi| *dpi > 0)
            .map_or(1.0, |dpi| (dpi as f32 / 1024.0 / 96.0).clamp(0.5, 4.0))
    }

    fn prefers_dark() -> bool {
        gtk::Settings::default().is_some_and(|settings| {
            settings.is_gtk_application_prefer_dark_theme()
                || settings.gtk_theme_name().is_some_and(|name| name.to_lowercase().contains("dark"))
        })
    }

    fn modifiers(state: gdk::ModifierType) -> Modifiers {
        Modifiers {
            shift: state.contains(gdk::ModifierType::SHIFT_MASK),
            control: state.contains(gdk::ModifierType::CONTROL_MASK),
            alt: state.contains(gdk::ModifierType::ALT_MASK),
            meta: state.contains(gdk::ModifierType::META_MASK) || state.contains(gdk::ModifierType::SUPER_MASK),
        }
    }

    /// Keys nobody consumed (bubble phase), as W3C `key` values.
    fn install_key_events(&self, window: WindowId, native: &gtk::ApplicationWindow) {
        let controller = gtk::EventControllerKey::new();
        controller.set_propagation_phase(gtk::PropagationPhase::Bubble);
        let events = Rc::clone(&self.events);
        controller.connect_key_pressed(move |_, key, _, state| {
            events.push(BackendEvent::Key(KeyEvent {
                window,
                pressed: true,
                key: key_name(key),
                modifiers: GtkBackend::modifiers(state),
                repeat: false,
            }));
            glib::Propagation::Proceed
        });
        let events = Rc::clone(&self.events);
        controller.connect_key_released(move |_, key, _, state| {
            events.push(BackendEvent::Key(KeyEvent {
                window,
                pressed: false,
                key: key_name(key),
                modifiers: GtkBackend::modifiers(state),
                repeat: false,
            }));
        });
        native.add_controller(controller);
    }

    /// The window host holds the root view and, above it, the overlays.
    fn arrange_window(entry: &WindowEntry, views: &IdMap<ViewEntry>) {
        let wanted: Vec<gtk::Widget> = entry
            .root
            .iter()
            .chain(entry.overlays.iter())
            .filter_map(|id| views.get(id).map(|view| view.widget.clone()))
            .collect();
        // Detach whatever is no longer presented in this window.
        let mut child = entry.host.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            if !wanted.contains(&current) {
                current.unparent();
            }
        }
        let mut previous: Option<gtk::Widget> = None;
        for widget in wanted {
            let owned = widget.parent().is_some_and(|parent| parent == *entry.host.upcast_ref::<gtk::Widget>());
            if !owned {
                if widget.parent().is_some() {
                    widget.unparent();
                }
                widget.insert_after(&entry.host, previous.as_ref());
            } else if widget.prev_sibling() != previous {
                widget.insert_after(&entry.host, previous.as_ref());
            }
            previous = Some(widget);
        }
        entry.host.queue_allocate();
    }

    fn install_pointer_events(&self, window: WindowId, host: &PamView) {
        let controller = gtk::EventControllerLegacy::new();
        controller.set_propagation_phase(gtk::PropagationPhase::Capture);
        let events = Rc::clone(&self.events);
        let target = host.downgrade();
        controller.connect_event(move |_, event| {
            let Some(host) = target.upgrade() else { return glib::Propagation::Proceed };
            let phase = match event.event_type() {
                gdk::EventType::ButtonPress => PointerPhase::Down,
                gdk::EventType::ButtonRelease => PointerPhase::Up,
                gdk::EventType::MotionNotify => PointerPhase::Move,
                gdk::EventType::LeaveNotify => PointerPhase::Leave,
                gdk::EventType::TouchCancel => PointerPhase::Cancel,
                _ => return glib::Propagation::Proceed,
            };
            let position = event.position().and_then(|(x, y)| {
                let native = host.native()?;
                let (offset_x, offset_y) = native.surface_transform();
                let point = graphene::Point::new((x - offset_x) as f32, (y - offset_y) as f32);
                let converted = native.compute_point(&host, &point)?;
                Some(Point::new(converted.x(), converted.y()))
            });
            let Some(position) = position.or_else(|| (phase == PointerPhase::Leave).then_some(Point::ZERO)) else {
                return glib::Propagation::Proceed;
            };
            let button = match event.downcast_ref::<gdk::ButtonEvent>().map(gdk::ButtonEvent::button) {
                Some(2) => PointerButton::Middle,
                Some(3) => PointerButton::Secondary,
                // The "back" side button of a mouse.
                Some(8) => {
                    if phase == PointerPhase::Up {
                        events.push(BackendEvent::BackRequested { window });
                    }
                    return glib::Propagation::Proceed;
                }
                Some(button) if button > 3 => return glib::Propagation::Proceed,
                _ => PointerButton::Primary,
            };
            let state = event.modifier_state();
            events.push(BackendEvent::Pointer(PointerEvent {
                window,
                phase,
                position,
                button,
                modifiers: GtkBackend::modifiers(state),
                click_count: 1,
                time_ms: u64::from(event.time()),
            }));
            glib::Propagation::Proceed
        });
        host.add_controller(controller);
    }
}

impl Backend for GtkBackend {
    fn open_window(&mut self, window: WindowId, config: &WindowConfig) {
        if let Some(entry) = self.windows.get(&window) {
            entry.window.set_title(Some(&config.title));
            entry.window.set_resizable(config.resizable);
            return;
        }
        let native = gtk::ApplicationWindow::new(&self.application);
        native.set_title(Some(&config.title));
        native.set_default_size(config.size.width.round() as i32, config.size.height.round() as i32);
        native.set_resizable(config.resizable);
        if let Some(minimum) = config.min_size {
            native.set_size_request(minimum.width.round() as i32, minimum.height.round() as i32);
        }

        let host = PamView::new();
        let size = Rc::new(Cell::new(config.size));
        {
            let events = Rc::clone(&self.events);
            let size = Rc::clone(&size);
            let weak = host.downgrade();
            host.with_state(|state| {
                state.on_resize = Some(Box::new(move |width, height| {
                    let next = Size::new(width, height);
                    if size.get() == next {
                        return;
                    }
                    size.set(next);
                    let scale = weak.upgrade().map_or(1.0, |host| host.scale_factor() as f32);
                    // Allocation runs inside GTK's layout pass: apply the new
                    // viewport right after it instead of re-entering layout.
                    let events = Rc::clone(&events);
                    glib::idle_add_local_full(glib::Priority::HIGH, move || {
                        events.push(BackendEvent::WindowResized { window, size: next, scale });
                        glib::ControlFlow::Break
                    });
                }));
            });
        }
        native.set_child(Some(&host));
        self.install_pointer_events(window, &host);
        self.install_key_events(window, &native);

        let events = Rc::clone(&self.events);
        native.connect_close_request(move |_| {
            events.push(BackendEvent::WindowCloseRequested { window });
            glib::Propagation::Stop
        });
        let events = Rc::clone(&self.events);
        native.connect_is_active_notify(move |native| {
            events.push(BackendEvent::WindowFocusChanged { window, focused: native.is_active() });
        });
        let events = Rc::clone(&self.events);
        let size_for_scale = Rc::clone(&size);
        native.connect_scale_factor_notify(move |native| {
            events.push(BackendEvent::WindowResized {
                window,
                size: size_for_scale.get(),
                scale: native.scale_factor() as f32,
            });
        });

        if self.measure_layout.is_none() {
            self.measure_layout = Some(host.create_pango_layout(None));
        }
        self.windows.insert(window, WindowEntry { window: native, host, size, presented: false, root: None, overlays: Vec::new() });
    }

    fn close_window(&mut self, window: WindowId) {
        if let Some(entry) = self.windows.remove(&window) {
            entry.window.destroy();
        }
    }

    fn window_metrics(&self, window: WindowId) -> WindowMetrics {
        let Some(entry) = self.windows.get(&window) else { return WindowMetrics::default() };
        let refresh_rate = entry
            .window
            .surface()
            .and_then(|surface| WidgetExt::display(&entry.window).monitor_at_surface(&surface))
            .map(|monitor| monitor.refresh_rate() as f32 / 1000.0)
            .filter(|rate| *rate > 1.0)
            .unwrap_or(60.0);
        WindowMetrics {
            size: entry.size.get(),
            scale: entry.window.scale_factor() as f32,
            font_scale: Self::settings_font_scale(),
            dark: Self::prefers_dark(),
            refresh_rate,
            reduced_motion: gtk::Settings::default().is_some_and(|settings| !settings.is_gtk_enable_animations()),
        }
    }

    fn set_window_root(&mut self, window: WindowId, root: Option<NodeId>) {
        let Some(entry) = self.windows.get_mut(&window) else { return };
        entry.root = root;
        Self::arrange_window(entry, &self.views);
        if root.is_some() && !entry.presented {
            entry.presented = true;
            entry.window.present();
        }
    }

    fn set_window_overlays(&mut self, window: WindowId, overlays: &[NodeId]) {
        let Some(entry) = self.windows.get_mut(&window) else { return };
        entry.overlays = overlays.to_vec();
        Self::arrange_window(entry, &self.views);
    }

    fn create_view(&mut self, id: NodeId, class: ViewClass) {
        let container = PamView::new();
        {
            let focus = gtk::EventControllerFocus::new();
            let events = Rc::clone(&self.events);
            focus.connect_enter(move |_| events.push(BackendEvent::FocusChanged { id, focused: true }));
            let events = Rc::clone(&self.events);
            focus.connect_leave(move |_| events.push(BackendEvent::FocusChanged { id, focused: false }));
            container.add_controller(focus);
        }
        let widget: gtk::Widget = match class {
            ViewClass::Scroll => {
                let scrolled = gtk::ScrolledWindow::new();
                // The engine sizes the viewport; content never dictates it.
                scrolled.set_policy(gtk::PolicyType::External, gtk::PolicyType::Automatic);
                scrolled.set_propagate_natural_width(false);
                scrolled.set_propagate_natural_height(false);
                scrolled.set_child(Some(&container));
                let report = {
                    let events = Rc::clone(&self.events);
                    let weak = scrolled.downgrade();
                    move || {
                        if let Some(scrolled) = weak.upgrade() {
                            let offset = Point::new(scrolled.hadjustment().value() as f32, scrolled.vadjustment().value() as f32);
                            events.push(BackendEvent::ScrollChanged { id, offset });
                        }
                    }
                };
                let report_horizontal = report.clone();
                scrolled.hadjustment().connect_value_changed(move |_| report_horizontal());
                scrolled.vadjustment().connect_value_changed(move |_| report());
                scrolled.upcast()
            }
            _ => container.clone().upcast(),
        };
        self.views.insert(id, ViewEntry { widget, container, class });
    }

    fn destroy_view(&mut self, id: NodeId) {
        if let Some(entry) = self.views.remove(&id) {
            if entry.widget.parent().is_some() {
                entry.widget.unparent();
            }
            view::forget_layout(&entry.widget);
        }
    }

    fn set_children(&mut self, parent: NodeId, children: &[NodeId]) {
        let Some(parent_entry) = self.views.get(&parent) else { return };
        let container = parent_entry.container.clone();
        let mut previous: Option<gtk::Widget> = None;
        for child in children {
            let Some(entry) = self.views.get(child) else { continue };
            let widget = &entry.widget;
            let owned = widget.parent().is_some_and(|current| current == *container.upcast_ref::<gtk::Widget>());
            if owned {
                if widget.prev_sibling() != previous {
                    widget.insert_after(&container, previous.as_ref());
                }
            } else {
                if widget.parent().is_some() {
                    widget.unparent();
                }
                widget.insert_after(&container, previous.as_ref());
            }
            previous = Some(widget.clone());
        }
        container.queue_allocate();
    }

    fn set_frame(&mut self, id: NodeId, frame: Rect) {
        let Some(entry) = self.views.get(&id) else { return };
        view::set_layout(&entry.widget, |layout| layout.frame = frame);
        if entry.class != ViewClass::Scroll {
            entry.container.set_size(frame.size());
        }
        if let Some(parent) = entry.widget.parent() {
            parent.queue_allocate();
        }
    }

    fn set_decoration(&mut self, id: NodeId, decoration: &Decoration) {
        if let Some(entry) = self.views.get(&id) {
            entry.container.set_decoration(decoration);
        }
    }

    fn set_layer(&mut self, id: NodeId, layer: &LayerState) {
        let Some(entry) = self.views.get(&id) else { return };
        entry.widget.set_opacity(f64::from(layer.opacity));
        entry.widget.set_visible(layer.visible);
        view::set_layout(&entry.widget, |layout| layout.transform = layer.transform);
        if let Some(parent) = entry.widget.parent() {
            parent.queue_allocate();
        }
    }

    fn set_text(&mut self, id: NodeId, text: Option<&TextContent>) {
        if let Some(entry) = self.views.get(&id) {
            entry.container.set_text(text);
        }
    }

    fn set_focusable(&mut self, id: NodeId, focusable: bool) {
        if let Some(entry) = self.views.get(&id) {
            entry.container.set_focusable(focusable);
            if focusable {
                entry.container.add_css_class("pam-focusable");
            } else {
                entry.container.remove_css_class("pam-focusable");
            }
        }
    }

    fn focus(&mut self, id: Option<NodeId>) {
        match id.and_then(|id| self.views.get(&id)) {
            Some(entry) => {
                entry.container.grab_focus();
            }
            None => {
                for window in self.windows.values() {
                    gtk::prelude::GtkWindowExt::set_focus(&window.window, None::<&gtk::Widget>);
                }
            }
        }
    }

    fn load_image(&mut self, key: ImageKey, source: ImageSource) {
        let events = Rc::clone(&self.events);
        let textures = Rc::clone(&self.textures);
        // Decoding runs on a worker thread; the texture is handed back to the UI thread.
        let (sender, receiver) = std::sync::mpsc::channel::<Result<gdk::Texture, String>>();
        std::thread::spawn(move || {
            let decoded = match source {
                ImageSource::Bytes(bytes) => gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes.to_vec())),
                ImageSource::File(path) => gdk::Texture::from_filename(path),
            };
            let _ = sender.send(decoded.map_err(|error| error.to_string()));
            glib::MainContext::default().wakeup();
        });
        glib::idle_add_local_full(glib::Priority::DEFAULT, move || match receiver.try_recv() {
            Ok(Ok(texture)) => {
                let size = Size::new(texture.width() as f32, texture.height() as f32);
                textures.borrow_mut().insert(key, texture);
                events.push(BackendEvent::ImageLoaded { key, size });
                glib::ControlFlow::Break
            }
            Ok(Err(message)) => {
                events.push(BackendEvent::ImageFailed { key, message });
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                events.push(BackendEvent::ImageFailed { key, message: "image decoder stopped".into() });
                glib::ControlFlow::Break
            }
        });
    }

    fn release_image(&mut self, key: ImageKey) {
        self.textures.borrow_mut().remove(&key);
    }

    fn set_image(&mut self, id: NodeId, image: Option<&ImageContent>) {
        let Some(entry) = self.views.get(&id) else { return };
        let state = image.and_then(|content| {
            let texture = self.textures.borrow().get(&content.key).cloned()?;
            Some(ImageState { content: *content, texture })
        });
        entry.container.set_image(state);
    }

    fn set_scroll_content(&mut self, id: NodeId, size: Size) {
        if let Some(entry) = self.views.get(&id) {
            entry.container.set_content_size(Some(size));
        }
    }

    fn set_scroll_config(&mut self, id: NodeId, config: &ScrollConfig) {
        let Some(scrolled) = self.views.get(&id).and_then(|entry| entry.widget.downcast_ref::<gtk::ScrolledWindow>()) else {
            return;
        };
        let scrolling = if !config.enabled {
            // Programmatic scrolling still works without a scrollbar.
            gtk::PolicyType::External
        } else if config.shows_indicators {
            gtk::PolicyType::Automatic
        } else {
            gtk::PolicyType::External
        };
        if config.horizontal {
            scrolled.set_policy(scrolling, gtk::PolicyType::External);
        } else {
            scrolled.set_policy(gtk::PolicyType::External, scrolling);
        }
        scrolled.set_kinetic_scrolling(config.enabled);
    }

    fn scroll_to(&mut self, id: NodeId, offset: Point, _animated: bool) {
        let Some(scrolled) = self.views.get(&id).and_then(|entry| entry.widget.downcast_ref::<gtk::ScrolledWindow>()) else {
            return;
        };
        scrolled.hadjustment().set_value(f64::from(offset.x));
        scrolled.vadjustment().set_value(f64::from(offset.y));
    }

    fn measure_text(&mut self, block: &TextBlock, max_width: f32) -> TextMetrics {
        match &self.measure_layout {
            Some(layout) => text::measure(layout, block, max_width),
            None => TextMetrics::default(),
        }
    }

    fn request_frame(&mut self) {
        if self.frame_requested.replace(true) {
            return;
        }
        let Some(entry) = self.windows.values().next() else {
            self.frame_requested.set(false);
            return;
        };
        let events = Rc::clone(&self.events);
        let requested = Rc::clone(&self.frame_requested);
        entry.host.add_tick_callback(move |_, clock| {
            requested.set(false);
            events.push(BackendEvent::Frame { time_ms: u64::try_from(clock.frame_time() / 1000).unwrap_or(0) });
            glib::ControlFlow::Break
        });
    }

    fn schedule_wake(&mut self, delay_ms: u32) {
        if let Some(source) = self.wake_source.take() {
            // Removing a source that already ran is an error in GLib.
            if !self.wake_fired.get() {
                source.remove();
            }
        }
        let events = Rc::clone(&self.events);
        let slot = Rc::clone(&self.wake_fired);
        slot.set(false);
        let fired = Rc::clone(&slot);
        self.wake_source = Some(glib::timeout_add_local_once(std::time::Duration::from_millis(u64::from(delay_ms)), move || {
            fired.set(true);
            events.push(BackendEvent::Wake);
        }));
    }

    fn quit(&mut self) {
        for (_, entry) in self.windows.drain() {
            entry.window.destroy();
        }
        self.application.quit();
    }
}

/// GDK key symbol to the W3C UI Events `key` value.
fn key_name(key: gdk::Key) -> String {
    let named = match key {
        gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter => "Enter",
        gdk::Key::Escape => "Escape",
        gdk::Key::Tab | gdk::Key::ISO_Left_Tab | gdk::Key::KP_Tab => "Tab",
        gdk::Key::BackSpace => "Backspace",
        gdk::Key::Delete | gdk::Key::KP_Delete => "Delete",
        gdk::Key::Insert | gdk::Key::KP_Insert => "Insert",
        gdk::Key::Left | gdk::Key::KP_Left => "ArrowLeft",
        gdk::Key::Right | gdk::Key::KP_Right => "ArrowRight",
        gdk::Key::Up | gdk::Key::KP_Up => "ArrowUp",
        gdk::Key::Down | gdk::Key::KP_Down => "ArrowDown",
        gdk::Key::Home | gdk::Key::KP_Home => "Home",
        gdk::Key::End | gdk::Key::KP_End => "End",
        gdk::Key::Page_Up | gdk::Key::KP_Page_Up => "PageUp",
        gdk::Key::Page_Down | gdk::Key::KP_Page_Down => "PageDown",
        gdk::Key::space | gdk::Key::KP_Space => " ",
        gdk::Key::Shift_L | gdk::Key::Shift_R => "Shift",
        gdk::Key::Control_L | gdk::Key::Control_R => "Control",
        gdk::Key::Alt_L | gdk::Key::Alt_R => "Alt",
        gdk::Key::Meta_L | gdk::Key::Meta_R | gdk::Key::Super_L | gdk::Key::Super_R => "Meta",
        gdk::Key::Caps_Lock => "CapsLock",
        gdk::Key::Menu => "ContextMenu",
        gdk::Key::Print => "PrintScreen",
        gdk::Key::Pause => "Pause",
        gdk::Key::F1 => "F1",
        gdk::Key::F2 => "F2",
        gdk::Key::F3 => "F3",
        gdk::Key::F4 => "F4",
        gdk::Key::F5 => "F5",
        gdk::Key::F6 => "F6",
        gdk::Key::F7 => "F7",
        gdk::Key::F8 => "F8",
        gdk::Key::F9 => "F9",
        gdk::Key::F10 => "F10",
        gdk::Key::F11 => "F11",
        gdk::Key::F12 => "F12",
        _ => "",
    };
    if !named.is_empty() {
        return named.to_owned();
    }
    match key.to_unicode() {
        Some(character) if !character.is_control() => character.to_string(),
        _ => key.name().map_or_else(|| String::from("Unidentified"), |name| name.to_string()),
    }
}

/// Focus ring of keyboard-focusable boxes, drawn by GTK around the widget.
const STYLESHEET: &str = "
.pam-focusable:focus-visible {
    outline: 2px solid @accent_color;
    outline-offset: 2px;
}
";

/// Runs a PAM Desktop application on GTK until its window closes.
pub fn run(application_id: &str, config: RuntimeConfig) -> ExitCode {
    let application = gtk::Application::new(Some(application_id), gio::ApplicationFlags::NON_UNIQUE);
    let config = RefCell::new(Some(config));
    let shared: Rc<RefCell<Option<Rc<Shared<GtkBackend>>>>> = Rc::new(RefCell::new(None));
    let failed = Rc::new(Cell::new(false));

    let keep = Rc::clone(&shared);
    let failure = Rc::clone(&failed);
    application.connect_activate(move |application| {
        let Some(config) = config.borrow_mut().take() else { return };
        if let Some(display) = gdk::Display::default() {
            let provider = gtk::CssProvider::new();
            provider.load_from_string(STYLESHEET);
            gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
        let events = EventQueue::new();
        let inbox = Inbox::new(|| {
            glib::MainContext::default().invoke(runtime::pump);
        });
        let backend = GtkBackend::new(application.clone(), Rc::clone(&events));
        match Runtime::new(backend, inbox, events, config) {
            Ok(runtime) => {
                *keep.borrow_mut() = Some(Shared::install(runtime));
                runtime::pump();
            }
            Err(error) => {
                eprintln!("pam-desktop: could not start the PHP worker: {error}");
                failure.set(true);
                application.quit();
            }
        }
    });
    let status = application.run_with_args::<&str>(&[]);
    runtime::set_pump(None);
    shared.borrow_mut().take();
    if failed.get() || status != glib::ExitCode::SUCCESS {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
