//! `Backend` implementation on GTK 4.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::process::ExitCode;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib, graphene, pango};
use pam_desktop_ui::backend::{
    AlertButtonStyle, AlertConfig, Backend, BackendEvent, ImageContent, ImageKey, ImageSource,
    InputConfig, KeyEvent, Modifiers, NodeId, PointerButton, PointerEvent, PointerPhase,
    ScrollConfig, TextContent, ViewClass, WindowConfig, WindowId, WindowMetrics,
};
use pam_desktop_ui::geometry::{Point, Rect, Size};
use pam_desktop_ui::runtime::{self, EventQueue, Inbox, Runtime, RuntimeConfig, Shared};
use pam_desktop_ui::scene::IdMap;
use pam_desktop_ui::shell::{
    Cursor, FileDialogConfig, MenuNode, NotificationConfig, Shortcut, TrayConfig, WindowCommand,
    WindowState,
};
use pam_desktop_ui::style::{Decoration, LayerState};
use pam_desktop_ui::text::{TextBlock, TextMetrics};

use crate::input::InputHost;
use crate::shell::{self, Shell};
use crate::text;
use crate::view::{self, ImageState, PamView};

struct ViewEntry {
    parent: NodeId,
    target_self: bool,
    target_children: bool,
    /// The widget placed in the parent.
    widget: gtk::Widget,
    /// Where child node widgets are added.
    container: PamView,
    class: ViewClass,
    /// The native text control of input classes.
    input: Option<InputHost>,
    /// An offset asked for before the content was laid out that far.
    pending_scroll: Rc<Cell<Option<Point>>>,
    /// The player of a media view.
    media: Option<crate::media::MediaHost>,
    web: Option<crate::webview::WebHost>,
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

/// A button press: device, button, position and time.
type Press = (gdk::Device, i32, f64, f64, u32);

pub struct GtkBackend {
    application: gtk::Application,
    events: Rc<EventQueue>,
    views: IdMap<ViewEntry>,
    key_captures: Rc<RefCell<HashMap<usize, String>>>,
    windows: HashMap<WindowId, WindowEntry>,
    measure_layout: Option<pango::Layout>,
    frame_requested: Rc<Cell<bool>>,
    /// A tick callback is installed on the main window's frame clock.
    frame_ticking: Rc<Cell<bool>>,
    textures: Rc<RefCell<HashMap<ImageKey, gdk::Texture>>>,
    wake_source: Option<glib::SourceId>,
    /// Set when the pending wake already ran (its source id is then stale).
    wake_fired: Rc<Cell<bool>>,
    shell: Shell,
    /// Reports the desktop's appearance, text size and motion changes.
    _system: crate::system::Watch,
    /// The application overrides the appearance of the desktop.
    forced_appearance: bool,
    /// The last button press, needed to start a window move.
    last_press: Rc<RefCell<Option<Press>>>,
    #[cfg(feature = "tray")]
    tray: Option<crate::tray::TrayHost>,
    #[cfg(feature = "hotkeys")]
    hotkeys: Option<crate::hotkeys::Hotkeys>,
}

impl GtkBackend {
    #[must_use]
    pub fn new(application: gtk::Application, events: Rc<EventQueue>) -> Self {
        let shell = Shell::new(application.clone(), Rc::clone(&events));
        let system = crate::system::Watch::new(&events);
        Self {
            application,
            events,
            views: IdMap::default(),
            key_captures: Rc::new(RefCell::new(HashMap::new())),
            windows: HashMap::new(),
            measure_layout: None,
            frame_requested: Rc::new(Cell::new(false)),
            frame_ticking: Rc::new(Cell::new(false)),
            textures: Rc::new(RefCell::new(HashMap::new())),
            wake_source: None,
            wake_fired: Rc::new(Cell::new(false)),
            shell,
            _system: system,
            forced_appearance: false,
            last_press: Rc::new(RefCell::new(None)),
            #[cfg(feature = "tray")]
            tray: None,
            #[cfg(feature = "hotkeys")]
            hotkeys: None,
        }
    }

    fn modifiers(state: gdk::ModifierType) -> Modifiers {
        Modifiers {
            shift: state.contains(gdk::ModifierType::SHIFT_MASK),
            control: state.contains(gdk::ModifierType::CONTROL_MASK),
            alt: state.contains(gdk::ModifierType::ALT_MASK),
            meta: state.contains(gdk::ModifierType::META_MASK)
                || state.contains(gdk::ModifierType::SUPER_MASK),
        }
    }

