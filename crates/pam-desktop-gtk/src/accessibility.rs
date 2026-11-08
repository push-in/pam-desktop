//! Maps the core's accessibility description to GTK's accessible API
//! (AT-SPI on Linux).

use std::ffi::{c_int, c_void};

use gtk::glib::translate::{IntoGlib, from_glib};
use gtk::prelude::*;
use gtk::{AccessibleProperty, AccessibleRole, AccessibleState, AccessibleTristate, glib};
use pam_desktop_ui::accessibility::{Accessibility, Checked, LiveRegion, Role};

// The accessible API is GTK 4.0, but gtk4-rs binds it from 4.10 only.
unsafe extern "C" {
    fn gtk_accessible_update_property_value(
        accessible: *mut c_void,
        n_properties: c_int,
        properties: *const c_int,
        values: *const glib::gobject_ffi::GValue,
    );
    fn gtk_accessible_update_state_value(
        accessible: *mut c_void,
        n_states: c_int,
        states: *const c_int,
        values: *const glib::gobject_ffi::GValue,
    );
    fn gtk_accessible_reset_property(accessible: *mut c_void, property: c_int);
    fn gtk_accessible_reset_state(accessible: *mut c_void, state: c_int);
}

/// `GTK_ACCESSIBLE_ROLE_TOGGLE_BUTTON`, new in GTK 4.10.
const TOGGLE_BUTTON: c_int = 78;

fn update_properties(widget: &gtk::Widget, properties: &[(AccessibleProperty, glib::Value)]) {
    let (names, values): (Vec<c_int>, Vec<glib::Value>) = properties
        .iter()
        .map(|(name, value)| (name.into_glib(), value.clone()))
        .unzip();
    // SAFETY: a widget is an accessible; `glib::Value` is a `GValue`.
    unsafe {
        gtk_accessible_update_property_value(
            widget.as_ptr().cast(),
            names.len() as c_int,
            names.as_ptr(),
            values.as_ptr().cast(),
        );
    }
}

fn update_states(widget: &gtk::Widget, states: &[(AccessibleState, glib::Value)]) {
    let (names, values): (Vec<c_int>, Vec<glib::Value>) = states
        .iter()
        .map(|(name, value)| (name.into_glib(), value.clone()))
        .unzip();
    // SAFETY: a widget is an accessible; `glib::Value` is a `GValue`.
    unsafe {
        gtk_accessible_update_state_value(
            widget.as_ptr().cast(),
            names.len() as c_int,
            names.as_ptr(),
            values.as_ptr().cast(),
        );
    }
}

fn reset_property(widget: &gtk::Widget, property: AccessibleProperty) {
    // SAFETY: a widget is an accessible.
    unsafe { gtk_accessible_reset_property(widget.as_ptr().cast(), property.into_glib()) };
}

fn reset_state(widget: &gtk::Widget, state: AccessibleState) {
    // SAFETY: a widget is an accessible.
    unsafe { gtk_accessible_reset_state(widget.as_ptr().cast(), state.into_glib()) };
}

/// A boolean state that may also be undefined (-1).
fn optional(value: Option<bool>) -> glib::Value {
    value.map_or(-1, i32::from).to_value()
}

fn role(role: Role, hidden: bool) -> AccessibleRole {
    if hidden {
        return AccessibleRole::Presentation;
    }
    match role {
        Role::Generic => AccessibleRole::Group,
        Role::Button => AccessibleRole::Button,
        Role::TextInput => AccessibleRole::TextBox,
        Role::Image => AccessibleRole::Img,
        Role::Switch => AccessibleRole::Switch,
        Role::Slider => AccessibleRole::Slider,
        Role::Alert => AccessibleRole::Alert,
        Role::Checkbox => AccessibleRole::Checkbox,
        Role::ComboBox => AccessibleRole::ComboBox,
        Role::Heading => AccessibleRole::Heading,
        Role::Link => AccessibleRole::Link,
        Role::Menu => AccessibleRole::Menu,
        Role::MenuBar => AccessibleRole::MenuBar,
        Role::MenuItem => AccessibleRole::MenuItem,
        Role::ProgressBar => AccessibleRole::ProgressBar,
        Role::Radio => AccessibleRole::Radio,
        Role::RadioGroup => AccessibleRole::RadioGroup,
        Role::ScrollBar => AccessibleRole::Scrollbar,
        Role::Search => AccessibleRole::SearchBox,
        Role::SpinButton => AccessibleRole::SpinButton,
        Role::Summary => AccessibleRole::Note,
        Role::Tab => AccessibleRole::Tab,
        Role::TabList => AccessibleRole::TabList,
        Role::Text => AccessibleRole::Label,
        Role::Timer => AccessibleRole::Timer,
        // GTK before 4.10 has no toggle button role.
        Role::ToggleButton if gtk::check_version(4, 10, 0).is_none() => {
            // SAFETY: a role of the running GTK.
            unsafe { from_glib(TOGGLE_BUTTON) }
        }
        Role::ToggleButton => AccessibleRole::Button,
        Role::Toolbar => AccessibleRole::Toolbar,
        Role::Grid => AccessibleRole::Grid,
        Role::List => AccessibleRole::List,
        Role::ListItem => AccessibleRole::ListItem,
        Role::Dialog => AccessibleRole::Dialog,
    }
}

