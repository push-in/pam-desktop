//! Global shortcuts: key combinations that reach the application while
//! another one has the focus.
//!
//! On X11 the keys are grabbed on the root window (a small helper thread
//! owns the connection and sleeps in the server's event queue). On Wayland
//! applications cannot grab keys: the GlobalShortcuts desktop portal binds
//! them, with the consent of the user.

use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use gtk::prelude::*;
use gtk::{gio, glib};
use pam_desktop_ui::backend::{BackendEvent, Modifiers};
use pam_desktop_ui::runtime::EventQueue;
use pam_desktop_ui::shell::{Accelerator, Shortcut};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, ConnectionExt as _, GrabMode, ModMask};
use x11rb::protocol::Event;

/// The X keysym of a W3C key name.
fn keysym(key: &str) -> Option<u32> {
    Some(match key {
        "Enter" => 0xFF0D,
        "Escape" => 0xFF1B,
        "Tab" => 0xFF09,
        "Backspace" => 0xFF08,
        "Delete" => 0xFFFF,
        "Insert" => 0xFF63,
        "Home" => 0xFF50,
        "End" => 0xFF57,
        "PageUp" => 0xFF55,
        "PageDown" => 0xFF56,
        "ArrowLeft" => 0xFF51,
        "ArrowUp" => 0xFF52,
        "ArrowRight" => 0xFF53,
        "ArrowDown" => 0xFF54,
        _ => {
            if let Some(number) = key.strip_prefix('F').and_then(|number| number.parse::<u32>().ok()).filter(|number| (1..=24).contains(number)) {
                return Some(0xFFBE + number - 1);
            }
            let mut characters = key.chars();
            match (characters.next(), characters.next()) {
                // Latin-1 keysyms are the character codes themselves.
                (Some(character), None) if (character as u32) >= 0x20 && (character as u32) <= 0xFF => character as u32,
                _ => return None,
            }
        }
    })
}

/// The name the portal knows a key by (an XKB keysym name).
fn keysym_name(key: &str) -> Option<String> {
    Some(
        match key {
            "Enter" => "Return",
            "Escape" => "Escape",
            "Tab" => "Tab",
            "Backspace" => "BackSpace",
            "Delete" => "Delete",
            "Insert" => "Insert",
            "Home" => "Home",
            "End" => "End",
            "PageUp" => "Page_Up",
            "PageDown" => "Page_Down",
            "ArrowLeft" => "Left",
            "ArrowUp" => "Up",
            "ArrowRight" => "Right",
            "ArrowDown" => "Down",
            " " => "space",
            "+" => "plus",
            "-" => "minus",
            "," => "comma",
            "." => "period",
            "/" => "slash",
            other => {
                let single = other.chars().count() == 1 && other.chars().all(|character| character.is_ascii_alphanumeric());
                let function = other.strip_prefix('F').is_some_and(|number| number.parse::<u8>().is_ok());
                return (single || function).then(|| other.to_owned());
            }
        }
        .to_owned(),
    )
}

/// `CTRL+SHIFT+a`, the trigger syntax of the shortcuts portal.
fn portal_trigger(accelerator: &Accelerator) -> Option<String> {
    let mut trigger = String::new();
    for (held, name) in [(accelerator.modifiers.control, "CTRL"), (accelerator.modifiers.alt, "ALT"), (accelerator.modifiers.shift, "SHIFT"), (accelerator.modifiers.meta, "LOGO")] {
        if held {
            trigger.push_str(name);
            trigger.push('+');
        }
    }
    trigger.push_str(&keysym_name(&accelerator.key)?);
    Some(trigger)
}

fn x11_modifiers(modifiers: Modifiers) -> ModMask {
    let mut mask = ModMask::default();
    if modifiers.shift {
        mask |= ModMask::SHIFT;
    }
    if modifiers.control {
        mask |= ModMask::CONTROL;
    }
    if modifiers.alt {
        mask |= ModMask::M1;
    }
    if modifiers.meta {
        mask |= ModMask::M4;
    }
    mask
}

