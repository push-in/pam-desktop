//! `Backend` implementation on GTK 4.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::process::ExitCode;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib, graphene, pango};
use pam_desktop_ui::backend::{
    Backend, BackendEvent, Modifiers, NodeId, PointerButton, PointerEvent, PointerPhase, TextContent, ViewClass,
    WindowConfig, WindowId, WindowMetrics,
};
use pam_desktop_ui::geometry::{Point, Rect, Size};
use pam_desktop_ui::runtime::{self, EventQueue, Inbox, Runtime, RuntimeConfig, Shared};
use pam_desktop_ui::scene::IdMap;
use pam_desktop_ui::style::{Decoration, LayerState};
use pam_desktop_ui::text::{TextBlock, TextMetrics};

use crate::text;
use crate::view::{self, PamView};

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
}

pub struct GtkBackend {
    application: gtk::Application,
    events: Rc<EventQueue>,
    views: IdMap<ViewEntry>,
    windows: HashMap<WindowId, WindowEntry>,
    measure_layout: Option<pango::Layout>,
    frame_requested: Rc<Cell<bool>>,
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
                _ => PointerButton::Primary,
            };
            let state = event.modifier_state();
            events.push(BackendEvent::Pointer(PointerEvent {
                window,
                phase,
                position,
                button,
                modifiers: Modifiers {
                    shift: state.contains(gdk::ModifierType::SHIFT_MASK),
                    control: state.contains(gdk::ModifierType::CONTROL_MASK),
                    alt: state.contains(gdk::ModifierType::ALT_MASK),
                    meta: state.contains(gdk::ModifierType::META_MASK) || state.contains(gdk::ModifierType::SUPER_MASK),
                },
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
        self.windows.insert(window, WindowEntry { window: native, host, size, presented: false });
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
        while let Some(child) = entry.host.first_child() {
            child.unparent();
        }
        if let Some(view) = root.and_then(|root| self.views.get(&root)) {
            if view.widget.parent().is_some() {
                view.widget.unparent();
            }
            view.widget.set_parent(&entry.host);
        }
        entry.host.queue_allocate();
        if !entry.presented {
            entry.presented = true;
            entry.window.present();
        }
    }

    fn create_view(&mut self, id: NodeId, class: ViewClass) {
        let container = PamView::new();
        let widget: gtk::Widget = match class {
            ViewClass::Scroll => {
                let scrolled = gtk::ScrolledWindow::new();
                scrolled.set_child(Some(&container));
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

    fn quit(&mut self) {
        for (_, entry) in self.windows.drain() {
            entry.window.destroy();
        }
        self.application.quit();
    }
}

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
