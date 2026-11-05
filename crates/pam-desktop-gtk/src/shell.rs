//! Desktop shell on GTK: the menu bar, context menus, file dialogs,
//! notifications, window commands and drag and drop of files.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use pam_desktop_ui::backend::{BackendEvent, WindowId};
use pam_desktop_ui::geometry::Point;
use pam_desktop_ui::runtime::EventQueue;
use pam_desktop_ui::shell::{
    Accelerator, Cursor, FileDialogConfig, FileDialogKind, MenuNode, MenuRole, NotificationConfig, WindowCommand, WindowState,
};

use crate::view::PamView;

/// GTK accelerator syntax (`<Control><Shift>n`) of a portable accelerator.
pub fn accelerator_string(accelerator: &Accelerator) -> String {
    let mut text = String::new();
    if accelerator.modifiers.control {
        text.push_str("<Control>");
    }
    if accelerator.modifiers.alt {
        text.push_str("<Alt>");
    }
    if accelerator.modifiers.shift {
        text.push_str("<Shift>");
    }
    if accelerator.modifiers.meta {
        text.push_str("<Super>");
    }
    let key = match accelerator.key.as_str() {
        "Enter" => "Return",
        "ArrowLeft" => "Left",
        "ArrowRight" => "Right",
        "ArrowUp" => "Up",
        "ArrowDown" => "Down",
        "PageUp" => "Page_Up",
        "PageDown" => "Page_Down",
        "Backspace" => "BackSpace",
        " " => "space",
        "+" => "plus",
        "-" => "minus",
        "," => "comma",
        "." => "period",
        "/" => "slash",
        other => other,
    };
    text.push_str(key);
    text
}

/// What choosing an item does.
#[derive(Clone)]
enum Activation {
    /// Report the id to PHP.
    Report(String),
    Role(MenuRole),
}

/// Builds a `GMenu` and registers one action per item in `group` under
/// names `<prefix><n>`. Returns the accelerators to install.
fn build_menu(
    nodes: &[MenuNode],
    group: &impl IsA<gio::ActionMap>,
    scope: &str,
    counter: &mut u32,
    on_activate: &Rc<dyn Fn(Activation)>,
    accelerators: &mut Vec<(String, String)>,
) -> gio::Menu {
    let menu = gio::Menu::new();
    let mut section = gio::Menu::new();
    for node in nodes {
        match node {
            MenuNode::Separator => {
                menu.append_section(None, &section);
                section = gio::Menu::new();
            }
            MenuNode::Submenu { label, children, .. } => {
                let submenu = build_menu(children, group, scope, counter, on_activate, accelerators);
                section.append_submenu(Some(label), &submenu);
            }
            MenuNode::Item { id, label, accelerator, enabled, checked, role, .. } => {
                *counter += 1;
                let name = format!("item{counter}");
                let activation = match role {
                    Some(role) => Activation::Role(*role),
                    None => Activation::Report(id.clone()),
                };
                let action = match checked {
                    Some(checked) => gio::SimpleAction::new_stateful(&name, None, &checked.to_variant()),
                    None => gio::SimpleAction::new(&name, None),
                };
                action.set_enabled(*enabled);
                let handler = Rc::clone(on_activate);
                action.connect_activate(move |action, _| {
                    // Check items flip at once; PHP confirms with the next menu.
                    if let Some(state) = action.state().and_then(|state| state.get::<bool>()) {
                        action.set_state(&(!state).to_variant());
                    }
                    handler(activation.clone());
                });
                group.add_action(&action);
                let detailed = format!("{scope}.{name}");
                section.append(Some(label), Some(&detailed));
                if let Some(accelerator) = accelerator {
                    accelerators.push((detailed, accelerator_string(accelerator)));
                }
            }
        }
    }
    menu.append_section(None, &section);
    menu
}

pub struct Shell {
    application: gtk::Application,
    events: Rc<EventQueue>,
    /// Action names and accelerators of the current menu bar.
    menu_actions: Vec<String>,
    menu_accelerators: Vec<String>,
}