/// Applies `accessibility` to a view. `text` is what the view shows, used
/// as the name when the description brings none. `announced` is the name
/// applied before, so live regions only speak when it changes. `native`
/// marks a platform control, which keeps its own role.
pub fn apply(
    widget: &gtk::Widget,
    accessibility: Option<&Accessibility>,
    text: Option<&str>,
    announced: &mut String,
    native: bool,
) {
    let plain = Accessibility::default();
    let described = accessibility.unwrap_or(&plain);
    let wanted = match accessibility {
        Some(accessibility) => role(accessibility.role, accessibility.hidden),
        // A plain view that shows text is static text.
        None if text.is_some() => AccessibleRole::Label,
        None => AccessibleRole::Group,
    };
    // GTK fixes the role once the accessible context is in use, and native
    // controls keep the role they were built with.
    if !native
        && widget.property::<AccessibleRole>("accessible-role") != wanted
        && !widget.is_realized()
    {
        widget.set_property("accessible-role", wanted);
    }
    let name = if described.label.is_empty() {
        text.unwrap_or_default()
    } else {
        described.label.as_str()
    };
    if name.is_empty() {
        reset_property(widget, AccessibleProperty::Label);
    } else {
        update_properties(widget, &[(AccessibleProperty::Label, name.to_value())]);
    }
    if described.hint.is_empty() {
        reset_property(widget, AccessibleProperty::Description);
    } else {
        update_properties(
            widget,
            &[(AccessibleProperty::Description, described.hint.to_value())],
        );
    }
    match described.range {
        Some((minimum, maximum, now)) => update_properties(
            widget,
            &[
                (AccessibleProperty::ValueMin, f64::from(minimum).to_value()),
                (AccessibleProperty::ValueMax, f64::from(maximum).to_value()),
                (AccessibleProperty::ValueNow, f64::from(now).to_value()),
            ],
        ),
        None => {
            reset_property(widget, AccessibleProperty::ValueMin);
            reset_property(widget, AccessibleProperty::ValueMax);
            reset_property(widget, AccessibleProperty::ValueNow);
        }
    }
    if described.value_text.is_empty() {
        reset_property(widget, AccessibleProperty::ValueText);
    } else {
        update_properties(
            widget,
            &[(
                AccessibleProperty::ValueText,
                described.value_text.to_value(),
            )],
        );
    }
    match described.checked {
        Some(checked) => {
            let checked = match checked {
                Checked::Unchecked => AccessibleTristate::False,
                Checked::Checked => AccessibleTristate::True,
                Checked::Mixed => AccessibleTristate::Mixed,
            };
            update_states(
                widget,
                &[(AccessibleState::Checked, checked.into_glib().to_value())],
            );
        }
        None => reset_state(widget, AccessibleState::Checked),
    }
    match described.selected {
        Some(selected) => update_states(
            widget,
            &[(AccessibleState::Selected, optional(Some(selected)))],
        ),
        None => reset_state(widget, AccessibleState::Selected),
    }
    match described.expanded {
        Some(expanded) => update_states(
            widget,
            &[(AccessibleState::Expanded, optional(Some(expanded)))],
        ),
        None => reset_state(widget, AccessibleState::Expanded),
    }
    update_states(
        widget,
        &[
            (AccessibleState::Disabled, described.disabled.to_value()),
            (AccessibleState::Busy, described.busy.to_value()),
            (
                AccessibleState::Hidden,
                described.hide_descendants.to_value(),
            ),
        ],
    );

    // Live regions speak their new content.
    if described.live != LiveRegion::Off
        && !name.is_empty()
        && name != announced.as_str()
        && widget.is_realized()
    {
        crate::compat::announce(widget, name, described.live == LiveRegion::Assertive);
    }
    announced.clear();
    announced.push_str(name);
}

/// Whether a screen reader is running (`org.a11y.Status`), asked at most
/// once every few seconds.
pub fn screen_reader_active() -> bool {
    use std::cell::Cell;
    use std::time::{Duration, Instant};

    thread_local! {
        static KNOWN: Cell<Option<(Instant, bool)>> = const { Cell::new(None) };
    }
    if let Some((asked, active)) = KNOWN.get()
        && asked.elapsed() < Duration::from_secs(5)
    {
        return active;
    }
    let active = gtk::gio::bus_get_sync(gtk::gio::BusType::Session, gtk::gio::Cancellable::NONE)
        .ok()
        .and_then(|connection| {
            connection
                .call_sync(
                    Some("org.a11y.Bus"),
                    "/org/a11y/bus",
                    "org.freedesktop.DBus.Properties",
                    "Get",
                    Some(&("org.a11y.Status", "ScreenReaderEnabled").to_variant()),
                    Some(gtk::glib::VariantTy::new("(v)").ok()?),
                    gtk::gio::DBusCallFlags::NO_AUTO_START,
                    250,
                    gtk::gio::Cancellable::NONE,
                )
                .ok()
        })
        .and_then(|reply| reply.child_value(0).as_variant()?.get::<bool>())
        .unwrap_or(false);
    KNOWN.set(Some((Instant::now(), active)));
    active
}
