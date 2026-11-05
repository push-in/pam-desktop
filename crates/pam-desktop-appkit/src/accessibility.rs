//! Maps the core's accessibility description to `NSAccessibility`.

use std::rc::Rc;

use block2::RcBlock;
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityAnnouncementKey,
    NSAccessibilityAnnouncementRequestedNotification, NSAccessibilityButtonRole,
    NSAccessibilityCheckBoxRole, NSAccessibilityComboBoxRole, NSAccessibilityCustomAction,
    NSAccessibilityDialogSubrole, NSAccessibilityGridRole, NSAccessibilityGroupRole,
    NSAccessibilityImageRole, NSAccessibilityIncrementorRole, NSAccessibilityLinkRole,
    NSAccessibilityListRole, NSAccessibilityMenuBarRole, NSAccessibilityMenuItemRole,
    NSAccessibilityMenuRole, NSAccessibilityPostNotificationWithUserInfo,
    NSAccessibilityPriorityKey, NSAccessibilityPriorityLevel, NSAccessibilityProgressIndicatorRole,
    NSAccessibilityRadioButtonRole, NSAccessibilityRadioGroupRole, NSAccessibilityScrollBarRole,
    NSAccessibilitySearchFieldSubrole, NSAccessibilitySliderRole, NSAccessibilityStaticTextRole,
    NSAccessibilitySwitchSubrole, NSAccessibilityTabButtonSubrole, NSAccessibilityTabGroupRole,
    NSAccessibilityTextFieldRole, NSAccessibilityToggleSubrole, NSAccessibilityToolbarRole, NSView,
    NSWindow, NSWorkspace,
};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};
use pam_desktop_ui::accessibility::{Accessibility, Checked, LiveRegion, Role};
use pam_desktop_ui::backend::{BackendEvent, NodeId};

use crate::events::Events;
use crate::util::ns_string;
use crate::view::PamView;

/// What was last applied to a view and must be compared or undone.
#[derive(Default)]
pub struct Applied {
    /// The accessible name, so live regions only speak when it changes.
    announced: String,
    /// The view's children were hidden from assistive technology.
    children_hidden: bool,
}

/// The AppKit role (and subrole) of a role.
fn native_role(role: Role) -> (&'static NSString, Option<&'static NSString>) {
    // SAFETY: the role and subrole names are immutable strings exported by AppKit.
    unsafe {
        match role {
            Role::Generic | Role::Alert | Role::Summary | Role::ListItem => {
                (NSAccessibilityGroupRole, None)
            }
            Role::Button => (NSAccessibilityButtonRole, None),
            Role::TextInput => (NSAccessibilityTextFieldRole, None),
            Role::Image => (NSAccessibilityImageRole, None),
            Role::Switch => (
                NSAccessibilityCheckBoxRole,
                Some(NSAccessibilitySwitchSubrole),
            ),
            Role::Slider => (NSAccessibilitySliderRole, None),
            Role::Checkbox => (NSAccessibilityCheckBoxRole, None),
            Role::ComboBox => (NSAccessibilityComboBoxRole, None),
            Role::Heading | Role::Text | Role::Timer => (NSAccessibilityStaticTextRole, None),
            Role::Link => (NSAccessibilityLinkRole, None),
            Role::Menu => (NSAccessibilityMenuRole, None),
            Role::MenuBar => (NSAccessibilityMenuBarRole, None),
            Role::MenuItem => (NSAccessibilityMenuItemRole, None),
            Role::ProgressBar => (NSAccessibilityProgressIndicatorRole, None),
            Role::Radio => (NSAccessibilityRadioButtonRole, None),
            Role::RadioGroup => (NSAccessibilityRadioGroupRole, None),
            Role::ScrollBar => (NSAccessibilityScrollBarRole, None),
            Role::Search => (
                NSAccessibilityTextFieldRole,
                Some(NSAccessibilitySearchFieldSubrole),
            ),
            Role::SpinButton => (NSAccessibilityIncrementorRole, None),
            Role::Tab => (
                NSAccessibilityRadioButtonRole,
                Some(NSAccessibilityTabButtonSubrole),
            ),
            Role::TabList => (NSAccessibilityTabGroupRole, None),
            Role::ToggleButton => (
                NSAccessibilityCheckBoxRole,
                Some(NSAccessibilityToggleSubrole),
            ),
            Role::Toolbar => (NSAccessibilityToolbarRole, None),
            Role::Grid => (NSAccessibilityGridRole, None),
            Role::List => (NSAccessibilityListRole, None),
            Role::Dialog => (NSAccessibilityGroupRole, Some(NSAccessibilityDialogSubrole)),
        }
    }
}

