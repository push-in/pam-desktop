//! `Backend` implementation on GTK 4.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::process::ExitCode;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib, graphene, pango};
use pam_desktop_ui::backend::{
    AlertButtonStyle, AlertConfig, Backend, BackendEvent, ImageContent, ImageKey, ImageSource, InputConfig, KeyEvent,
    Modifiers, NodeId, PointerButton, PointerEvent,
    PointerPhase, ScrollConfig, TextContent, ViewClass,
    WindowConfig, WindowId, WindowMetrics,
};
use pam_desktop_ui::geometry::{Point, Rect, Size};
use pam_desktop_ui::runtime::{self, EventQueue, Inbox, Runtime, RuntimeConfig, Shared};
use pam_desktop_ui::scene::IdMap;
use pam_desktop_ui::shell::{Cursor, FileDialogConfig, MenuNode, NotificationConfig, Shortcut, TrayConfig, WindowCommand, WindowState};
use pam_desktop_ui::style::{Decoration, LayerState};
use pam_desktop_ui::text::{TextBlock, TextMetrics};

use crate::input::InputHost;
use crate::shell::{self, Shell};
use crate::text;
use crate::view::{self, ImageState, PamView};

struct ViewEntry {
    /// The widget placed in the parent.
    widget: gtk::Widget,
    /// Where child node widgets are added.
    container: PamView,
    class: ViewClass,
    /// The native text control of input classes.
    input: Option<InputHost>,
    /// What assistive technology was told, when it is more than the defaults.
    accessibility: Option<Box<pam_desktop_ui::accessibility::Accessibility>>,
    /// The text the view shows and the accessible name last applied.
    plain_text: Option<String>,
    announced: String,
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
    shell: Shell,
    /// The last button press, needed to start a window move.
    last_press: Rc<RefCell<Option<(gdk::Device, i32, f64, f64, u32)>>>,
}

impl GtkBackend {
    #[must_use]
    pub fn new(application: gtk::Application, events: Rc<EventQueue>) -> Self {
        let shell = Shell::new(application.clone(), Rc::clone(&events));
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
            shell,
            last_press: Rc::new(RefCell::new(None)),
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
        let last_press = Rc::clone(&self.last_press);
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
            if phase == PointerPhase::Down {
                if let (Some(device), Some((x, y)), Some(press)) = (event.device(), event.position(), event.downcast_ref::<gdk::ButtonEvent>()) {
                    *last_press.borrow_mut() = Some((device, press.button() as i32, x, y, event.time()));
                }
            }
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
        shell::accept_file_drops(&host, window, &self.events);
        shell::watch_window_state(&native, window, &self.events);

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
        let input = match class {
            ViewClass::Input => Some(InputHost::new(id, false, &container, &self.events)),
            ViewClass::TextArea => Some(InputHost::new(id, true, &container, &self.events)),
            _ => None,
        };
        self.views.insert(id, ViewEntry { widget, container, class, input, accessibility: None, plain_text: None, announced: String::new() });
    }