impl Shell {
    pub fn new(application: gtk::Application, events: Rc<EventQueue>) -> Self {
        // One action reports every activated notification.
        let action = gio::SimpleAction::new("notification-activated", Some(glib::VariantTy::STRING));
        let queue = Rc::clone(&events);
        action.connect_activate(move |_, parameter| {
            if let Some(id) = parameter.and_then(|parameter| parameter.get::<String>()) {
                queue.push(BackendEvent::NotificationActivated { id });
            }
        });
        application.add_action(&action);
        Self { application, events, menu_actions: Vec::new(), menu_accelerators: Vec::new() }
    }

    fn role_handler(&self, window: Option<gtk::ApplicationWindow>, main: WindowId) -> Rc<dyn Fn(Activation)> {
        let events = Rc::clone(&self.events);
        Rc::new(move |activation| match activation {
            Activation::Report(id) => events.push(BackendEvent::MenuAction { id }),
            Activation::Role(role) => {
                let focus = window.as_ref().and_then(gtk::prelude::GtkWindowExt::focus);
                let edit = |action: &str| {
                    if let Some(focus) = &focus {
                        let _ = focus.activate_action(action, None);
                    }
                };
                match role {
                    MenuRole::Quit | MenuRole::CloseWindow => events.push(BackendEvent::WindowCloseRequested { window: main }),
                    MenuRole::Undo => edit("text.undo"),
                    MenuRole::Redo => edit("text.redo"),
                    MenuRole::Cut => edit("clipboard.cut"),
                    MenuRole::Copy => edit("clipboard.copy"),
                    MenuRole::Paste => edit("clipboard.paste"),
                    MenuRole::SelectAll => edit("selection.select-all"),
                    MenuRole::Minimize => {
                        if let Some(window) = &window {
                            window.minimize();
                        }
                    }
                    MenuRole::Zoom => {
                        if let Some(window) = &window {
                            window.set_maximized(!window.is_maximized());
                        }
                    }
                    MenuRole::ToggleFullScreen => {
                        if let Some(window) = &window {
                            window.set_fullscreened(!window.is_fullscreen());
                        }
                    }
                    // The application decides what these show.
                    MenuRole::About => events.push(BackendEvent::MenuAction { id: "role:about".into() }),
                    MenuRole::Preferences => events.push(BackendEvent::MenuAction { id: "role:preferences".into() }),
                    MenuRole::Hide | MenuRole::HideOthers | MenuRole::ShowAll => {}
                }
            }
        })
    }

    pub fn set_menu_bar(&mut self, menu: &[MenuNode], window: Option<&gtk::ApplicationWindow>, main: WindowId) {
        for name in self.menu_actions.drain(..) {
            self.application.remove_action(&name);
        }
        for detailed in self.menu_accelerators.drain(..) {
            self.application.set_accels_for_action(&detailed, &[]);
        }
        if menu.is_empty() {
            self.application.set_menubar(None::<&gio::Menu>);
            if let Some(window) = window {
                window.set_show_menubar(false);
            }
            return;
        }
        let handler = self.role_handler(window.cloned(), main);
        let mut counter = 0;
        let mut accelerators = Vec::new();
        let model = build_menu(menu, &self.application, "app", &mut counter, &handler, &mut accelerators);
        self.menu_actions = (1..=counter).map(|index| format!("item{index}")).collect();
        for (detailed, accelerator) in accelerators {
            self.application.set_accels_for_action(&detailed, &[accelerator.as_str()]);
            self.menu_accelerators.push(detailed);
        }
        self.application.set_menubar(Some(&model));
        if let Some(window) = window {
            window.set_show_menubar(true);
        }
    }