/// Roles whose default action is a press.
fn is_pressable(role: Role) -> bool {
    matches!(
        role,
        Role::Button
            | Role::Link
            | Role::Switch
            | Role::Checkbox
            | Role::Radio
            | Role::Tab
            | Role::MenuItem
            | Role::ToggleButton
            | Role::ComboBox
    )
}

/// Speaks `text` through the running screen reader.
fn announce(view: &NSView, text: &str, live: LiveRegion) {
    if let Some(window) = view.window() {
        announce_in(&window, text, live == LiveRegion::Assertive);
    }
}

/// Speaks `text` as an announcement of `window`.
pub fn announce_in(window: &NSWindow, text: &str, assertive: bool) {
    let priority = if assertive {
        NSAccessibilityPriorityLevel::High
    } else {
        NSAccessibilityPriorityLevel::Medium
    };
    let (text, priority) = (ns_string(text), NSNumber::new_isize(priority.0));
    // SAFETY: the notification name and keys are immutable strings exported
    // by AppKit; the announcement is a string and the priority a number,
    // as the notification documents. The element is the window.
    unsafe {
        let info: Retained<NSDictionary<NSString, AnyObject>> = NSDictionary::from_slices(
            &[NSAccessibilityAnnouncementKey, NSAccessibilityPriorityKey],
            &[text.as_ref(), priority.as_ref()],
        );
        let element: &AnyObject = window.as_ref();
        NSAccessibilityPostNotificationWithUserInfo(
            element,
            NSAccessibilityAnnouncementRequestedNotification,
            Some(&info),
        );
    }
}

/// Whether VoiceOver is running.
pub fn screen_reader_active() -> bool {
    NSWorkspace::sharedWorkspace().isVoiceOverEnabled()
}