    fn destroy_view(&mut self, id: NodeId) {
        if let Some(entry) = self.views.remove(&id) {
            if let Some(input) = &entry.input {
                input.destroy();
            }
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
        let Some(entry) = self.views.get_mut(&id) else { return };
        view::set_layout(&entry.widget, |layout| layout.frame = frame);
        if entry.class != ViewClass::Scroll {
            entry.container.set_size(frame.size());
        }
        if let Some(input) = &mut entry.input {
            input.layout(frame.size(), &entry.container);
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
        if let Some(entry) = self.views.get_mut(&id) {
            entry.container.set_text(text);
            // The text is the accessible name unless a label was given.
            entry.plain_text = text.map(|content| content.block.runs.iter().map(|run| run.text.as_str()).collect());
            crate::accessibility::apply(entry.container.upcast_ref(), entry.accessibility.as_deref(), entry.plain_text.as_deref(), &mut entry.announced, false);
        }
    }

    fn set_accessibility(&mut self, id: NodeId, accessibility: Option<&pam_desktop_ui::accessibility::Accessibility>) {
        if let Some(entry) = self.views.get_mut(&id) {
            entry.accessibility = accessibility.cloned().map(Box::new);
            // Native text controls describe themselves; only the name and state are added.
            let target: gtk::Widget = match &entry.input {
                Some(input) => input.focus_target(),
                None => entry.container.clone().upcast(),
            };
            crate::accessibility::apply(&target, accessibility, entry.plain_text.as_deref(), &mut entry.announced, entry.input.is_some());
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
            Some(entry) => match &entry.input {
                Some(input) => {
                    input.focus_target().grab_focus();
                }
                None => {
                    entry.container.grab_focus();
                }
            },
            None => {
                for window in self.windows.values() {
                    gtk::prelude::GtkWindowExt::set_focus(&window.window, None::<&gtk::Widget>);
                }
            }
        }
    }

    fn set_input_config(&mut self, id: NodeId, config: &InputConfig) {
        if let Some(entry) = self.views.get_mut(&id) {
            if let Some(input) = &mut entry.input {
                input.configure(config, &entry.container);
            }
        }
    }

    fn set_input_text(&mut self, id: NodeId, text: &str, selection: Option<(u32, u32)>) {
        if let Some(input) = self.views.get(&id).and_then(|entry| entry.input.as_ref()) {
            input.set_text(text, selection);
        }
    }

    fn set_input_selection(&mut self, id: NodeId, start: u32, end: u32) {
        if let Some(input) = self.views.get(&id).and_then(|entry| entry.input.as_ref()) {
            input.set_selection(start, end);
        }
    }

    fn load_image(&mut self, key: ImageKey, source: ImageSource, max_dimension: Option<u32>) {
        let events = Rc::clone(&self.events);
        let textures = Rc::clone(&self.textures);
        // Decoding runs on a worker thread; only plain pixels cross back to
        // the UI thread, where they become a texture without another copy.
        let (sender, receiver) = std::sync::mpsc::channel::<Result<DecodedImage, String>>();
        let spawned = std::thread::Builder::new().name("pam-image-decode".into()).spawn(move || {
            let _ = sender.send(decode_image(source, max_dimension));
            glib::MainContext::default().wakeup();
        });
        if spawned.is_err() {
            return self.events.push(BackendEvent::ImageFailed { key, message: "image decoder could not start".into() });
        }
        glib::idle_add_local_full(glib::Priority::DEFAULT, move || match receiver.try_recv() {
            Ok(Ok(decoded)) => {
                let format = if decoded.alpha { gdk::MemoryFormat::R8g8b8a8 } else { gdk::MemoryFormat::R8g8b8 };
                let texture = gdk::MemoryTexture::new(decoded.width, decoded.height, format, &decoded.pixels, decoded.stride);
                textures.borrow_mut().insert(key, texture.upcast());
                events.push(BackendEvent::ImageLoaded { key, size: Size::new(decoded.width as f32, decoded.height as f32) });
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

    fn set_drawing(&mut self, id: NodeId, commands: &[pam_desktop_ui::backend::DrawCommand]) {
        if let Some(entry) = self.views.get(&id) {
            entry.container.set_drawing(commands);
        }
    }

    fn set_scroll_content(&mut self, id: NodeId, size: Size) {
        if std::env::var_os("PAM_DESKTOP_DEBUG").is_some() {
            eprintln!("scroll content {id} {size:?}");
        }
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

    fn clipboard_set_text(&mut self, text: &str) {
        if let Some(display) = gdk::Display::default() {
            display.clipboard().set_text(text);
        }
    }

    fn clipboard_text(&mut self, done: Box<dyn FnOnce(Option<String>)>) {
        let Some(display) = gdk::Display::default() else { return done(None) };
        display.clipboard().read_text_async(gio::Cancellable::NONE, move |result| {
            done(result.ok().flatten().map(|text| text.to_string()));
        });
    }

    fn show_alert(&mut self, window: WindowId, alert: &AlertConfig, done: Box<dyn FnOnce(Option<usize>)>) {
        let labels: Vec<&str> =
            if alert.buttons.is_empty() { vec!["OK"] } else { alert.buttons.iter().map(|button| button.label.as_str()).collect() };
        let dialog = gtk::AlertDialog::builder().modal(true).message(alert.title.as_str()).detail(alert.message.as_str()).buttons(labels).build();
        if let Some(cancel) = alert.buttons.iter().position(|button| button.style == AlertButtonStyle::Cancel) {
            dialog.set_cancel_button(cancel as i32);
        }
        // The first non-cancel button answers to Enter.
        let default = alert.buttons.iter().position(|button| button.style != AlertButtonStyle::Cancel).unwrap_or(0);
        dialog.set_default_button(default as i32);
        let parent = self.windows.get(&window).map(|entry| entry.window.clone());
        dialog.choose(parent.as_ref(), gio::Cancellable::NONE, move |result| {
            done(result.ok().and_then(|index| usize::try_from(index).ok()));
        });
    }

    fn set_appearance(&mut self, dark: Option<bool>) {
        let Some(settings) = gtk::Settings::default() else { return };
        match dark {
            Some(dark) => settings.set_gtk_application_prefer_dark_theme(dark),
            None => settings.reset_property("gtk-application-prefer-dark-theme"),
        }
    }

    fn set_menu_bar(&mut self, menu: &[MenuNode]) {
        let window = self.windows.get(&pam_desktop_ui::backend::MAIN_WINDOW).map(|entry| entry.window.clone());
        self.shell.set_menu_bar(menu, window.as_ref(), pam_desktop_ui::backend::MAIN_WINDOW);
    }

    fn popup_menu(&mut self, window: WindowId, position: Point, menu: &[MenuNode], done: Box<dyn FnOnce(Option<String>)>) {
        match self.windows.get(&window) {
            Some(entry) => self.shell.popup_menu(&entry.host, position, menu, done),
            None => done(None),
        }
    }

    /// GTK 4 has no status icon; the tray needs a StatusNotifierItem service.
    fn set_tray(&mut self, _tray: Option<&TrayConfig>) {}

    fn pick_files(&mut self, window: WindowId, config: &FileDialogConfig, done: Box<dyn FnOnce(Vec<std::path::PathBuf>)>) {
        let parent = self.windows.get(&window).map(|entry| entry.window.clone());
        self.shell.pick_files(parent.as_ref(), config, done);
    }

    fn notify(&mut self, notification: &NotificationConfig) {
        self.shell.notify(notification);
    }

    fn window_command(&mut self, window: WindowId, command: &WindowCommand) {
        if let Some(entry) = self.windows.get(&window) {
            shell::window_command(&entry.window, &entry.host, command);
        }
    }

    fn window_state(&self, window: WindowId) -> WindowState {
        self.windows.get(&window).map(|entry| shell::window_state(&entry.window)).unwrap_or_default()
    }

    /// Shortcuts of the focused window are matched by the core; global ones
    /// need the desktop portal and are not registered here.
    fn set_shortcuts(&mut self, _shortcuts: &[Shortcut]) {}

    fn set_cursor(&mut self, id: NodeId, cursor: Cursor) {
        if let Some(entry) = self.views.get(&id) {
            entry.widget.set_cursor_from_name(shell::cursor_name(cursor));
        }
    }

    fn set_tooltip(&mut self, id: NodeId, text: Option<&str>) {
        if let Some(entry) = self.views.get(&id) {
            entry.widget.set_tooltip_text(text);
        }
    }

    fn begin_window_drag(&mut self, window: WindowId) {
        let Some(entry) = self.windows.get(&window) else { return };
        let Some((device, button, x, y, time)) = self.last_press.borrow().clone() else { return };
        if let Some(toplevel) = entry.window.surface().and_then(|surface| surface.downcast::<gdk::Toplevel>().ok()) {
            toplevel.begin_move(&device, button, x, y, time);
        }
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

/// Pixels of a decoded image, ready to wrap in a texture.
struct DecodedImage {
    width: i32,
    height: i32,
    stride: usize,
    alpha: bool,
    pixels: glib::Bytes,
}

/// Decodes on the calling (worker) thread. With `max_dimension` the decoder
/// is told the target size up front, so large photos never exist at full
/// size in memory.
fn decode_image(source: ImageSource, max_dimension: Option<u32>) -> Result<DecodedImage, String> {
    use gtk::gdk_pixbuf::prelude::*;
    let loader = gtk::gdk_pixbuf::PixbufLoader::new();
    if let Some(limit) = max_dimension.and_then(|limit| i32::try_from(limit).ok()).filter(|limit| *limit > 0) {
        loader.connect_size_prepared(move |loader, width, height| {
            let longest = width.max(height);
            if longest > limit {
                let ratio = f64::from(limit) / f64::from(longest);
                loader.set_size(((f64::from(width) * ratio).round() as i32).max(1), ((f64::from(height) * ratio).round() as i32).max(1));
            }
        });
    }
    let written = match &source {
        ImageSource::Bytes(bytes) => loader.write(bytes),
        ImageSource::File(path) => match std::fs::read(path) {
            Ok(bytes) => loader.write(&bytes),
            Err(error) => {
                let _ = loader.close();
                return Err(format!("Image file could not be read: {error}"));
            }
        },
    };
    let closed = loader.close();
    written.and(closed).map_err(|error| error.to_string())?;
    let pixbuf = loader.pixbuf().ok_or_else(|| String::from("Image could not be decoded"))?;
    if pixbuf.bits_per_sample() != 8 || pixbuf.width() <= 0 || pixbuf.height() <= 0 {
        return Err("Image format is not supported".into());
    }
    Ok(DecodedImage {
        width: pixbuf.width(),
        height: pixbuf.height(),
        stride: usize::try_from(pixbuf.rowstride()).unwrap_or(0),
        alpha: pixbuf.has_alpha(),
        pixels: pixbuf.read_pixel_bytes(),
    })
}