    pub fn popup_menu(&self, host: &PamView, position: Point, menu: &[MenuNode], done: Box<dyn FnOnce(Option<String>)>) {
        let group = gio::SimpleActionGroup::new();
        let done = Rc::new(RefCell::new(Some(done)));
        let chosen = Rc::new(Cell::new(false));
        let handler: Rc<dyn Fn(Activation)> = {
            let done = Rc::clone(&done);
            let chosen = Rc::clone(&chosen);
            Rc::new(move |activation| {
                chosen.set(true);
                if let (Some(done), Activation::Report(id)) = (done.borrow_mut().take(), activation) {
                    done(Some(id));
                }
            })
        };
        let mut counter = 0;
        let mut accelerators = Vec::new();
        let model = build_menu(menu, &group, "context", &mut counter, &handler, &mut accelerators);
        host.insert_action_group("context", Some(&group));
        let popover = gtk::PopoverMenu::from_model(Some(&model));
        popover.set_parent(host);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(position.x.round() as i32, position.y.round() as i32, 1, 1)));
        // A tooltip must not open over the menu: silence the widgets under
        // the pointer until the menu closes.
        let mut silenced = Vec::new();
        let mut current = host.pick(f64::from(position.x), f64::from(position.y), gtk::PickFlags::DEFAULT);
        while let Some(widget) = current {
            if widget.has_tooltip() {
                widget.set_has_tooltip(false);
                silenced.push(widget.downgrade());
            }
            current = widget.parent();
        }
        popover.connect_closed(move |popover| {
            let popover = popover.clone();
            let done = Rc::clone(&done);
            for widget in silenced.iter().filter_map(glib::WeakRef::upgrade) {
                widget.set_has_tooltip(true);
            }
            // The chosen action runs after `closed`: answer "dismissed" only if none did.
            glib::idle_add_local_once(move || {
                if let Some(done) = done.borrow_mut().take() {
                    done(None);
                }
                popover.unparent();
            });
        });
        popover.popup();
    }

    pub fn pick_files(&self, parent: Option<&gtk::ApplicationWindow>, config: &FileDialogConfig, done: Box<dyn FnOnce(Vec<PathBuf>)>) {
        let dialog = gtk::FileDialog::builder().modal(true).build();
        if !config.title.is_empty() {
            dialog.set_title(&config.title);
        }
        if !config.default_name.is_empty() {
            dialog.set_initial_name(Some(&config.default_name));
        }
        if let Some(directory) = &config.directory {
            dialog.set_initial_folder(Some(&gio::File::for_path(directory)));
        }
        if !config.filters.is_empty() {
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            for filter in &config.filters {
                let entry = gtk::FileFilter::new();
                entry.set_name(Some(&filter.name));
                for extension in &filter.extensions {
                    if extension == "*" {
                        entry.add_pattern("*");
                    } else {
                        entry.add_suffix(extension);
                    }
                }
                filters.append(&entry);
            }
            dialog.set_filters(Some(&filters));
        }
        if config.kind == FileDialogKind::OpenMultiple {
            dialog.open_multiple(parent, gio::Cancellable::NONE, move |result| {
                let mut paths = Vec::new();
                if let Ok(files) = result {
                    for index in 0..files.n_items() {
                        if let Some(path) = files.item(index).and_downcast::<gio::File>().and_then(|file| file.path()) {
                            paths.push(path);
                        }
                    }
                }
                done(paths);
            });
            return;
        }
        let single = move |result: Result<gio::File, glib::Error>| {
            done(result.ok().and_then(|file| file.path()).into_iter().collect());
        };
        match config.kind {
            FileDialogKind::Save => dialog.save(parent, gio::Cancellable::NONE, single),
            FileDialogKind::SelectFolder => dialog.select_folder(parent, gio::Cancellable::NONE, single),
            FileDialogKind::Open | FileDialogKind::OpenMultiple => dialog.open(parent, gio::Cancellable::NONE, single),
        }
    }

    pub fn notify(&self, notification: &NotificationConfig) {
        let native = gio::Notification::new(&notification.title);
        if !notification.body.is_empty() {
            native.set_body(Some(&notification.body));
        }
        native.set_default_action_and_target_value("app.notification-activated", Some(&notification.id.to_variant()));
        let id = (!notification.id.is_empty()).then_some(notification.id.as_str());
        self.application.send_notification(id, &native);
    }
}

