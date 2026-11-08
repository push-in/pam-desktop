//! Window placement and stacking on X11.
//!
//! GTK 4 exposes neither, because Wayland gives applications no say on
//! them. On X11 the window manager still takes both requests, so they are
//! sent to it directly; on Wayland the functions here do nothing.

use std::cell::Cell;
use std::ffi::{c_ulong, c_void};
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, glib};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConfigureWindowAux, ConnectionExt as _, EventMask, PropMode,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

/// The X window of a GTK window; `None` on Wayland and before the window
/// is realized.
fn xid(surface: &gdk::Surface) -> Option<u32> {
    // A toplevel is a `GdkX11Toplevel`, a kind of `GdkX11Surface`.
    let x11 = glib::Type::from_name("GdkX11Surface")?;
    if !surface.type_().is_a(x11) {
        return None;
    }
    // The X11 backend is part of libgtk-4. Looked up at run time, so that a
    // GTK built without it still loads.
    let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"gdk_x11_surface_get_xid".as_ptr()) };
    if symbol.is_null() {
        return None;
    }
    let get: unsafe extern "C" fn(*mut c_void) -> c_ulong = unsafe { std::mem::transmute(symbol) };
    u32::try_from(unsafe { get(surface.as_ptr().cast()) }).ok()
}

/// A connection to the display of the surface and the root window of its
/// screen. Opened per request: these are rare.
fn connect(surface: &gdk::Surface) -> Option<(RustConnection, u32)> {
    let name = surface.display().name();
    let (connection, screen) = x11rb::connect(Some(name.as_str())).ok()?;
    let root = connection.setup().roots.get(screen)?.root;
    Some((connection, root))
}

fn atom(connection: &RustConnection, name: &[u8]) -> Option<u32> {
    Some(connection.intern_atom(false, name).ok()?.reply().ok()?.atom)
}

/// Moves the window to the middle of the monitor it is on.
pub fn center(window: &gtk::Window) -> Option<()> {
    if window.is_maximized() || window.is_fullscreen() {
        return None;
    }
    let surface = window.surface()?;
    let xid = xid(&surface)?;
    let (connection, _) = connect(&surface)?;
    // X works in device pixels, GDK in application pixels.
    let scale = surface.scale_factor().max(1);
    let monitor = surface.display().monitor_at_surface(&surface)?.geometry();
    let size = connection.get_geometry(xid).ok()?.reply().ok()?;
    // The frame the window manager drew around the window: left, right, top, bottom.
    let frame: Vec<u32> = atom(&connection, b"_NET_FRAME_EXTENTS")
        .and_then(|extents| {
            connection
                .get_property(false, xid, extents, AtomEnum::CARDINAL, 0, 4)
                .ok()?
                .reply()
                .ok()
        })
        .and_then(|reply| reply.value32().map(Iterator::collect))
        .unwrap_or_default();
    let extent = |index: usize| frame.get(index).copied().unwrap_or(0) as i32;
    let width = i32::from(size.width) + extent(0) + extent(1);
    let height = i32::from(size.height) + extent(2) + extent(3);
    // The position of a managed window is that of its frame.
    let x = monitor.x() * scale + (monitor.width() * scale - width) / 2;
    let y = monitor.y() * scale + (monitor.height() * scale - height) / 2;
    connection
        .configure_window(xid, &ConfigureWindowAux::new().x(x).y(y))
        .ok()?;
    connection.flush().ok()
}

/// Asks the window manager to keep the window above the others, or to stop.
fn apply_above(window: &gtk::Window, enabled: bool) -> Option<()> {
    let surface = window.surface()?;
    let xid = xid(&surface)?;
    let (connection, root) = connect(&surface)?;
    let state = atom(&connection, b"_NET_WM_STATE")?;
    let above = atom(&connection, b"_NET_WM_STATE_ABOVE")?;
    if window.is_mapped() {
        // 1 adds the state, 0 removes it; the last 1 says an application asks.
        let message = ClientMessageEvent::new(32, xid, state, [u32::from(enabled), above, 0, 1, 0]);
        connection
            .send_event(
                false,
                root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                message,
            )
            .ok()?;
    } else {
        // Read by the window manager when the window is shown.
        let mut states: Vec<u32> = connection
            .get_property(false, xid, state, AtomEnum::ATOM, 0, 64)
            .ok()?
            .reply()
            .ok()
            .and_then(|reply| reply.value32().map(Iterator::collect))
            .unwrap_or_default();
        states.retain(|item| *item != above);
        if enabled {
            states.push(above);
        }
        connection
            .change_property32(PropMode::REPLACE, xid, state, AtomEnum::ATOM, &states)
            .ok()?;
    }
    connection.flush().ok()
}

const ABOVE: &str = "pam-desktop-always-on-top";

/// Keeps the window above the others, also after it is hidden and shown
/// again: the window manager forgets the state of a window that goes away.
pub fn set_always_on_top(window: &gtk::Window, enabled: bool) {
    // SAFETY: the key is only used here, always with a `Cell<bool>`.
    let wanted = unsafe { window.data::<Cell<bool>>(ABOVE) };
    match wanted {
        Some(wanted) => unsafe { wanted.as_ref() }.set(enabled),
        None => {
            unsafe { window.set_data(ABOVE, Cell::new(enabled)) };
            window.connect_map(|window| {
                let window = window.downgrade();
                // After the window manager took the window over.
                glib::timeout_add_local_once(Duration::from_millis(60), move || {
                    let Some(window) = window.upgrade() else {
                        return;
                    };
                    let wanted = unsafe { window.data::<Cell<bool>>(ABOVE) };
                    if let Some(wanted) = wanted
                        && unsafe { wanted.as_ref() }.get()
                    {
                        apply_above(&window, true);
                    }
                });
            });
        }
    }
    apply_above(window, enabled);
}