/// Grabs the shortcuts on the root window and reports presses until
/// `stop` is set. Runs on its own thread.
fn x11_listen(shortcuts: Vec<(String, Accelerator)>, sender: async_channel::Sender<String>, stop: Arc<AtomicBool>, wake_window: std::sync::mpsc::Sender<u32>) -> Result<(), Box<dyn std::error::Error>> {
    let (connection, screen) = x11rb::connect(None)?;
    let setup = connection.setup();
    let root = setup.roots.get(screen).ok_or("no X screen")?.root;
    // A window of our own receives the message that ends the wait below.
    let window = connection.generate_id()?;
    connection.create_window(0, window, root, 0, 0, 1, 1, 0, xproto::WindowClass::INPUT_ONLY, 0, &xproto::CreateWindowAux::new())?;
    let _ = wake_window.send(window);

    let (first, count) = (setup.min_keycode, setup.max_keycode - setup.min_keycode + 1);
    let mapping = connection.get_keyboard_mapping(first, count)?.reply()?;
    let per_code = usize::from(mapping.keysyms_per_keycode).max(1);
    let keycode_of = |symbol: u32| {
        // Letters are grabbed by their lower-case symbol: Shift is a modifier of the grab.
        mapping.keysyms.chunks(per_code).position(|symbols| symbols.first() == Some(&symbol) || symbols.get(1) == Some(&symbol)).map(|index| first + index as u8)
    };
    let mut grabbed: Vec<(u8, u16, String)> = Vec::new();
    // Caps Lock and Num Lock must not hide a shortcut.
    let locks = [ModMask::default(), ModMask::LOCK, ModMask::M2, ModMask::LOCK | ModMask::M2];
    for (id, accelerator) in shortcuts {
        let Some(code) = keysym(&accelerator.key).and_then(keycode_of) else {
            eprintln!("[pam-desktop] the global shortcut {id} names a key this keyboard does not have");
            continue;
        };
        let mask = x11_modifiers(accelerator.modifiers);
        for lock in locks {
            // A combination another application holds is refused by the server.
            let grab = connection.grab_key(false, root, mask | lock, code, GrabMode::ASYNC, GrabMode::ASYNC)?;
            if grab.check().is_err() {
                eprintln!("[pam-desktop] the global shortcut {id} is taken by another application");
                break;
            }
        }
        grabbed.push((code, u16::from(mask), id));
    }
    connection.flush()?;
    let relevant = u16::from(ModMask::SHIFT | ModMask::CONTROL | ModMask::M1 | ModMask::M4);
    while !stop.load(Ordering::Relaxed) {
        match connection.wait_for_event()? {
            Event::KeyPress(press) => {
                let state = u16::from(press.state) & relevant;
                if let Some((_, _, id)) = grabbed.iter().find(|(code, mask, _)| *code == press.detail && *mask == state)
                    && sender.send_blocking(id.clone()).is_err() {
                        break;
                    }
            }
            // The message `Hotkeys::stop` sends, or anything else: re-check.
            _ => {}
        }
    }
    // Closing the connection releases every grab.
    Ok(())
}

struct X11Listener {
    stop: Arc<AtomicBool>,
    window: Option<u32>,
}

impl X11Listener {
    fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wake the thread out of its wait with a message to its own window.
        let Some(window) = self.window else { return };
        if let Ok((connection, _)) = x11rb::connect(None) {
            let event = xproto::ClientMessageEvent::new(32, window, xproto::AtomEnum::STRING, [0_u32; 5]);
            let _ = connection.send_event(false, window, xproto::EventMask::NO_EVENT, event);
            let _ = connection.flush();
        }
    }
}

/// The portal session that holds the bindings on Wayland.
struct PortalSession {
    connection: gio::DBusConnection,
    /// Dropping a subscription removes it from the bus.
    subscriptions: Vec<gio::SignalSubscription>,
    session: Rc<std::cell::RefCell<Option<String>>>,
}

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SHORTCUTS: &str = "org.freedesktop.portal.GlobalShortcuts";