    /// Wheel input no scrolling view took, and the touchpad's pinch and
    /// rotation gestures.
    fn install_gesture_events(&self, window: WindowId, host: &PamView) {
        use pam_desktop_ui::backend::GesturePhase;
        // One notch of a wheel in logical units.
        const NOTCH: f32 = 100.0;
        let pointer = Rc::new(Cell::new(Point::ZERO));
        {
            let motion = gtk::EventControllerMotion::new();
            motion.set_propagation_phase(gtk::PropagationPhase::Capture);
            let pointer = Rc::clone(&pointer);
            motion.connect_motion(move |_, x, y| pointer.set(Point::new(x as f32, y as f32)));
            host.add_controller(motion);
        }
        for phase in [
            gtk::PropagationPhase::Capture,
            gtk::PropagationPhase::Bubble,
        ] {
            let scroll =
                gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
            scroll.set_propagation_phase(phase);
            let (events, pointer) = (Rc::clone(&self.events), Rc::clone(&pointer));
            scroll.connect_scroll(move |controller, dx, dy| {
                let modifiers = GtkBackend::modifiers(controller.current_event_state());
                // Control + wheel zooms and never scrolls: it is taken before
                // any scrolling view sees it. Plain wheel input is only
                // reported when it bubbled up unused.
                if (phase == gtk::PropagationPhase::Capture) != modifiers.control {
                    return glib::Propagation::Proceed;
                }
                let scale = if crate::compat::scrolls_in_wheel_steps(controller) {
                    NOTCH
                } else {
                    1.0
                };
                let delta = Point::new(dx as f32 * scale, dy as f32 * scale);
                events.push(BackendEvent::Wheel {
                    window,
                    position: pointer.get(),
                    delta,
                    modifiers,
                });
                if modifiers.control {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });
            host.add_controller(scroll);
        }
        let center = |gesture: &gtk::Gesture, fallback: Point| {
            gesture
                .bounding_box_center()
                .map_or(fallback, |(x, y)| Point::new(x as f32, y as f32))
        };
        {
            let zoom = gtk::GestureZoom::new();
            let (events, tracked) = (Rc::clone(&self.events), Rc::clone(&pointer));
            zoom.connect_begin(move |gesture, _| {
                events.push(BackendEvent::Magnify {
                    window,
                    position: center(gesture.upcast_ref(), tracked.get()),
                    phase: GesturePhase::Begin,
                    scale: 1.0,
                });
            });
            let (events, tracked) = (Rc::clone(&self.events), Rc::clone(&pointer));
            zoom.connect_scale_changed(move |gesture, scale| {
                events.push(BackendEvent::Magnify {
                    window,
                    position: center(gesture.upcast_ref(), tracked.get()),
                    phase: GesturePhase::Update,
                    scale: scale as f32,
                });
            });
            let (events, tracked) = (Rc::clone(&self.events), Rc::clone(&pointer));
            zoom.connect_end(move |gesture, _| {
                events.push(BackendEvent::Magnify {
                    window,
                    position: center(gesture.upcast_ref(), tracked.get()),
                    phase: GesturePhase::End,
                    scale: 1.0,
                });
            });
            host.add_controller(zoom);
        }
        {
            let rotate = gtk::GestureRotate::new();
            let (events, tracked) = (Rc::clone(&self.events), Rc::clone(&pointer));
            rotate.connect_begin(move |gesture, _| {
                events.push(BackendEvent::Rotate {
                    window,
                    position: center(gesture.upcast_ref(), tracked.get()),
                    phase: GesturePhase::Begin,
                    radians: 0.0,
                });
            });
            let (events, tracked) = (Rc::clone(&self.events), Rc::clone(&pointer));
            rotate.connect_angle_changed(move |gesture, _, delta| {
                events.push(BackendEvent::Rotate {
                    window,
                    position: center(gesture.upcast_ref(), tracked.get()),
                    phase: GesturePhase::Update,
                    radians: delta as f32,
                });
            });
            let (events, tracked) = (Rc::clone(&self.events), Rc::clone(&pointer));
            rotate.connect_end(move |gesture, _| {
                events.push(BackendEvent::Rotate {
                    window,
                    position: center(gesture.upcast_ref(), tracked.get()),
                    phase: GesturePhase::End,
                    radians: 0.0,
                });
            });
            host.add_controller(rotate);
        }
    }

    /// Keys nobody consumed (bubble phase), as W3C `key` values.
    fn install_key_events(&self, window: WindowId, native: &gtk::ApplicationWindow) {
        let capture = gtk::EventControllerKey::new();
        capture.set_propagation_phase(gtk::PropagationPhase::Capture);
        let captures = Rc::clone(&self.key_captures);
        let events = Rc::clone(&self.events);
        let host = native.downgrade();
        capture.connect_key_pressed(move |_, key, _, state| {
            let modifiers = GtkBackend::modifiers(state);
            if modifiers.control || modifiers.alt || modifiers.meta {
                return glib::Propagation::Proceed;
            }
            let name = key_name(key);
            let wanted = if name == " " { "Space" } else { name.as_str() };
            let mut widget = host
                .upgrade()
                .and_then(|window| gtk::prelude::GtkWindowExt::focus(&window));
            while let Some(target) = widget {
                let owned = captures
                    .borrow()
                    .get(&(target.as_ptr() as usize))
                    .is_some_and(|keys| {
                        keys.split(',').any(|candidate| candidate.trim() == wanted)
                    });
                if owned {
                    events.push(BackendEvent::Key(KeyEvent {
                        window,
                        pressed: true,
                        key: name,
                        modifiers,
                        repeat: false,
                    }));
                    return glib::Propagation::Stop;
                }
                widget = target.parent();
            }
            glib::Propagation::Proceed
        });
        native.add_controller(capture);
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
            let owned = widget
                .parent()
                .is_some_and(|parent| parent == *entry.host.upcast_ref::<gtk::Widget>());
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
        // Time, position, button and count of the last press, to tell double
        // and triple clicks with the system's own thresholds.
        let clicks = Cell::new((0_u32, Point::ZERO, PointerButton::Primary, 0_u8));
        controller.connect_event(move |_, event| {
            let Some(host) = target.upgrade() else {
                return glib::Propagation::Proceed;
            };
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
            let Some(position) =
                position.or_else(|| (phase == PointerPhase::Leave).then_some(Point::ZERO))
            else {
                return glib::Propagation::Proceed;
            };
            let button = match event
                .downcast_ref::<gdk::ButtonEvent>()
                .map(gdk::ButtonEvent::button)
            {
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
            if phase == PointerPhase::Down
                && let (Some(device), Some((x, y)), Some(press)) = (
                    event.device(),
                    event.position(),
                    event.downcast_ref::<gdk::ButtonEvent>(),
                )
            {
                *last_press.borrow_mut() =
                    Some((device, press.button() as i32, x, y, event.time()));
            }
            let state = event.modifier_state();
            let click_count = match phase {
                PointerPhase::Down => {
                    let (time, place, last_button, count) = clicks.get();
                    let settings = gtk::Settings::default();
                    let interval = settings.as_ref().map_or(400, |settings| {
                        settings.gtk_double_click_time().max(0) as u32
                    });
                    let distance = settings
                        .as_ref()
                        .map_or(5, |settings| settings.gtk_double_click_distance().max(0))
                        as f32;
                    let repeated = count > 0
                        && last_button == button
                        && event.time().wrapping_sub(time) <= interval
                        && (position.x - place.x).abs() <= distance
                        && (position.y - place.y).abs() <= distance;
                    let count = if repeated { count.saturating_add(1) } else { 1 };
                    clicks.set((event.time(), position, button, count));
                    count
                }
                PointerPhase::Up => clicks.get().3.max(1),
                _ => 1,
            };
            events.push(BackendEvent::Pointer(PointerEvent {
                window,
                phase,
                position,
                button,
                modifiers: GtkBackend::modifiers(state),
                click_count,
                time_ms: u64::from(event.time()),
            }));
            glib::Propagation::Proceed
        });
        host.add_controller(controller);
    }
}

impl Backend for GtkBackend {
    fn set_asset_root(&mut self, root: &std::path::Path) {
        crate::fonts::set_root(root);
    }

    fn open_window(&mut self, window: WindowId, config: &WindowConfig) {
        if let Some(entry) = self.windows.get(&window) {
            entry.window.set_title(Some(&config.title));
            entry.window.set_resizable(config.resizable);
            entry.window.set_modal(config.modal);
            // A new size from the application resizes a secondary window.
            if window != pam_desktop_ui::backend::MAIN_WINDOW && entry.size.get() != config.size {
                entry.window.set_default_size(
                    config.size.width.round() as i32,
                    config.size.height.round() as i32,
                );
            }
            return;
        }
        let native = gtk::ApplicationWindow::new(&self.application);
        native.set_title(Some(&config.title));
        native.set_default_size(
            config.size.width.round() as i32,
            config.size.height.round() as i32,
        );
        native.set_resizable(config.resizable);
        native.set_decorated(config.decorated);
        // The menu bar belongs to the main window.
        native.set_show_menubar(window == pam_desktop_ui::backend::MAIN_WINDOW);
        if let Some(parent) = config.parent.and_then(|parent| self.windows.get(&parent)) {
            native.set_transient_for(Some(&parent.window));
            native.set_destroy_with_parent(true);
            native.set_modal(config.modal);
        }
        if let Some(minimum) = config.min_size {
            native.set_size_request(minimum.width.round() as i32, minimum.height.round() as i32);
        }

        let host = PamView::new();
        if std::env::var("PAM_DESKTOP_PROFILE").as_deref() == Ok("1") {
            native.connect_realize(|window| {
                if let Some(clock) = window.frame_clock() {
                    clock.connect_after_paint(|_| {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |elapsed| elapsed.as_millis() % 100_000);
                        if let Some(report) = crate::prof::report() {
                            eprintln!("[pam-desktop profile] @{now} paint after {report}");
                        }
                    });
                }
            });
        }
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
                    let scale = weak
                        .upgrade()
                        .map_or(1.0, |host| host.scale_factor() as f32);
                    // Allocation runs inside GTK's layout pass: apply the new
                    // viewport right after it instead of re-entering layout.
                    let events = Rc::clone(&events);
                    glib::idle_add_local_full(glib::Priority::HIGH, move || {
                        events.push(BackendEvent::WindowResized {
                            window,
                            size: next,
                            scale,
                        });
                        glib::ControlFlow::Break
                    });
                }));
            });
        }
        native.set_child(Some(&host));
        self.install_pointer_events(window, &host);
        self.install_gesture_events(window, &host);
        self.install_key_events(window, &native);
        crate::trim::watch(&native);
        shell::accept_file_drops(&host, window, &self.events);
        shell::accept_text_drops(&host, window, &self.events);
        shell::watch_window_state(&native, window, &self.events);

        let events = Rc::clone(&self.events);
        native.connect_close_request(move |_| {
            events.push(BackendEvent::WindowCloseRequested { window });
            glib::Propagation::Stop
        });
        let events = Rc::clone(&self.events);
        native.connect_is_active_notify(move |native| {
            events.push(BackendEvent::WindowFocusChanged {
                window,
                focused: native.is_active(),
            });
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
            let layout = host.create_pango_layout(None);
            text::use_subpixel_positions(&layout);
            self.measure_layout = Some(layout);
        }
        self.windows.insert(
            window,
            WindowEntry {
                window: native,
                host,
                size,
                presented: false,
                root: None,
                overlays: Vec::new(),
            },
        );
    }

    fn close_window(&mut self, window: WindowId) {
        if let Some(entry) = self.windows.remove(&window) {
            entry.window.destroy();
        }
    }

    fn window_metrics(&self, window: WindowId) -> WindowMetrics {
        let Some(entry) = self.windows.get(&window) else {
            return WindowMetrics::default();
        };
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
            font_scale: crate::system::font_scale(),
            dark: crate::system::prefers_dark(self.forced_appearance),
            refresh_rate,
            reduced_motion: crate::system::reduced_motion(),
        }
    }

    fn set_window_root(&mut self, window: WindowId, root: Option<NodeId>) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        entry.root = root;
        Self::arrange_window(entry, &self.views);
        if root.is_some() && !entry.presented {
            entry.presented = true;
            entry.window.present();
        }
    }

    fn set_window_overlays(&mut self, window: WindowId, overlays: &[NodeId]) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        entry.overlays = overlays.to_vec();
        Self::arrange_window(entry, &self.views);
        // A boot failure can produce host chrome before any application root.
        if !overlays.is_empty() && !entry.presented {
            entry.presented = true;
            entry.window.present();
        }
    }

    fn create_view(&mut self, id: NodeId, class: ViewClass) {
        let _span = crate::prof::Span::new("create_view");
        let container = PamView::new();
        {
            let focus = gtk::EventControllerFocus::new();
            if matches!(class, ViewClass::Input | ViewClass::TextArea) {
                // Text focus belongs to GTK's inner editor, represented by this input node.
                let events = Rc::clone(&self.events);
                focus.connect_enter(move |_| {
                    events.push(BackendEvent::FocusChanged { id, focused: true })
                });
                let events = Rc::clone(&self.events);
                focus.connect_leave(move |_| {
                    events.push(BackendEvent::FocusChanged { id, focused: false })
                });
            } else {
                // `enter` includes descendants: reporting every ancestor steals the
                // focused node from pressables and sends keys to unrelated controls.
                let events = Rc::clone(&self.events);
                focus.connect_is_focus_notify(move |controller| {
                    events.push(BackendEvent::FocusChanged {
                        id,
                        focused: controller.is_focus(),
                    })
                });
            }
            container.add_controller(focus);
        }
        let pending_scroll: Rc<Cell<Option<Point>>> = Rc::new(Cell::new(None));
        let widget: gtk::Widget = match class {
            ViewClass::Scroll => {
                let scrolled = gtk::ScrolledWindow::new();
                // An offset beyond what is laid out lands when the content grows.
                for adjustment in [scrolled.hadjustment(), scrolled.vadjustment()] {
                    let (pending, weak) = (Rc::clone(&pending_scroll), scrolled.downgrade());
                    adjustment.connect_changed(move |_| {
                        if pending.get().is_none() {
                            return;
                        }
                        // GTK is still configuring the adjustment here and
                        // would overwrite a value set now: apply it right after.
                        let (pending, weak) = (Rc::clone(&pending), weak.clone());
                        glib::idle_add_local_full(glib::Priority::HIGH, move || {
                            let (Some(target), Some(scrolled)) = (pending.get(), weak.upgrade())
                            else {
                                return glib::ControlFlow::Break;
                            };
                            let reach = |adjustment: &gtk::Adjustment| {
                                (adjustment.upper() - adjustment.page_size()).max(0.0)
                            };
                            let (horizontal, vertical) =
                                (scrolled.hadjustment(), scrolled.vadjustment());
                            if f64::from(target.x) <= reach(&horizontal) + 0.5
                                && f64::from(target.y) <= reach(&vertical) + 0.5
                            {
                                pending.set(None);
                                horizontal.set_value(f64::from(target.x));
                                vertical.set_value(f64::from(target.y));
                            }
                            glib::ControlFlow::Break
                        });
                    });
                }
                // The engine sizes the viewport; content never dictates it.
                scrolled.set_policy(gtk::PolicyType::External, gtk::PolicyType::Automatic);
                scrolled.set_propagate_natural_width(false);
                scrolled.set_propagate_natural_height(false);
                scrolled.set_child(Some(&container));
                container.cache_rows(&scrolled);
                let report = {
                    let events = Rc::clone(&self.events);
                    let weak = scrolled.downgrade();
                    move || {
                        if let Some(scrolled) = weak.upgrade() {
                            let offset = Point::new(
                                scrolled.hadjustment().value() as f32,
                                scrolled.vadjustment().value() as f32,
                            );
                            events.push(BackendEvent::ScrollChanged { id, offset });
                        }
                    }
                };
                let report_horizontal = report.clone();
                scrolled
                    .hadjustment()
                    .connect_value_changed(move |_| report_horizontal());
                scrolled
                    .vadjustment()
                    .connect_value_changed(move |_| report());
                scrolled.upcast()
            }
            _ => container.clone().upcast(),
        };
        let input = match class {
            ViewClass::Input => Some(InputHost::new(id, false, &container, &self.events)),
            ViewClass::TextArea => Some(InputHost::new(id, true, &container, &self.events)),
            _ => None,
        };
        let media = (class == ViewClass::Media)
            .then(|| crate::media::MediaHost::new(id, &container, &self.events));
        let web = (class == ViewClass::WebView)
            .then(|| crate::webview::WebHost::new(id, &container, &self.events))
            .flatten();
        self.views.insert(
            id,
            ViewEntry {
                parent: 0,
                target_self: true,
                target_children: true,
                widget,
                container,
                class,
                input,
                pending_scroll,
                media,
                web,
                accessibility: None,
                plain_text: None,
                announced: String::new(),
            },
        );
    }

    fn destroy_view(&mut self, id: NodeId) {
        let _span = crate::prof::Span::new("destroy_view");
        if let Some(mut entry) = self.views.remove(&id) {
            self.key_captures
                .borrow_mut()
                .remove(&(entry.container.as_ptr() as usize));
            if let Some(input) = &entry.input {
                input.destroy();
            }
            if let Some(media) = &mut entry.media {
                media.destroy();
            }
            if let Some(web) = &mut entry.web {
                web.destroy();
            }
            if entry.widget.parent().is_some() {
                entry.widget.unparent();
            }
            view::forget_layout(&entry.widget);
        }
    }

    fn set_children(&mut self, parent: NodeId, children: &[NodeId]) {
        let _span = crate::prof::Span::new("set_children");
        let Some(parent_entry) = self.views.get(&parent) else {
            return;
        };
        let container = parent_entry.container.clone();
        let target_children = parent_entry.target_children;
        let mut previous: Option<gtk::Widget> = None;
        for child in children {
            let Some(entry) = self.views.get_mut(child) else {
                continue;
            };
            entry.parent = parent;
            entry
                .widget
                .set_can_target(target_children && (entry.target_self || entry.target_children));
            let widget = &entry.widget;
            let owned = widget
                .parent()
                .is_some_and(|current| current == *container.upcast_ref::<gtk::Widget>());
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
        let _span = crate::prof::Span::new("set_frame");
        let Some(entry) = self.views.get_mut(&id) else {
            return;
        };
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

    fn set_pointer_events(&mut self, id: NodeId, target_self: bool, target_children: bool) {
        let _span = crate::prof::Span::new("set_pointer_events");
        let parent_allows = self
            .views
            .get(&id)
            .and_then(|entry| self.views.get(&entry.parent))
            .is_none_or(|parent| parent.target_children);
        let Some(entry) = self.views.get_mut(&id) else {
            return;
        };
        entry.target_self = target_self;
        entry.target_children = target_children;
        view::set_layout(&entry.widget, |layout| {
            layout.pointer_targetable = target_self || target_children
        });
        entry
            .widget
            .set_can_target(parent_allows && (target_self || target_children));
        entry
            .container
            .with_state(|state| state.pointer_passthrough = !target_self);
        // Internal native input widgets also obey box-only/none. PAM children
        // retain their own policy when re-enabled by a changed parent policy.
        let mut child = entry.container.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            let own_allows =
                view::layout_of(&widget).is_none_or(|layout| layout.pointer_targetable);
            widget.set_can_target(target_children && own_allows);
        }
    }

    fn set_decoration(&mut self, id: NodeId, decoration: &Decoration) {
        let _span = crate::prof::Span::new("set_decoration");
        if let Some(entry) = self.views.get(&id) {
            entry.container.set_decoration(decoration);
            if let Some(scrolled) = entry.widget.downcast_ref::<gtk::ScrolledWindow>() {
                style_gutter(scrolled, decoration);
            }
        }
    }

    fn set_layer(&mut self, id: NodeId, layer: &LayerState) {
        let _span = crate::prof::Span::new("set_layer");
        let Some(entry) = self.views.get(&id) else {
            return;
        };
        // GTK does not pick fully transparent widgets, but CSS keeps an
        // `opacity: 0` element clickable (hover-revealed actions). GTK stores
        // opacity as an 8-bit alpha: the lowest step (1/255) is invisible in
        // practice and keeps the hit target.
        entry
            .widget
            .set_opacity(f64::from(pickable_opacity(layer.opacity)));
        entry.widget.set_visible(layer.visible);
        view::set_layout(&entry.widget, |layout| layout.transform = layer.transform);
        if let Some(parent) = entry.widget.parent() {
            parent.queue_allocate();
        }
    }

    fn set_text_geometry(&mut self, id: NodeId, geometry: pam_desktop_ui::backend::TextGeometry) {
        let _span = crate::prof::Span::new("set_text_geometry");
        if let Some(entry) = self.views.get(&id) {
            entry.container.set_text_geometry(geometry);
        }
    }

    fn set_text(&mut self, id: NodeId, text: Option<&TextContent>) {
        let _span = crate::prof::Span::new("set_text");
        if let Some(entry) = self.views.get_mut(&id) {
            entry.container.set_text(text);
            // The text is the accessible name unless a label was given.
            entry.plain_text = text.map(|content| {
                content
                    .block
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect()
            });
            crate::accessibility::apply(
                entry.container.upcast_ref(),
                entry.accessibility.as_deref(),
                entry.plain_text.as_deref(),
                &mut entry.announced,
                false,
            );
        }
    }

    fn set_accessibility(
        &mut self,
        id: NodeId,
        accessibility: Option<&pam_desktop_ui::accessibility::Accessibility>,
    ) {
        let _span = crate::prof::Span::new("set_accessibility");
        if let Some(entry) = self.views.get_mut(&id) {
            entry.accessibility = accessibility.cloned().map(Box::new);
            // Native text controls describe themselves; only the name and state are added.
            let target: gtk::Widget = match &entry.input {
                Some(input) => input.focus_target(),
                None => entry.container.clone().upcast(),
            };
            crate::accessibility::apply(
                &target,
                accessibility,
                entry.plain_text.as_deref(),
                &mut entry.announced,
                entry.input.is_some(),
            );
        }
    }

    fn set_focusable(&mut self, id: NodeId, focusable: bool) {
        let _span = crate::prof::Span::new("set_focusable");
        if let Some(entry) = self.views.get(&id) {
            entry.container.set_focusable(focusable);
            if focusable {
                entry.container.add_css_class("pam-focusable");
            } else {
                entry.container.remove_css_class("pam-focusable");
            }
        }
    }

    fn set_key_capture(&mut self, id: NodeId, keys: &str) {
        let _span = crate::prof::Span::new("set_key_capture");
        if let Some(entry) = self.views.get(&id) {
            let pointer = entry.container.as_ptr() as usize;
            if keys.is_empty() {
                self.key_captures.borrow_mut().remove(&pointer);
            } else {
                self.key_captures
                    .borrow_mut()
                    .insert(pointer, keys.to_owned());
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
        let _span = crate::prof::Span::new("set_input_config");
        if let Some(entry) = self.views.get_mut(&id)
            && let Some(input) = &mut entry.input
        {
            input.configure(config, &entry.container);
        }
    }

    fn set_input_text(&mut self, id: NodeId, text: &str, selection: Option<(u32, u32)>) {
        let _span = crate::prof::Span::new("set_input_text");
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
        let _span = crate::prof::Span::new("load_image");
        let events = Rc::clone(&self.events);
        let textures = Rc::clone(&self.textures);
        // Decoding runs on a worker thread; only plain pixels cross back to
        // the UI thread, where they become a texture without another copy.
        let (sender, receiver) = std::sync::mpsc::channel::<Result<DecodedImage, String>>();
        let spawned = std::thread::Builder::new()
            .name("pam-image-decode".into())
            .spawn(move || {
                let _ = sender.send(decode_image(source, max_dimension));
                glib::MainContext::default().wakeup();
            });
        if spawned.is_err() {
            return self.events.push(BackendEvent::ImageFailed {
                key,
                message: "image decoder could not start".into(),
            });
        }
        glib::idle_add_local_full(glib::Priority::DEFAULT, move || match receiver.try_recv() {
            Ok(Ok(decoded)) => {
                let format = if decoded.alpha {
                    gdk::MemoryFormat::R8g8b8a8
                } else {
                    gdk::MemoryFormat::R8g8b8
                };
                let texture = gdk::MemoryTexture::new(
                    decoded.width,
                    decoded.height,
                    format,
                    &decoded.pixels,
                    decoded.stride,
                );
                textures.borrow_mut().insert(key, texture.upcast());
                events.push(BackendEvent::ImageLoaded {
                    key,
                    size: Size::new(decoded.width as f32, decoded.height as f32),
                });
                glib::ControlFlow::Break
            }
            Ok(Err(message)) => {
                events.push(BackendEvent::ImageFailed { key, message });
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                events.push(BackendEvent::ImageFailed {
                    key,
                    message: "image decoder stopped".into(),
                });
                glib::ControlFlow::Break
            }
        });
    }

    fn release_image(&mut self, key: ImageKey) {
        self.textures.borrow_mut().remove(&key);
    }

    fn set_image(&mut self, id: NodeId, image: Option<&ImageContent>) {
        let _span = crate::prof::Span::new("set_image");
        let Some(entry) = self.views.get(&id) else {
            return;
        };
        let state = image.and_then(|content| {
            let texture = self.textures.borrow().get(&content.key).cloned()?;
            Some(ImageState {
                content: *content,
                texture,
            })
        });
        entry.container.set_image(state);
    }

    fn text_index_at(&self, id: NodeId, point: Point) -> Option<u32> {
        self.views
            .get(&id)?
            .container
            .text_index_at(point.x, point.y)
    }

    fn set_text_selection(&mut self, id: NodeId, range: Option<(u32, u32)>) {
        if let Some(entry) = self.views.get(&id) {
            entry.container.set_text_selection(range);
        }
    }

    fn print_document(
        &mut self,
        window: WindowId,
        document: pam_desktop_ui::printing::Document,
        complete: pam_desktop_ui::printing::PrintCompletion,
    ) {
        crate::printing::print(
            self.windows.get(&window).map(|entry| &entry.window),
            document,
            complete,
        );
    }

    fn announce(&mut self, message: &str, assertive: bool) -> bool {
        let active = self
            .application
            .active_window()
            .and_then(|active| self.windows.values().find(|entry| entry.window == active));
        let Some(entry) = active
            .or_else(|| self.windows.get(&pam_desktop_ui::backend::MAIN_WINDOW))
            .filter(|entry| entry.host.is_realized())
        else {
            return false;
        };
        crate::compat::announce(&entry.host, message, assertive)
            && crate::accessibility::screen_reader_active()
    }

    fn screen_reader_active(&self) -> bool {
        crate::accessibility::screen_reader_active()
    }

    fn composites_backdrop_boxes(&self) -> pam_desktop_ui::backend::BackdropBoxes {
        // One window-sized texture: the topmost dialog's.
        pam_desktop_ui::backend::BackdropBoxes::Topmost
    }

    fn begin_text_drag(&mut self, window: WindowId, source: NodeId, text: &str) {
        let finished = |events: &Rc<EventQueue>, dropped: bool| {
            events.push(BackendEvent::DragFinished { source, dropped })
        };
        let surface = self
            .windows
            .get(&window)
            .and_then(|entry| entry.window.surface());
        let (Some(surface), Some((device, _, x, y, _))) =
            (surface, self.last_press.borrow().clone())
        else {
            return finished(&self.events, false);
        };
        let content = gdk::ContentProvider::for_value(&text.to_value());
        let Some(drag) = gdk::Drag::begin(&surface, &device, &content, gdk::DragAction::COPY, x, y)
        else {
            return finished(&self.events, false);
        };
        // The dragged view follows the pointer.
        if let Some(entry) = self.views.get(&source) {
            let icon = gtk::DragIcon::for_drag(&drag);
            let picture =
                gtk::Picture::for_paintable(&gtk::WidgetPaintable::new(Some(&entry.widget)));
            picture.set_opacity(0.85);
            icon.set_child(Some(&picture));
        }
        let dropped = Rc::new(Cell::new(false));
        {
            let dropped = Rc::clone(&dropped);
            drag.connect_drop_performed(move |_| dropped.set(true));
        }
        {
            let (events, dropped) = (Rc::clone(&self.events), Rc::clone(&dropped));
            drag.connect_dnd_finished(move |_| {
                events.push(BackendEvent::DragFinished {
                    source,
                    dropped: dropped.get(),
                })
            });
        }
        let events = Rc::clone(&self.events);
        drag.connect_cancel(move |_, _| {
            events.push(BackendEvent::DragFinished {
                source,
                dropped: false,
            })
        });
    }

    fn set_media(&mut self, id: NodeId, state: &pam_desktop_ui::backend::MediaState) {
        if let Some(media) = self
            .views
            .get_mut(&id)
            .and_then(|entry| entry.media.as_mut())
        {
            media.apply(state);
        }
    }

    fn set_web_view(&mut self, id: NodeId, state: &pam_desktop_ui::backend::WebViewState) {
        match self.views.get_mut(&id).and_then(|entry| entry.web.as_mut()) {
            Some(web) => web.apply(state),
            // WebKitGTK is not installed.
            None if state.source != pam_desktop_ui::backend::WebSource::None => {
                let event = pam_desktop_ui::backend::WebViewEvent::Failed(
                    crate::webview::UNAVAILABLE.into(),
                );
                self.events.push(BackendEvent::WebView { id, event });
            }
            None => {}
        }
    }

    fn media_position(&self, id: NodeId) -> Option<(f64, f64)> {
        self.views.get(&id)?.media.as_ref()?.position()
    }

    fn set_drawing(&mut self, id: NodeId, commands: &[pam_desktop_ui::backend::DrawCommand]) {
        if let Some(entry) = self.views.get(&id) {
            entry.container.set_drawing(commands);
        }
    }

    fn set_scroll_content(&mut self, id: NodeId, size: Size) {
        let _span = crate::prof::Span::new("set_scroll_content");
        if let Some(entry) = self.views.get(&id) {
            entry.container.set_content_size(Some(size));
        }
    }

    fn set_scroll_config(&mut self, id: NodeId, config: &ScrollConfig) {
        let _span = crate::prof::Span::new("set_scroll_config");
        let Some(scrolled) = self
            .views
            .get(&id)
            .and_then(|entry| entry.widget.downcast_ref::<gtk::ScrolledWindow>())
        else {
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
        if config.both {
            scrolled.set_policy(scrolling, scrolling);
        } else if config.horizontal {
            scrolled.set_policy(scrolling, gtk::PolicyType::External);
        } else {
            scrolled.set_policy(gtk::PolicyType::External, scrolling);
        }
        style_scrollbar(scrolled, config);
        scrolled.set_kinetic_scrolling(config.enabled);
    }

    fn scroll_to(&mut self, id: NodeId, offset: Point, _animated: bool) {
        let Some(entry) = self.views.get(&id) else {
            return;
        };
        let Some(scrolled) = entry.widget.downcast_ref::<gtk::ScrolledWindow>() else {
            return;
        };
        let reach =
            |adjustment: &gtk::Adjustment| (adjustment.upper() - adjustment.page_size()).max(0.0);
        let (horizontal, vertical) = (scrolled.hadjustment(), scrolled.vadjustment());
        // Content that is not laid out yet cannot be scrolled to: remember
        // the offset and apply it when the adjustments grow.
        let reachable = f64::from(offset.x) <= reach(&horizontal) + 0.5
            && f64::from(offset.y) <= reach(&vertical) + 0.5;
        entry.pending_scroll.set((!reachable).then_some(offset));
        horizontal.set_value(f64::from(offset.x));
        vertical.set_value(f64::from(offset.y));
    }

    fn measure_text(&mut self, block: &TextBlock, max_width: f32) -> TextMetrics {
        let _span = crate::prof::Span::new("measure_text");
        match &self.measure_layout {
            Some(layout) => text::measure(layout, block, max_width),
            None => TextMetrics::default(),
        }
    }

    fn text_control_metrics(
        &mut self,
        block: &TextBlock,
    ) -> Option<pam_desktop_ui::text::CharMetrics> {
        let _span = crate::prof::Span::new("text_control_metrics");
        text::control_metrics(self.measure_layout.as_ref()?, block)
    }

    fn request_frame(&mut self) {
        self.frame_requested.set(true);
        if self.frame_ticking.get() {
            return;
        }
        let Some(entry) = self.windows.values().next() else {
            self.frame_requested.set(false);
            return;
        };
        // One tick callback stays installed while frames keep being asked
        // for. Installing one per frame put the frame clock back into its
        // "first frame of an animation" state each time: every frame was
        // timed from the late start of the previous one, and the cadence
        // slipped from 16.7 ms to 18-19 ms (about 53 frames per second).
        self.frame_ticking.set(true);
        let events = Rc::clone(&self.events);
        let requested = Rc::clone(&self.frame_requested);
        let ticking = Rc::clone(&self.frame_ticking);
        entry.host.add_tick_callback(move |_, clock| {
            if !requested.replace(false) {
                ticking.set(false);
                return glib::ControlFlow::Break;
            }
            events.push(BackendEvent::Frame {
                time_ms: u64::try_from(clock.frame_time() / 1000).unwrap_or(0),
            });
            glib::ControlFlow::Continue
        });
    }

    fn clipboard_set_text(&mut self, text: &str) {
        if let Some(display) = gdk::Display::default() {
            display.clipboard().set_text(text);
        }
    }

    fn clipboard_text(&mut self, done: Box<dyn FnOnce(Option<String>)>) {
        let Some(display) = gdk::Display::default() else {
            return done(None);
        };
        display
            .clipboard()
            .read_text_async(gio::Cancellable::NONE, move |result| {
                done(result.ok().flatten().map(|text| text.to_string()));
            });
    }

    fn show_alert(
        &mut self,
        window: WindowId,
        alert: &AlertConfig,
        done: Box<dyn FnOnce(Option<usize>)>,
    ) {
        let labels: Vec<&str> = if alert.buttons.is_empty() {
            vec![pam_desktop_ui::i18n::text(pam_desktop_ui::i18n::Text::Ok)]
        } else {
            alert
                .buttons
                .iter()
                .map(|button| button.label.as_str())
                .collect()
        };
        let alert = crate::compat::Alert {
            message: &alert.title,
            detail: &alert.message,
            buttons: labels,
            cancel: alert
                .buttons
                .iter()
                .position(|button| button.style == AlertButtonStyle::Cancel),
            // The first non-cancel button answers to Enter.
            default: alert
                .buttons
                .iter()
                .position(|button| button.style != AlertButtonStyle::Cancel)
                .unwrap_or(0),
        };
        let parent = self.windows.get(&window).map(|entry| entry.window.clone());
        crate::compat::choose(
            parent.as_ref().map(|parent| parent.upcast_ref()),
            &alert,
            done,
        );
    }

    fn set_appearance(&mut self, dark: Option<bool>) {
        let Some(settings) = gtk::Settings::default() else {
            return;
        };
        self.forced_appearance = dark.is_some();
        match dark {
            Some(dark) => settings.set_gtk_application_prefer_dark_theme(dark),
            None => settings.reset_property("gtk-application-prefer-dark-theme"),
        }
    }

    fn set_menu_bar(&mut self, menu: &[MenuNode]) {
        let window = self
            .windows
            .get(&pam_desktop_ui::backend::MAIN_WINDOW)
            .map(|entry| entry.window.clone());
        self.shell
            .set_menu_bar(menu, window.as_ref(), pam_desktop_ui::backend::MAIN_WINDOW);
    }

    fn share(&mut self, window: WindowId, request: &pam_desktop_ui::shell::ShareRequest) -> bool {
        self.windows.get(&window).is_some_and(|entry| {
            crate::share::open(&self.shell, &entry.window, &entry.host, request)
        })
    }

    fn popup_menu(
        &mut self,
        window: WindowId,
        position: Point,
        menu: &[MenuNode],
        done: Box<dyn FnOnce(Option<String>)>,
    ) {
        match self.windows.get(&window) {
            Some(entry) => self.shell.popup_menu(&entry.host, position, menu, done),
            None => done(None),
        }
    }

    #[cfg(feature = "tray")]
    fn set_tray(&mut self, tray: Option<&TrayConfig>) {
        if self.tray.is_none() && tray.is_some() {
            let id = self
                .application
                .application_id()
                .map(|id| id.to_string())
                .unwrap_or_else(|| String::from("pam-desktop"));
            let title = self
                .windows
                .get(&pam_desktop_ui::backend::MAIN_WINDOW)
                .and_then(|entry| entry.window.title())
                .map(|title| title.to_string())
                .unwrap_or_else(|| id.clone());
            self.tray = Some(crate::tray::TrayHost::new(&id, &title, &self.events));
        }
        if let Some(host) = &mut self.tray {
            host.set(tray);
        }
    }

    #[cfg(not(feature = "tray"))]
    fn set_tray(&mut self, _tray: Option<&TrayConfig>) {}

    fn pick_files(
        &mut self,
        window: WindowId,
        config: &FileDialogConfig,
        done: Box<dyn FnOnce(Vec<std::path::PathBuf>)>,
    ) {
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
        self.windows
            .get(&window)
            .map(|entry| shell::window_state(&entry.window))
            .unwrap_or_default()
    }

    /// Shortcuts of the focused window are matched by the core; only the
    /// global ones are registered with the system.
    #[cfg(feature = "hotkeys")]
    fn set_shortcuts(&mut self, shortcuts: &[Shortcut]) {
        if self.hotkeys.is_none() && shortcuts.iter().any(|shortcut| shortcut.global) {
            self.hotkeys = Some(crate::hotkeys::Hotkeys::new(&self.events));
        }
        if let Some(hotkeys) = &mut self.hotkeys {
            hotkeys.set(shortcuts);
        }
    }

    #[cfg(not(feature = "hotkeys"))]
    fn set_shortcuts(&mut self, _shortcuts: &[Shortcut]) {}

    fn set_cursor(&mut self, id: NodeId, cursor: Cursor) {
        let _span = crate::prof::Span::new("set_cursor");
        if let Some(entry) = self.views.get(&id) {
            entry
                .widget
                .set_cursor_from_name(shell::cursor_name(cursor));
        }
    }

    fn set_tooltip(&mut self, id: NodeId, text: Option<&str>) {
        let _span = crate::prof::Span::new("set_tooltip");
        if let Some(entry) = self.views.get(&id) {
            entry.widget.set_tooltip_text(text);
        }
    }

    fn begin_window_drag(&mut self, window: WindowId) {
        let Some(entry) = self.windows.get(&window) else {
            return;
        };
        let Some((device, button, x, y, time)) = self.last_press.borrow().clone() else {
            return;
        };
        if let Some(toplevel) = entry
            .window
            .surface()
            .and_then(|surface| surface.downcast::<gdk::Toplevel>().ok())
        {
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
        self.wake_source = Some(glib::timeout_add_local_once(
            std::time::Duration::from_millis(u64::from(delay_ms)),
            move || {
                fired.set(true);
                events.push(BackendEvent::Wake);
            },
        ));
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
        _ => key
            .name()
            .map_or_else(|| String::from("Unidentified"), |name| name.to_string()),
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
    skip_gl_without_gpu();
    skip_a11y_without_bus();
    // One instance per application: a second launch hands its URL over.
    let flags = if config.single_instance {
        gio::ApplicationFlags::HANDLES_OPEN
    } else {
        gio::ApplicationFlags::NON_UNIQUE
    };
    let application = gtk::Application::new(Some(application_id), flags);
    if config.single_instance
        && application.register(gio::Cancellable::NONE).is_ok()
        && application.is_remote()
    {
        // Running a remote application forwards its arguments and returns.
        let arguments: Vec<&str> = std::iter::once("pam-desktop")
            .chain(config.launch_url.as_deref())
            .collect();
        application.run_with_args(&arguments);
        return ExitCode::SUCCESS;
    }
    let queue: Rc<RefCell<Option<Rc<EventQueue>>>> = Rc::new(RefCell::new(None));
    let links = Rc::clone(&queue);
    application.connect_open(move |application, files, _| {
        let Some(events) = links.borrow().clone() else {
            return;
        };
        if let Some(window) = application.windows().into_iter().next_back() {
            window.present();
        }
        for file in files {
            events.push(BackendEvent::OpenUrl(file.uri().to_string()));
        }
    });
    let config = RefCell::new(Some(config));
    let shared: Rc<RefCell<Option<Rc<Shared<GtkBackend>>>>> = Rc::new(RefCell::new(None));
    let failed = Rc::new(Cell::new(false));

    let keep = Rc::clone(&shared);
    let failure = Rc::clone(&failed);
    application.connect_activate(move |application| {
        let Some(config) = config.borrow_mut().take() else {
            // A second launch without a link: bring the application forward.
            if let Some(window) = application.windows().into_iter().next_back() {
                window.present();
            }
            return;
        };
        if let Some(display) = gdk::Display::default() {
            // Without a settings daemon GTK's X11 fallback is full hinting;
            // the fontconfig default (what Chromium and Qt use) is kept instead.
            if crate::x11::xsettings_managed(&display) == Some(false)
                && let (Some(style), Some(settings)) = (
                    crate::fonts::default_hint_style(),
                    gtk::Settings::for_display(&display).into(),
                )
            {
                settings.set_gtk_xft_hintstyle(Some(style));
            }
            let provider = gtk::CssProvider::new();
            crate::compat::load_css(&provider, STYLESHEET);
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        let events = EventQueue::new();
        *queue.borrow_mut() = Some(Rc::clone(&events));
        // Always queued, never run in place: `MainContext::invoke` calls the
        // pump at once on the UI thread, where a reply posted while the
        // runtime drains (a shortcut answering the shell listener) found it
        // borrowed, was skipped and stranded the inbox.
        let inbox = Inbox::new(|| {
            glib::idle_add_full(glib::Priority::DEFAULT, || {
                runtime::pump();
                glib::ControlFlow::Break
            });
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

/// The largest side a drawing (SVG) is rasterized at, in pixels.
const VECTOR_LIMIT: i32 = 4096;

/// The longest side to decode an image at, when it is not its own. A
/// picture is only ever made smaller, to `limit`. A drawing has no pixels
/// of its own: it is rasterized at the size it will be shown at, or at
/// twice its natural size when that is unknown, so that it stays sharp.
fn decode_side(longest: i32, limit: Option<i32>, vector: bool) -> Option<i32> {
    let wanted = match (limit, vector) {
        (Some(limit), true) => limit.min(VECTOR_LIMIT),
        (None, true) => longest.saturating_mul(2).min(VECTOR_LIMIT),
        (Some(limit), false) if longest > limit => limit,
        _ => return None,
    };
    (wanted != longest && longest > 0).then_some(wanted)
}

/// Decodes on the calling (worker) thread. The decoder is told the target
/// size up front, so large photos never exist at full size in memory.
fn decode_image(source: ImageSource, max_dimension: Option<u32>) -> Result<DecodedImage, String> {
    use gtk::gdk_pixbuf::prelude::*;
    let loader = gtk::gdk_pixbuf::PixbufLoader::new();
    let limit = max_dimension
        .and_then(|limit| i32::try_from(limit).ok())
        .filter(|limit| *limit > 0);
    loader.connect_size_prepared(move |loader, width, height| {
        let vector = loader
            .format()
            .is_some_and(|format| format.name().is_some_and(|name| name == "svg"));
        let longest = width.max(height);
        if let Some(side) = decode_side(longest, limit, vector) {
            let ratio = f64::from(side) / f64::from(longest);
            loader.set_size(
                ((f64::from(width) * ratio).round() as i32).max(1),
                ((f64::from(height) * ratio).round() as i32).max(1),
            );
        }
    });
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
    let pixbuf = loader
        .pixbuf()
        .ok_or_else(|| String::from("Image could not be decoded"))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_shrink_and_drawings_take_the_shown_size() {
        // A picture is never enlarged.
        assert_eq!(decode_side(4000, Some(512), false), Some(512));
        assert_eq!(decode_side(100, Some(512), false), None);
        assert_eq!(decode_side(100, None, false), None);
        // A drawing is rasterized at the size it is shown at.
        assert_eq!(decode_side(100, Some(512), true), Some(512));
        assert_eq!(decode_side(100, Some(48), true), Some(48));
        assert_eq!(decode_side(100, None, true), Some(200));
        assert_eq!(decode_side(100, Some(100), true), None);
        assert_eq!(decode_side(3000, None, true), Some(VECTOR_LIMIT));
    }
}

/// The widget opacity for a layer: CSS hit-testing ignores opacity, GTK picking
/// skips widgets whose 8-bit alpha is 0, so 0 becomes the lowest alpha step.
#[must_use]
pub(crate) fn pickable_opacity(opacity: f32) -> f32 {
    if opacity * 255.0 < 1.0 {
        1.0 / 255.0
    } else {
        opacity.min(1.0)
    }
}

thread_local! {
    /// One display-wide stylesheet holding a rule per distinct scrollbar
    /// style (thickness, colour); classes are reused across scroll views.
    static SCROLLBAR_STYLES: std::cell::RefCell<(Option<gtk::CssProvider>, Vec<String>, String)> =
        const { std::cell::RefCell::new((None, Vec::new(), String::new())) };
}

/// CSS `scrollbar-width`/`scrollbar-color`: the bar is drawn at the authored
/// thickness and exact colour (no theme tint, fade or transition). With a
/// reserved gutter the bar is classic: GTK shows it only while the content
/// overflows (the gutter stays reserved by the layout, as in Chromium);
/// otherwise it stays an overlay. `thin` draws Chromium's thumb, inset 2px
/// across its 10px track.
fn style_scrollbar(scrolled: &gtk::ScrolledWindow, config: &ScrollConfig) {
    for class in scrolled.css_classes() {
        if class.starts_with("pam-scrollbar-") {
            scrolled.remove_css_class(&class);
        }
    }
    let styled = config.scrollbar_thickness.is_some() || config.scrollbar_color.is_some();
    scrolled.set_overlay_scrolling(!config.reserved_gutter);
    if let (None, Some(c), true) = (
        config.scrollbar_thickness,
        config.scrollbar_color,
        config.reserved_gutter,
    ) {
        // `scrollbar-color` with the default width: Chromium's 15 px classic
        // bar (arrows, 9 px thumb) in the authored thumb colour on the
        // authored track colour (transparent without one), corner included.
        let track = config.scrollbar_track;
        let class = format!(
            "pam-scrollbar-classic-{}-{}",
            u32::from_be_bytes([c.a, c.r, c.g, c.b]),
            track.map_or(0, |t| u32::from_be_bytes([t.a, t.r, t.g, t.b]))
        );
        let css = |c: pam_desktop_ui::color::Color| {
            format!(
                "rgba({},{},{},{:.4})",
                c.r,
                c.g,
                c.b,
                f32::from(c.a) / 255.0
            )
        };
        let thumb = css(c);
        let track = track.map_or_else(|| "transparent".to_owned(), css);
        add_scrollbar_rule(&class, || classic_rule(&class, &track, &thumb));
        scrolled.add_css_class(&class);
        return;
    }
    if !styled {
        if config.reserved_gutter {
            // A reserved gutter without an authored bar: Chromium's default
            // classic bar. Its colour scheme follows the page (CSS
            // `color-scheme`); natively, the first opaque background behind
            // the scroller tells a dark page from a light one.
            let apply = |scrolled: &gtk::ScrolledWindow| {
                for class in ["pam-scrollbar-default-0", "pam-scrollbar-default-1"] {
                    scrolled.remove_css_class(class);
                }
                let dark = backdrop_is_dark(scrolled.upcast_ref());
                let class = format!("pam-scrollbar-default-{}", u8::from(dark));
                add_scrollbar_rule(&class, || classic_default_rule(&class, dark));
                scrolled.add_css_class(&class);
            };
            apply(scrolled);
            // The views around it may be attached after the configuration.
            let weak = scrolled.downgrade();
            glib::idle_add_local_once(move || {
                if let Some(scrolled) = weak.upgrade() {
                    apply(&scrolled);
                }
            });
        }
        return;
    }
    let thickness = config.scrollbar_thickness.unwrap_or(10.0).clamp(0.0, 64.0);
    let inset = config.thumb_inset.clamp(0.0, thickness / 4.0);
    let thumb = thickness - 2.0 * inset;
    let color = config.scrollbar_color.map_or_else(
        || "alpha(currentColor, 0.35)".to_owned(),
        |c| {
            format!(
                "rgba({},{},{},{:.4})",
                c.r,
                c.g,
                c.b,
                f32::from(c.a) / 255.0
            )
        },
    );
    let class = format!(
        "pam-scrollbar-{}-{}-{}",
        (thickness * 100.0).round() as u32,
        (inset * 100.0).round() as u32,
        config
            .scrollbar_color
            .map_or(0, |c| u32::from_be_bytes([c.a, c.r, c.g, c.b]))
    );
    let rule = || {
        format!(
            ".{class} > scrollbar {{ background: transparent; border: none; padding: 0; margin: 0; min-width: {thickness}px; min-height: {thickness}px; opacity: 1; transition: none; }}\n\
                 .{class} > scrollbar trough {{ background: transparent; border: none; padding: 0; margin: 0; }}\n\
                 .{class} > scrollbar slider {{ min-width: {thumb}px; min-height: {thumb}px; margin: 0; padding: 0; border: none; border-radius: {radius}px; background-clip: border-box; background-color: {color}; opacity: 1; transition: none; }}\n\
                 .{class} > scrollbar.vertical slider {{ margin: 0 {inset}px; min-height: 40px; }}\n\
                 .{class} > scrollbar.horizontal slider {{ margin: {inset}px 0; min-width: 40px; }}\n",
            radius = thumb / 2.0,
        )
    };
    add_scrollbar_rule(&class, rule);
    scrolled.add_css_class(&class);
    if config.reserved_gutter && config.thumb_inset > 0.0 {
        let classic = format!("{class}-classic");
        add_scrollbar_rule(&classic, || {
            classic_thin_rule(&classic, thickness, inset, &color)
        });
        scrolled.add_css_class(&classic);
    }
}

/// Whether the nearest mostly opaque view background around `widget` is dark.
fn backdrop_is_dark(widget: &gtk::Widget) -> bool {
    let mut current = Some(widget.clone());
    while let Some(widget) = current {
        if let Some(view) = widget.downcast_ref::<crate::view::PamView>() {
            let background = view.with_state(|state| state.decoration.background);
            if background.a >= 128 {
                let luminance = 0.2126 * f32::from(background.r)
                    + 0.7152 * f32::from(background.g)
                    + 0.0722 * f32::from(background.b);
                return luminance < 128.0;
            }
        }
        current = widget.parent();
    }
    false
}

/// Chromium's default classic bar on Linux (Fluent, no `scrollbar-width` or
/// `scrollbar-color`): a 15 px track, a 9 px rounded thumb 3 px from the
/// track edges, and a 15 px arrow button at each end with a 5-row triangle
/// (1, 3 .. 9 px) in the thumb colour. Dark colour scheme: track #2C2C2C,
/// thumb #9F9F9F; light: track #FCFCFC, thumb #8B8B8B (both measured on Electron 37).
fn classic_default_rule(class: &str, dark: bool) -> String {
    let (track, thumb_color) = if dark {
        ("#2C2C2C", "#9F9F9F")
    } else {
        ("#FCFCFC", "#8B8B8B")
    };
    classic_rule(class, track, thumb_color)
}

/// Chromium's 15 px classic bar in `track`/`thumb_color`; the corner between
/// two bars (GTK's junction) takes the track colour.
fn classic_rule(class: &str, track: &str, thumb_color: &str) -> String {
    const THICKNESS: f32 = 15.0;
    const INSET: f32 = 3.0;
    let thumb = THICKNESS - 2.0 * INSET;
    // The triangle's apex sits 6 px from the bar's end, its base 4 rows further.
    let (rows, apex) = (5_u32, 6.0_f32);
    let mut vertical = (Vec::new(), Vec::new(), Vec::new());
    let mut horizontal = (Vec::new(), Vec::new(), Vec::new());
    for row in 0..rows {
        let width = 2 * row + 1;
        let across = (THICKNESS - width as f32) / 2.0;
        let along = apex + row as f32;
        for along in [
            format!("{along}px"),
            format!("calc(100% - {}px)", along + 1.0),
        ] {
            vertical.0.push(format!("image({thumb_color})"));
            vertical.1.push(format!("{width}px 1px"));
            vertical.2.push(format!("{across}px {along}"));
            horizontal.0.push(format!("image({thumb_color})"));
            horizontal.1.push(format!("1px {width}px"));
            horizontal.2.push(format!("{along} {across}px"));
        }
    }
    let repeat = vec!["no-repeat"; vertical.0.len()].join(", ");
    format!(
        ".{class} > scrollbar {{ background-color: {track}; border: none; padding: 0; margin: 0; min-width: {THICKNESS}px; min-height: {THICKNESS}px; opacity: 1; transition: none; }}\n\
         .{class} > junction {{ background: {track}; border: none; }}\n\
         .{class} > scrollbar trough {{ background: transparent; border: none; padding: 0; }}\n\
         .{class} > scrollbar slider {{ min-width: {thumb}px; min-height: {thumb}px; margin: 0; padding: 0; border: none; border-radius: {radius}px; background-color: {thumb_color}; opacity: 1; transition: none; }}\n\
         .{class} > scrollbar.vertical:not(.overlay-indicator) {{ background-image: {vi}; background-size: {vs}; background-position: {vp}; background-repeat: {repeat}; }}\n\
         .{class} > scrollbar.horizontal:not(.overlay-indicator) {{ background-image: {hi}; background-size: {hs}; background-position: {hp}; background-repeat: {repeat}; }}\n\
         .{class} > scrollbar.vertical trough {{ margin: {edge}px 0; }}\n\
         .{class} > scrollbar.horizontal trough {{ margin: 0 {edge}px; }}\n\
         .{class} > scrollbar.vertical slider {{ margin: 0 {INSET}px; min-height: {min}px; }}\n\
         .{class} > scrollbar.horizontal slider {{ margin: {INSET}px 0; min-width: {min}px; }}\n",
        radius = thumb / 2.0,
        edge = THICKNESS + INSET,
        min = THICKNESS + 1.0,
        vi = vertical.0.join(", "),
        vs = vertical.1.join(", "),
        vp = vertical.2.join(", "),
        hi = horizontal.0.join(", "),
        hs = horizontal.1.join(", "),
        hp = horizontal.2.join(", "),
    )
}

/// Chromium's classic `thin` bar (Fluent): an arrow button as long as the
/// bar is thick at each end, holding a small triangle in the thumb colour;
/// the thumb travels between the buttons, `inset` away from them, and is at
/// least `thickness + 1` long.
fn classic_thin_rule(class: &str, thickness: f32, inset: f32, color: &str) -> String {
    let thumb = thickness - 2.0 * inset;
    let rows = (thumb / 2.0).floor().max(1.0) as u32;
    // The first row (2px wide) sits this far from the bar's end.
    let start = ((thickness - rows as f32) / 2.0).round();
    let edge = thickness + inset;
    let mut vertical = (Vec::new(), Vec::new(), Vec::new());
    let mut horizontal = (Vec::new(), Vec::new(), Vec::new());
    for row in 0..rows {
        let width = 2 * (row + 1);
        let across = (thickness - width as f32) / 2.0;
        let along = start + row as f32;
        // GTK has no `bottom 4px` positions: the far end is `100% - offset`.
        for along in [format!("{along}px"), format!("calc(100% - {along}px)")] {
            vertical.0.push(format!("image({color})"));
            vertical.1.push(format!("{width}px 1px"));
            vertical.2.push(format!("{across}px {along}"));
            horizontal.0.push(format!("image({color})"));
            horizontal.1.push(format!("1px {width}px"));
            horizontal.2.push(format!("{along} {across}px"));
        }
    }
    let repeat = vec!["no-repeat"; vertical.0.len()].join(", ");
    format!(
        ".{class} > scrollbar.vertical:not(.overlay-indicator) {{ background-image: {vi}; background-size: {vs}; background-position: {vp}; background-repeat: {repeat}; }}\n\
         .{class} > scrollbar.horizontal:not(.overlay-indicator) {{ background-image: {hi}; background-size: {hs}; background-position: {hp}; background-repeat: {repeat}; }}\n\
         .{class} > scrollbar.vertical:not(.overlay-indicator) trough {{ margin: {edge}px 0; }}\n\
         .{class} > scrollbar.horizontal:not(.overlay-indicator) trough {{ margin: 0 {edge}px; }}\n\
         .{class} > scrollbar.vertical:not(.overlay-indicator) slider {{ min-height: {min}px; }}\n\
         .{class} > scrollbar.horizontal:not(.overlay-indicator) slider {{ min-width: {min}px; }}\n",
        vi = vertical.0.join(", "),
        vs = vertical.1.join(", "),
        vp = vertical.2.join(", "),
        hi = horizontal.0.join(", "),
        hs = horizontal.1.join(", "),
        hp = horizontal.2.join(", "),
        min = thickness + 1.0,
    )
}

/// Adds `class`'s rule (built once) to the display-wide scrollbar stylesheet.
fn add_scrollbar_rule(class: &str, rule: impl FnOnce() -> String) {
    SCROLLBAR_STYLES.with(|styles| {
        let mut styles = styles.borrow_mut();
        if !styles.1.iter().any(|known| known == class) {
            let rule = rule();
            styles.1.push(class.to_owned());
            styles.2.push_str(&rule);
            if styles.0.is_none() {
                let provider = gtk::CssProvider::new();
                if let Some(display) = gtk::gdk::Display::default() {
                    gtk::style_context_add_provider_for_display(
                        &display,
                        &provider,
                        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
                    );
                }
                styles.0 = Some(provider);
            }
            let css = styles.2.clone();
            if let Some(provider) = styles.0.as_ref() {
                crate::compat::load_css(provider, &css);
            }
        }
    });
}

/// The reserved gutter of a classic bar shows the scroll element's own
/// background, as in Chromium: the content view stops at the bar, so without
/// this the (transparent) track showed whatever is behind the element (the
/// window). The track follows the element's outer corner radii. Overlay bars
/// float over the content and stay transparent.
fn style_gutter(scrolled: &gtk::ScrolledWindow, decoration: &Decoration) {
    for class in scrolled.css_classes() {
        if class.starts_with("pam-gutter") {
            scrolled.remove_css_class(&class);
        }
    }
    let c = decoration.background;
    if c.is_transparent() {
        return;
    }
    let r = decoration.radii;
    let class = format!(
        "pam-gutter-{}-{}-{}-{}",
        u32::from_be_bytes([c.a, c.r, c.g, c.b]),
        (r.top_right * 100.0).round() as u32,
        (r.bottom_right * 100.0).round() as u32,
        (r.bottom_left * 100.0).round() as u32,
    );
    let rule = || {
        let color = format!(
            "rgba({},{},{},{:.4})",
            c.r,
            c.g,
            c.b,
            f32::from(c.a) / 255.0
        );
        format!(
            ".pam-gutter.{class} > scrollbar:not(.overlay-indicator) {{ background-color: {color}; }}\n\
             .pam-gutter.{class} > scrollbar.vertical:not(.overlay-indicator) {{ border-radius: 0 {tr}px {br}px 0; }}\n\
             .pam-gutter.{class} > scrollbar.horizontal:not(.overlay-indicator) {{ border-radius: 0 0 {br}px {bl}px; }}\n",
            tr = r.top_right,
            br = r.bottom_right,
            bl = r.bottom_left,
        )
    };
    add_scrollbar_rule(&class, rule);
    scrolled.add_css_class("pam-gutter");
    scrolled.add_css_class(&class);
}

/// GTK test thread: a classic (reserved-gutter) thin bar is shown only while
/// its content overflows, at Chromium's thin metrics.
#[cfg(test)]
pub(crate) fn assert_classic_scrollbar() {
    use gtk::prelude::*;
    let config = ScrollConfig {
        scrollbar_thickness: Some(10.0),
        scrollbar_color: Some(pam_desktop_ui::color::Color::rgba(255, 255, 255, 20)),
        reserved_gutter: true,
        thumb_inset: 2.0,
        ..ScrollConfig::default()
    };
    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_policy(gtk::PolicyType::External, gtk::PolicyType::Automatic);
    style_scrollbar(&scrolled, &config);
    assert!(
        !scrolled.is_overlay_scrolling(),
        "a reserved gutter holds a classic bar"
    );
    let content = crate::view::PamView::new();
    scrolled.set_child(Some(&content));
    let window = gtk::Window::new();
    window.set_child(Some(&scrolled));
    for (height, overflows) in [(100.0, false), (1000.0, true)] {
        content.set_content_size(Some(pam_desktop_ui::geometry::Size::new(190.0, height)));
        scrolled.measure(gtk::Orientation::Vertical, 200);
        scrolled.allocate(200, 300, -1, None);
        let bar = scrolled.vscrollbar();
        assert_eq!(bar.is_child_visible(), overflows, "content {height}");
        if overflows {
            assert_eq!(bar.width(), 10, "thin track");
            let slider = bar
                .first_child()
                .and_then(|range| range.first_child())
                .and_then(|trough| trough.first_child())
                .expect("slider");
            assert_eq!(slider.width(), 6, "Chromium's thin thumb");
            let bounds = slider.compute_bounds(&bar).expect("slider bounds");
            assert!(
                (bounds.x() - 2.0).abs() < 0.01,
                "thumb inset {}",
                bounds.x()
            );
        }
    }
    let overlay = ScrollConfig {
        reserved_gutter: false,
        ..config
    };
    style_scrollbar(&scrolled, &overlay);
    assert!(
        scrolled.is_overlay_scrolling(),
        "scrollbar-gutter: auto overlays the bar"
    );
}

/// Without a GPU device (a virtual machine, a container, a remote or
/// headless session) GTK still loads the system OpenGL stack to probe it
/// (Mesa's llvmpipe: about 60 MB of the host's memory) and then draws with
/// Cairo all the same. There the probe is skipped. A GL setting or a GPU
/// renderer the user chose (`GDK_DISABLE`, `GDK_DEBUG=gl-…`, `GSK_RENDERER`
/// other than `cairo`) is kept.
fn skip_gl_without_gpu() {
    // GSK_RENDERER=cairo alone does not stop the probe.
    let chosen = std::env::var("GSK_RENDERER").is_ok_and(|renderer| renderer != "cairo")
        || std::env::var_os("GDK_DISABLE").is_some()
        || std::env::var("GDK_DEBUG").is_ok_and(|debug| debug.contains("gl"));
    if chosen || gpu_device() {
        return;
    }
    let debug = match std::env::var("GDK_DEBUG") {
        Ok(debug) if !debug.is_empty() => format!("{debug},gl-disable"),
        _ => "gl-disable".to_owned(),
    };
    // SAFETY: called first in `run`, before GTK or any other thread starts.
    unsafe {
        // GTK up to 4.14 reads GDK_DEBUG; GTK 4.16 and later read GDK_DISABLE.
        std::env::set_var("GDK_DEBUG", debug);
        std::env::set_var("GDK_DISABLE", "gl");
    }
}

/// Without a D-Bus session bus (a headless session, ssh, CI, a system
/// service) no assistive technology can reach the application, yet GTK up
/// to 4.14 tries to register every new widget on the accessibility bus and
/// waits for it to fail: about 5-13 ms per widget, a second and a half for
/// a screen of a few hundred views. There accessibility is switched off.
/// A `GTK_A11Y` the user set is kept.
fn skip_a11y_without_bus() {
    if std::env::var_os("GTK_A11Y").is_some() || session_bus() {
        return;
    }
    // SAFETY: called first in `run`, before GTK or any other thread starts.
    unsafe { std::env::set_var("GTK_A11Y", "none") };
}

/// A session bus address, or the per-user socket GIO falls back to.
fn session_bus() -> bool {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some_and(|address| !address.is_empty()) {
        return true;
    }
    std::env::var_os("XDG_RUNTIME_DIR")
        .is_some_and(|runtime| std::path::Path::new(&runtime).join("bus").exists())
}

/// A DRM device (Mesa, the open drivers, nvidia-drm) or the NVIDIA device.
fn gpu_device() -> bool {
    let drm = std::fs::read_dir("/dev/dri").is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("renderD") || name.starts_with("card")
        })
    });
    drm || std::path::Path::new("/dev/nvidia0").exists()
}
