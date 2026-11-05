//! Maps the core's accessibility description to GTK's accessible API
//! (AT-SPI on Linux).

use gtk::accessible::{Property, State};
use gtk::prelude::*;
use gtk::{AccessibleProperty, AccessibleRole, AccessibleState, AccessibleTristate};
use pam_desktop_ui::accessibility::{Accessibility, Checked, LiveRegion, Role};

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
        Role::ToggleButton => AccessibleRole::ToggleButton,
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
pub fn apply(widget: &gtk::Widget, accessibility: Option<&Accessibility>, text: Option<&str>, announced: &mut String, native: bool) {
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
    if !native && widget.accessible_role() != wanted && !widget.is_realized() {
        widget.set_accessible_role(wanted);
    }
    let name = if described.label.is_empty() { text.unwrap_or_default() } else { described.label.as_str() };
    if name.is_empty() {
        widget.reset_property(AccessibleProperty::Label);
    } else {
        widget.update_property(&[Property::Label(name)]);
    }
    if described.hint.is_empty() {
        widget.reset_property(AccessibleProperty::Description);
    } else {
        widget.update_property(&[Property::Description(&described.hint)]);
    }
    match described.range {
        Some((minimum, maximum, now)) => {
            widget.update_property(&[Property::ValueMin(f64::from(minimum)), Property::ValueMax(f64::from(maximum)), Property::ValueNow(f64::from(now))]);
        }
        None => {
            widget.reset_property(AccessibleProperty::ValueMin);
            widget.reset_property(AccessibleProperty::ValueMax);
            widget.reset_property(AccessibleProperty::ValueNow);
        }
    }
    if described.value_text.is_empty() {
        widget.reset_property(AccessibleProperty::ValueText);
    } else {
        widget.update_property(&[Property::ValueText(&described.value_text)]);
    }
    match described.checked {
        Some(checked) => widget.update_state(&[State::Checked(match checked {
            Checked::Unchecked => AccessibleTristate::False,
            Checked::Checked => AccessibleTristate::True,
            Checked::Mixed => AccessibleTristate::Mixed,
        })]),
        None => widget.reset_state(AccessibleState::Checked),
    }
    match described.selected {
        Some(selected) => widget.update_state(&[State::Selected(Some(selected))]),
        None => widget.reset_state(AccessibleState::Selected),
    }
    match described.expanded {
        Some(expanded) => widget.update_state(&[State::Expanded(Some(expanded))]),
        None => widget.reset_state(AccessibleState::Expanded),
    }
    widget.update_state(&[State::Disabled(described.disabled), State::Busy(described.busy), State::Hidden(described.hide_descendants)]);

    // Live regions speak their new content.
    if described.live != LiveRegion::Off && !name.is_empty() && name != announced.as_str() && widget.is_realized() {
        let priority = match described.live {
            LiveRegion::Assertive => gtk::AccessibleAnnouncementPriority::High,
            _ => gtk::AccessibleAnnouncementPriority::Medium,
        };
        widget.announce(name, priority);
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