impl PortalSession {
    /// Opens a session and binds `shortcuts` once it exists.
    fn start(shortcuts: &[(String, Accelerator)], events: &Rc<EventQueue>) -> Option<Self> {
        let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok()?;
        let sender = connection.unique_name()?.trim_start_matches(':').replace('.', "_");
        let token = format!("pam{}", std::process::id());
        let request_path = format!("{PORTAL_PATH}/request/{sender}/{token}");
        let session_slot: Rc<std::cell::RefCell<Option<String>>> = Rc::new(std::cell::RefCell::new(None));
        let mut subscriptions = Vec::new();

        // `Activated(session, shortcut id, timestamp, options)`.
        {
            let events = Rc::clone(events);
            subscriptions.push(connection.subscribe_to_signal(Some(PORTAL), Some(SHORTCUTS), Some("Activated"), Some(PORTAL_PATH), None, gio::DBusSignalFlags::NONE, move |signal| {
                if let Some(id) = signal.parameters.child_value(1).str() {
                    events.push(BackendEvent::Shortcut { id: id.to_owned() });
                }
            }));
        }
        // The answer to `CreateSession` carries the session handle: bind then.
        {
            let bindings: Vec<(String, String, String)> =
                shortcuts.iter().filter_map(|(id, accelerator)| Some((id.clone(), id.clone(), portal_trigger(accelerator)?))).collect();
            let (session_slot, connection_for_bind, token) = (Rc::clone(&session_slot), connection.clone(), token.clone());
            subscriptions.push(connection.subscribe_to_signal(Some(PORTAL), Some("org.freedesktop.portal.Request"), Some("Response"), Some(&request_path), None, gio::DBusSignalFlags::NONE, move |signal| {
                if session_slot.borrow().is_some() {
                    return;
                }
                let results = glib::VariantDict::new(Some(&signal.parameters.child_value(1)));
                let Some(session) = results.lookup_value("session_handle", None).and_then(|value| value.str().map(str::to_owned)) else {
                    eprintln!("[pam-desktop] the desktop refused a global shortcuts session");
                    return;
                };
                *session_slot.borrow_mut() = Some(session.clone());
                let entries: Vec<glib::Variant> = bindings
                    .iter()
                    .map(|(id, description, trigger)| {
                        let options = glib::VariantDict::new(None);
                        options.insert_value("description", &description.to_variant());
                        options.insert_value("preferred_trigger", &trigger.to_variant());
                        glib::Variant::tuple_from_iter([id.to_variant(), options.end()])
                    })
                    .collect();
                let list = glib::Variant::array_from_iter_with_type(glib::VariantTy::new("(sa{sv})").unwrap_or(glib::VariantTy::ANY), entries);
                let options = glib::VariantDict::new(None);
                options.insert_value("handle_token", &format!("{token}b").to_variant());
                let Ok(path) = glib::variant::ObjectPath::try_from(session) else { return };
                let arguments = glib::Variant::tuple_from_iter([path.to_variant(), list, "".to_variant(), options.end()]);
                connection_for_bind.call(Some(PORTAL), PORTAL_PATH, SHORTCUTS, "BindShortcuts", Some(&arguments), None, gio::DBusCallFlags::NONE, -1, gio::Cancellable::NONE, |result| {
                    if let Err(error) = result {
                        eprintln!("[pam-desktop] global shortcuts could not be bound: {}", error.message());
                    }
                });
            }));
        }
        let options = glib::VariantDict::new(None);
        options.insert_value("handle_token", &token.to_variant());
        options.insert_value("session_handle_token", &format!("{token}s").to_variant());
        let arguments = glib::Variant::tuple_from_iter([options.end()]);
        connection.call(Some(PORTAL), PORTAL_PATH, SHORTCUTS, "CreateSession", Some(&arguments), None, gio::DBusCallFlags::NONE, -1, gio::Cancellable::NONE, |result| {
            if let Err(error) = result {
                eprintln!("[pam-desktop] global shortcuts are unavailable on this desktop: {}", error.message());
            }
        });
        Some(Self { connection, subscriptions, session: session_slot })
    }

