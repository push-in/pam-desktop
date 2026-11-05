//! The tray icon. GTK 4 has none of its own, so the application registers
//! a StatusNotifierItem on the session bus (the protocol KDE, the GNOME
//! AppIndicator extension, Cinnamon, XFCE and others display).

use std::path::Path;
use std::rc::Rc;

use gtk::glib;
use ksni::blocking::TrayMethods as _;
use pam_desktop_ui::backend::{BackendEvent, MAIN_WINDOW};
use pam_desktop_ui::runtime::EventQueue;
use pam_desktop_ui::shell::{MenuNode, MenuRole, TrayConfig};

/// What the tray service reports from its own thread.
enum TrayEvent {
    Activated,
    Menu(String),
    Role(MenuRole),
}

struct AppTray {
    id: String,
    title: String,
    tooltip: String,
    icon: Vec<ksni::Icon>,
    menu: Vec<MenuNode>,
    sender: async_channel::Sender<TrayEvent>,
}

impl AppTray {
    fn items(&self, nodes: &[MenuNode]) -> Vec<ksni::MenuItem<Self>> {
        nodes
            .iter()
            .map(|node| match node {
                MenuNode::Separator => ksni::MenuItem::Separator,
                MenuNode::Submenu { label, enabled, children } => {
                    ksni::menu::SubMenu { label: label.clone(), enabled: *enabled, submenu: self.items(children), ..Default::default() }.into()
                }
                MenuNode::Item { id, label, enabled, checked, role, .. } => {
                    let event = match role {
                        Some(role) => TrayEvent::Role(*role),
                        None => TrayEvent::Menu(id.clone()),
                    };
                    let event = std::sync::Mutex::new(Some(event));
                    let (id, role) = (id.clone(), *role);
                    let activate = move |tray: &mut Self| {
                        // The first activation moves the event out; later ones rebuild it.
                        let next = event.lock().ok().and_then(|mut slot| slot.take()).unwrap_or(match role {
                            Some(role) => TrayEvent::Role(role),
                            None => TrayEvent::Menu(id.clone()),
                        });
                        let _ = tray.sender.send_blocking(next);
                    };
                    match checked {
                        Some(checked) => ksni::menu::CheckmarkItem { label: label.clone(), enabled: *enabled, checked: *checked, activate: Box::new(activate), ..Default::default() }.into(),
                        None => ksni::menu::StandardItem { label: label.clone(), enabled: *enabled, activate: Box::new(activate), ..Default::default() }.into(),
                    }
                }
            })
            .collect()
    }
}

impl ksni::Tray for AppTray {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn title(&self) -> String {
        self.title.clone()
    }

    fn icon_name(&self) -> String {
        // The themed icon of the application when no picture was given.
        if self.icon.is_empty() { self.id.clone() } else { String::new() }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icon.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: if self.tooltip.is_empty() { self.title.clone() } else { self.tooltip.clone() }, ..Default::default() }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        self.items(&self.menu)
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.sender.send_blocking(TrayEvent::Activated);
    }
}

/// The pixels of a PNG as the big-endian ARGB the protocol carries.
fn load_icon(path: &Path) -> Vec<ksni::Icon> {
    use gtk::gdk_pixbuf::Pixbuf;
    let Ok(pixbuf) = Pixbuf::from_file_at_scale(path, 64, 64, true) else { return Vec::new() };
    let Ok(pixbuf) = pixbuf.add_alpha(false, 0, 0, 0) else { return Vec::new() };
    let (width, height, stride) = (pixbuf.width(), pixbuf.height(), pixbuf.rowstride() as usize);
    let pixels = pixbuf.read_pixel_bytes();
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height as usize {
        for column in 0..width as usize {
            let offset = row * stride + column * 4;
            let Some(rgba) = pixels.get(offset..offset + 4) else { return Vec::new() };
            data.extend_from_slice(&[rgba[3], rgba[0], rgba[1], rgba[2]]);
        }
    }
    vec![ksni::Icon { width, height, data }]
}

pub struct TrayHost {
    application_id: String,
    title: String,
    sender: async_channel::Sender<TrayEvent>,
    handle: Option<ksni::blocking::Handle<AppTray>>,
}

impl TrayHost {
    pub fn new(application_id: &str, title: &str, events: &Rc<EventQueue>) -> Self {
        let (sender, receiver) = async_channel::unbounded::<TrayEvent>();
        let events = Rc::clone(events);
        // The service runs on its own thread; its events are handled here,
        // on the thread that owns the windows.
        glib::MainContext::default().spawn_local(async move {
            while let Ok(event) = receiver.recv().await {
                events.push(match event {
                    TrayEvent::Activated => BackendEvent::TrayActivated,
                    TrayEvent::Menu(id) => BackendEvent::MenuAction { id },
                    TrayEvent::Role(MenuRole::Quit | MenuRole::CloseWindow) => BackendEvent::WindowCloseRequested { window: MAIN_WINDOW },
                    TrayEvent::Role(MenuRole::About) => BackendEvent::MenuAction { id: "role:about".into() },
                    TrayEvent::Role(MenuRole::Preferences) => BackendEvent::MenuAction { id: "role:preferences".into() },
                    // Edit and window roles mean nothing without a focused window.
                    TrayEvent::Role(_) => continue,
                });
            }
        });
        Self { application_id: application_id.to_owned(), title: title.to_owned(), sender, handle: None }
    }

    /// Shows, updates or removes the tray icon.
    pub fn set(&mut self, config: Option<&TrayConfig>) {
        let Some(config) = config else {
            if let Some(handle) = self.handle.take() {
                handle.shutdown().wait();
            }
            return;
        };
        let icon = config.icon.as_deref().map(load_icon).unwrap_or_default();
        let (tooltip, menu) = (config.tooltip.clone(), config.menu.clone());
        if let Some(handle) = &self.handle {
            let updated = handle.update(|tray| {
                tray.icon = icon.clone();
                tray.tooltip.clone_from(&tooltip);
                tray.menu.clone_from(&menu);
            });
            if updated.is_some() {
                return;
            }
            // The service stopped (the session bus went away): start again.
            self.handle = None;
        }
        let tray = AppTray { id: self.application_id.clone(), title: self.title.clone(), tooltip, icon, menu, sender: self.sender.clone() };
        // A desktop without a tray host must not fail the application: the
        // icon appears if one starts later.
        match tray.assume_sni_available(true).spawn() {
            Ok(handle) => self.handle = Some(handle),
            Err(error) => eprintln!("[pam-desktop] the tray icon is unavailable: {error}"),
        }
    }
}