pub fn window_command(window: &gtk::ApplicationWindow, host: &PamView, command: &WindowCommand) {
    match command {
        WindowCommand::SetTitle(title) => window.set_title(Some(title)),
        WindowCommand::SetSize(size) => window.set_default_size(size.width.round() as i32, size.height.round() as i32),
        WindowCommand::SetMinSize(size) => match size {
            Some(size) => host.set_size_request(size.width.round() as i32, size.height.round() as i32),
            None => host.set_size_request(-1, -1),
        },
        WindowCommand::SetResizable(resizable) => window.set_resizable(*resizable),
        WindowCommand::Minimize => window.minimize(),
        WindowCommand::Maximize => window.maximize(),
        WindowCommand::Restore => {
            if window.is_fullscreen() {
                window.unfullscreen();
            } else if window.is_maximized() {
                window.unmaximize();
            } else {
                window.present();
            }
        }
        WindowCommand::SetFullScreen(enabled) => window.set_fullscreened(*enabled),
        WindowCommand::Focus | WindowCommand::Show => window.present(),
        WindowCommand::Hide => window.set_visible(false),
        WindowCommand::Close => window.destroy(),
        // Wayland gives applications no say on placement, stacking or
        // taskbar progress; GTK exposes none of them.
        WindowCommand::Center | WindowCommand::SetAlwaysOnTop(_) | WindowCommand::SetProgress(_) => {}
    }
}

pub fn window_state(window: &gtk::ApplicationWindow) -> WindowState {
    let minimized = window
        .surface()
        .and_then(|surface| surface.downcast::<gdk::Toplevel>().ok())
        .is_some_and(|toplevel| toplevel.state().contains(gdk::ToplevelState::MINIMIZED));
    WindowState { maximized: window.is_maximized(), minimized, full_screen: window.is_fullscreen(), focused: window.is_active() }
}

/// Reports maximize, full screen and focus changes.
pub fn watch_window_state(window: &gtk::ApplicationWindow, id: WindowId, events: &Rc<EventQueue>) {
    for property in ["maximized", "fullscreened"] {
        let events = Rc::clone(events);
        window.connect_notify_local(Some(property), move |window, _| {
            events.push(BackendEvent::WindowStateChanged { window: id, state: window_state(window) });
        });
    }
}

/// Files dragged from the file manager onto the window.
pub fn accept_file_drops(host: &PamView, window: WindowId, events: &Rc<EventQueue>) {
    let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    let events = Rc::clone(events);
    target.connect_drop(move |_, value, x, y| {
        let Ok(files) = value.get::<gdk::FileList>() else { return false };
        let paths: Vec<PathBuf> = files.files().iter().filter_map(gio::File::path).collect();
        if paths.is_empty() {
            return false;
        }
        events.push(BackendEvent::FilesDropped { window, position: Point::new(x as f32, y as f32), paths });
        true
    });
    host.add_controller(target);
}

/// Text dragged from the application itself or from another one.
pub fn accept_text_drops(host: &PamView, window: WindowId, events: &Rc<EventQueue>) {
    let target = gtk::DropTarget::new(String::static_type(), gdk::DragAction::COPY);
    let events = Rc::clone(events);
    target.connect_drop(move |_, value, x, y| {
        let Ok(text) = value.get::<String>() else { return false };
        events.push(BackendEvent::TextDropped { window, position: Point::new(x as f32, y as f32), text });
        true
    });
    host.add_controller(target);
}

pub fn cursor_name(cursor: Cursor) -> Option<&'static str> {
    Some(match cursor {
        Cursor::Default => return None,
        Cursor::Pointer => "pointer",
        Cursor::Text => "text",
        Cursor::Grab => "grab",
        Cursor::Grabbing => "grabbing",
        Cursor::NotAllowed => "not-allowed",
        Cursor::Crosshair => "crosshair",
        Cursor::Wait => "wait",
        Cursor::Move => "move",
        Cursor::ResizeHorizontal => "ew-resize",
        Cursor::ResizeVertical => "ns-resize",
    })
}