    fn close(mut self) {
        self.subscriptions.clear();
        if let Some(session) = self.session.borrow_mut().take() {
            self.connection.call(Some(PORTAL), &session, "org.freedesktop.portal.Session", "Close", None, None, gio::DBusCallFlags::NONE, -1, gio::Cancellable::NONE, |_| {});
        }
    }
}

pub struct Hotkeys {
    events: Rc<EventQueue>,
    sender: async_channel::Sender<String>,
    x11: Option<X11Listener>,
    portal: Option<PortalSession>,
    current: Vec<(String, Accelerator)>,
}

impl Hotkeys {
    pub fn new(events: &Rc<EventQueue>) -> Self {
        let (sender, receiver) = async_channel::unbounded::<String>();
        let queue = Rc::clone(events);
        glib::MainContext::default().spawn_local(async move {
            while let Ok(id) = receiver.recv().await {
                queue.push(BackendEvent::Shortcut { id });
            }
        });
        Self { events: Rc::clone(events), sender, x11: None, portal: None, current: Vec::new() }
    }

    /// Replaces the global shortcuts of the application.
    pub fn set(&mut self, shortcuts: &[Shortcut]) {
        let wanted: Vec<(String, Accelerator)> = shortcuts.iter().filter(|shortcut| shortcut.global).map(|shortcut| (shortcut.id.clone(), shortcut.accelerator.clone())).collect();
        if wanted == self.current {
            return;
        }
        self.current.clone_from(&wanted);
        if let Some(listener) = self.x11.take() {
            listener.stop();
        }
        if let Some(portal) = self.portal.take() {
            portal.close();
        }
        if wanted.is_empty() {
            return;
        }
        let x11 = gtk::gdk::Display::default().is_some_and(|display| display.type_().name().starts_with("GdkX11"));
        if x11 {
            let stop = Arc::new(AtomicBool::new(false));
            let (window_sender, window_receiver) = std::sync::mpsc::channel();
            let (sender, thread_stop) = (self.sender.clone(), Arc::clone(&stop));
            let spawned = std::thread::Builder::new().name("pam-hotkeys".into()).spawn(move || {
                if let Err(error) = x11_listen(wanted, sender, thread_stop, window_sender) {
                    eprintln!("[pam-desktop] global shortcuts stopped: {error}");
                }
            });
            if spawned.is_ok() {
                // The thread reports its wake-up window as soon as it connects.
                let window = window_receiver.recv_timeout(std::time::Duration::from_secs(2)).ok();
                self.x11 = Some(X11Listener { stop, window });
            }
        } else {
            self.portal = PortalSession::start(&wanted, &self.events);
        }
    }
}

impl Drop for Hotkeys {
    fn drop(&mut self) {
        if let Some(listener) = self.x11.take() {
            listener.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_keysyms_and_portal_triggers() {
        assert_eq!(keysym("a"), Some(0x61));
        assert_eq!(keysym("F5"), Some(0xFFC2));
        assert_eq!(keysym(" "), Some(0x20));
        assert_eq!(keysym("ArrowUp"), Some(0xFF52));
        assert_eq!(keysym("Unknown"), None);
        let accelerator = Accelerator::parse("Ctrl+Shift+K").unwrap_or_else(|| panic!("valid accelerator"));
        assert_eq!(portal_trigger(&accelerator).as_deref(), Some("CTRL+SHIFT+k"));
        let space = Accelerator::parse("Super+Space").unwrap_or_else(|| panic!("valid accelerator"));
        assert_eq!(portal_trigger(&space).as_deref(), Some("LOGO+space"));
    }
}
