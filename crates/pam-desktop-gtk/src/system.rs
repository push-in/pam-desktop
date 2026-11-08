//! What the desktop says about appearance, text size and motion, and the
//! changes of each while the application runs.
//!
//! Dark mode is the `color-scheme` setting of the desktop portal (GNOME,
//! KDE and the others publish it there); a desktop without the portal falls
//! back to the GTK theme. Text size and animations are GTK settings.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use pam_desktop_ui::backend::BackendEvent;
use pam_desktop_ui::runtime::EventQueue;

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const APPEARANCE: &str = "org.freedesktop.appearance";
const COLOR_SCHEME: &str = "color-scheme";

/// `GTK_USE_PORTAL=0` keeps the application away from the portal, as it
/// does for GTK itself.
fn portal_allowed() -> bool {
    std::env::var("GTK_USE_PORTAL").map_or(true, |value| value != "0")
}

/// The number inside any nesting of variants.
fn unwrap_number(mut value: glib::Variant) -> Option<u32> {
    for _ in 0..4 {
        if let Some(number) = value.get::<u32>() {
            return Some(number);
        }
        // A boxed value and a tuple of one are both containers of one child.
        if !value.is_container() || value.n_children() != 1 {
            return None;
        }
        value = value.child_value(0);
    }
    None
}

/// `Some(true)` dark, `Some(false)` light, `None` when the desktop states
/// no preference or has no portal.
fn portal_prefers_dark() -> Option<bool> {
    if !portal_allowed() {
        return None;
    }
    let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok()?;
    let arguments = (APPEARANCE, COLOR_SCHEME).to_variant();
    // `ReadOne` replaced `Read` in version 2 of the interface.
    let reply = ["ReadOne", "Read"].into_iter().find_map(|method| {
        connection
            .call_sync(
                Some(PORTAL),
                PORTAL_PATH,
                SETTINGS,
                method,
                Some(&arguments),
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                250,
                gio::Cancellable::NONE,
            )
            .ok()
    })?;
    scheme_is_dark(unwrap_number(reply)?)
}

/// 1 prefers dark, 2 prefers light, 0 no preference.
fn scheme_is_dark(scheme: u32) -> Option<bool> {
    match scheme {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

fn theme_is_dark(settings: &gtk::Settings) -> bool {
    settings
        .gtk_theme_name()
        .is_some_and(|name| name.to_lowercase().contains("dark"))
}

/// Whether the desktop asks for a dark interface. `forced` says the
/// application overrides the GTK preference, which then tells nothing
/// about the desktop.
pub fn prefers_dark(forced: bool) -> bool {
    portal_prefers_dark().unwrap_or_else(|| {
        gtk::Settings::default().is_some_and(|settings| {
            theme_is_dark(&settings) || (!forced && settings.is_gtk_application_prefer_dark_theme())
        })
    })
}

/// GNOME's text scaling factor, published through the Xft DPI.
pub fn font_scale() -> f32 {
    gtk::Settings::default()
        .map(|settings| settings.gtk_xft_dpi())
        .filter(|dpi| *dpi > 0)
        .map_or(1.0, |dpi| (dpi as f32 / 1024.0 / 96.0).clamp(0.5, 4.0))
}

pub fn reduced_motion() -> bool {
    gtk::Settings::default().is_some_and(|settings| !settings.is_gtk_enable_animations())
}

/// Reports the changes of the desktop settings for as long as it lives.
pub struct Watch {
    _portal: Option<gio::SignalSubscription>,
    settings: Option<(gtk::Settings, Vec<glib::SignalHandlerId>)>,
}

impl Watch {
    pub fn new(events: &Rc<EventQueue>) -> Self {
        let portal = portal_allowed()
            .then(|| gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok())
            .flatten()
            .map(|connection| {
                let events = Rc::clone(events);
                // `SettingChanged(namespace, key, value)`.
                connection.subscribe_to_signal(
                    Some(PORTAL),
                    Some(SETTINGS),
                    Some("SettingChanged"),
                    Some(PORTAL_PATH),
                    Some(APPEARANCE),
                    gio::DBusSignalFlags::NONE,
                    move |signal| {
                        if signal.parameters.child_value(1).str() != Some(COLOR_SCHEME) {
                            return;
                        }
                        let dark = unwrap_number(signal.parameters.child_value(2))
                            .and_then(scheme_is_dark);
                        events.push(BackendEvent::AppearanceChanged {
                            dark: dark.unwrap_or_else(|| prefers_dark(true)),
                        });
                    },
                )
            });
        let settings = gtk::Settings::default().map(|settings| {
            let mut handlers = Vec::new();
            for property in ["gtk-xft-dpi", "gtk-enable-animations"] {
                let events = Rc::clone(events);
                handlers.push(settings.connect_notify_local(Some(property), move |_, _| {
                    events.push(BackendEvent::MetricsChanged)
                }));
            }
            // Without a portal the theme is all that tells light from dark.
            if portal.is_none() {
                let events = Rc::clone(events);
                handlers.push(settings.connect_notify_local(
                    Some("gtk-theme-name"),
                    move |settings, _| {
                        events.push(BackendEvent::AppearanceChanged {
                            dark: theme_is_dark(settings),
                        });
                    },
                ));
            }
            (settings, handlers)
        });
        Self {
            _portal: portal,
            settings,
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        if let Some((settings, handlers)) = self.settings.take() {
            for handler in handlers {
                settings.disconnect(handler);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_color_scheme_whatever_its_nesting() {
        // `ReadOne` answers `(v)`, `Read` answers `(v)` holding another variant.
        let one = glib::Variant::tuple_from_iter([1_u32.to_variant().to_variant()]);
        assert_eq!(unwrap_number(one), Some(1));
        let nested = glib::Variant::tuple_from_iter([2_u32.to_variant().to_variant().to_variant()]);
        assert_eq!(unwrap_number(nested), Some(2));
        assert_eq!(unwrap_number("dark".to_variant()), None);

        assert_eq!(scheme_is_dark(1), Some(true));
        assert_eq!(scheme_is_dark(2), Some(false));
        assert_eq!(scheme_is_dark(0), None);
    }
}