/// Applies `accessibility` to the view of a node. `text` is what the view
/// shows, used as the name when the description brings none. `control` is
/// the native text control inside the view: it keeps its own role and only
/// takes the name and the state.
pub fn apply(
    view: &PamView,
    control: Option<&NSView>,
    id: NodeId,
    events: &Rc<Events>,
    accessibility: Option<&Accessibility>,
    text: Option<&str>,
    applied: &mut Applied,
) {
    let plain = Accessibility::default();
    let described = accessibility.unwrap_or(&plain);
    let name = if described.label.is_empty() {
        text.unwrap_or_default()
    } else {
        described.label.as_str()
    };
    let optional = |value: &str| (!value.is_empty()).then(|| ns_string(value));
    let target: &NSView = control.unwrap_or(view);

    // The name, the hint and the state apply to native controls too.
    target.setAccessibilityHelp(optional(&described.hint).as_deref());
    target.setAccessibilityIdentifier(optional(&described.identifier).as_deref());
    target.setAccessibilityEnabled(!described.disabled);
    if control.is_some() {
        target.setAccessibilityLabel(optional(name).as_deref());
        // The box around the control says nothing itself.
        view.setAccessibilityElement(false);
        view.set_pressable(false);
    } else {
        // A plain view that shows text is static text; one that shows
        // nothing is a group, which only matters when it has a name.
        let role = match accessibility {
            Some(accessibility) => accessibility.role,
            None if text.is_some() => Role::Text,
            None => Role::Generic,
        };
        let (native, subrole) = native_role(role);
        let exposed = !described.hidden
            && !described.hide_descendants
            && (role != Role::Generic
                || !name.is_empty()
                || !described.actions.is_empty()
                || described.live != LiveRegion::Off);
        view.setAccessibilityElement(exposed);
        view.setAccessibilityRole(Some(native));
        view.setAccessibilitySubrole(subrole);
        view.set_pressable(exposed && is_pressable(role) && !described.disabled);

        // Static text is read from its value; everything else has a label.
        let static_text = matches!(role, Role::Text | Role::Heading | Role::Timer);
        view.setAccessibilityLabel(if static_text { None } else { optional(name) }.as_deref());
        let value: Option<Retained<AnyObject>> = if static_text {
            optional(name).map(|name| Retained::into_super(Retained::into_super(name)))
        } else if let Some(checked) = described.checked {
            let state = match checked {
                Checked::Unchecked => 0,
                Checked::Checked => 1,
                Checked::Mixed => 2,
            };
            Some(Retained::into_super(Retained::into_super(
                Retained::into_super(NSNumber::new_isize(state)),
            )))
        } else if let Some((_, _, now)) = described.range {
            Some(Retained::into_super(Retained::into_super(
                Retained::into_super(NSNumber::new_f32(now)),
            )))
        } else {
            optional(&described.value_text)
                .map(|value| Retained::into_super(Retained::into_super(value)))
        };
        let number = |value: f32| -> Retained<AnyObject> {
            Retained::into_super(Retained::into_super(Retained::into_super(
                NSNumber::new_f32(value),
            )))
        };
        let (minimum, maximum) = match described.range {
            Some((minimum, maximum, _)) => (Some(number(minimum)), Some(number(maximum))),
            None => (None, None),
        };
        // SAFETY: values are strings or numbers, the classes assistive
        // technology expects for these attributes.
        unsafe {
            view.setAccessibilityValue(value.as_deref());
            view.setAccessibilityMinValue(minimum.as_deref());
            view.setAccessibilityMaxValue(maximum.as_deref());
        }
        let spoken_value = if described.range.is_some() || described.checked.is_some() {
            optional(&described.value_text)
        } else {
            None
        };
        view.setAccessibilityValueDescription(spoken_value.as_deref());
        view.setAccessibilitySelected(described.selected.unwrap_or(false));
        view.setAccessibilityExpanded(described.expanded.unwrap_or(false));

        // `hide_descendants`: neither the view nor anything inside it.
        if described.hide_descendants != applied.children_hidden {
            applied.children_hidden = described.hide_descendants;
            view.setAccessibilityHidden(described.hide_descendants);
            let none: Retained<NSArray> = NSArray::new();
            // SAFETY: an empty array hides the children; nil restores the
            // view's own list.
            unsafe { view.setAccessibilityChildren(described.hide_descendants.then_some(&*none)) };
        }

        if described.actions.is_empty() {
            view.setAccessibilityCustomActions(None);
        } else {
            let actions: Vec<Retained<NSAccessibilityCustomAction>> = described
                .actions
                .iter()
                .map(|action| {
                    let (events, name) = (Rc::clone(events), action.name.clone());
                    let handler = RcBlock::new(move || {
                        events.push(BackendEvent::AccessibilityAction {
                            id,
                            name: name.clone(),
                        });
                        Bool::YES
                    });
                    NSAccessibilityCustomAction::initWithName_handler(
                        NSAccessibilityCustomAction::alloc(),
                        &ns_string(&action.label),
                        Some(&handler),
                    )
                })
                .collect();
            view.setAccessibilityCustomActions(Some(&NSArray::from_retained_slice(&actions)));
        }
    }

    // Live regions speak their new content.
    if described.live != LiveRegion::Off && !name.is_empty() && name != applied.announced {
        announce(view, name, described.live);
    }
    applied.announced.clear();
    applied.announced.push_str(name);
}
