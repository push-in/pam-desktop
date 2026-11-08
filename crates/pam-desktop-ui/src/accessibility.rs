//! What assistive technology is told about a node: role, name, state and
//! the custom actions it offers. Resolved here from the protocol
//! properties; backends only map it to the platform's accessibility API.

use std::hash::{Hash, Hasher};

use pam_desktop_wire::{NodeKind, PropKey};

use crate::props::Props;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Role {
    /// A container without meaning of its own.
    #[default]
    Generic,
    Button,
    TextInput,
    Image,
    Switch,
    Slider,
    Alert,
    Checkbox,
    ComboBox,
    Heading,
    Link,
    Menu,
    MenuBar,
    MenuItem,
    ProgressBar,
    Radio,
    RadioGroup,
    ScrollBar,
    Search,
    SpinButton,
    /// A note or summary region.
    Summary,
    Tab,
    TabList,
    /// Static text.
    Text,
    Timer,
    ToggleButton,
    Toolbar,
    Grid,
    List,
    ListItem,
    Dialog,
}

impl Role {
    /// The role of an `AccessibilityRole` property value; `None` for the
    /// roles that hide the element (17 None, 34 Presentation) and unknown ids.
    fn from_wire(value: i64) -> Option<Self> {
        Some(match value {
            1 => Self::Generic,
            2 | 11 | 12 => Self::Button,
            3 => Self::TextInput,
            4 => Self::Image,
            5 => Self::Switch,
            6 => Self::Slider,
            7 => Self::Alert,
            8 => Self::Checkbox,
            9 => Self::ComboBox,
            10 => Self::Heading,
            13 => Self::Link,
            14 => Self::Menu,
            15 => Self::MenuBar,
            16 => Self::MenuItem,
            18 => Self::ProgressBar,
            19 => Self::Radio,
            20 => Self::RadioGroup,
            21 => Self::ScrollBar,
            22 => Self::Search,
            23 => Self::SpinButton,
            24 => Self::Summary,
            25 => Self::Tab,
            26 => Self::TabList,
            27 => Self::Text,
            28 => Self::Timer,
            29 => Self::ToggleButton,
            30 => Self::Toolbar,
            31 => Self::Grid,
            32 => Self::List,
            33 => Self::ListItem,
            _ => return None,
        })
    }

    #[must_use]
    pub fn is_checkable(self) -> bool {
        matches!(
            self,
            Self::Switch | Self::Checkbox | Self::Radio | Self::ToggleButton
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Checked {
    Unchecked,
    Checked,
    Mixed,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum LiveRegion {
    #[default]
    Off,
    Polite,
    Assertive,
}

/// A custom action offered to assistive technology.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Action {
    /// Reported back in the `AccessibilityAction` event.
    pub name: String,
    /// What the user reads.
    pub label: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Accessibility {
    pub role: Role,
    /// The node is not exposed (its children still are).
    pub hidden: bool,
    /// Neither the node nor anything inside it is exposed.
    pub hide_descendants: bool,
    /// The accessible name. Empty on a view that shows text means "the text".
    pub label: String,
    pub hint: String,
    /// Stable identifier for UI automation.
    pub identifier: String,
    pub value_text: String,
    /// Minimum, maximum and current value of a range.
    pub range: Option<(f32, f32, f32)>,
    pub checked: Option<Checked>,
    pub selected: Option<bool>,
    pub expanded: Option<bool>,
    pub disabled: bool,
    pub busy: bool,
    pub live: LiveRegion,
    pub actions: Vec<Action>,
}

impl Accessibility {
    /// A value that changes whenever anything a backend maps changes.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.role.hash(&mut hasher);
        (self.hidden, self.hide_descendants, self.disabled, self.busy).hash(&mut hasher);
        (&self.label, &self.hint, &self.identifier, &self.value_text).hash(&mut hasher);
        self.range
            .map(|(minimum, maximum, now)| (minimum.to_bits(), maximum.to_bits(), now.to_bits()))
            .hash(&mut hasher);
        (self.checked, self.selected, self.expanded, self.live).hash(&mut hasher);
        self.actions.hash(&mut hasher);
        // Zero is reserved for "never pushed".
        hasher.finish().max(1)
    }

    /// Resolves the accessibility of a node. `pressable` tells whether it
    /// reacts to a press; `None` means there is nothing to say beyond what
    /// the backend assumes for a plain view.
    #[must_use]
    pub fn resolve(kind: NodeKind, props: Props<'_>, pressable: bool) -> Option<Self> {
        let authored = props.integer(PropKey::AccessibilityRole);
        let importance = props.integer_or(PropKey::AccessibilityImportance, 1);
        let label = props.text_or_empty(PropKey::AccessibilityLabel);
        let default_role = match kind {
            NodeKind::Text => Role::Text,
            NodeKind::Button => Role::Button,
            NodeKind::Input => Role::TextInput,
            NodeKind::Image => Role::Image,
            NodeKind::Switch => Role::Switch,
            NodeKind::ActivityIndicator => Role::ProgressBar,
            NodeKind::Modal => Role::Dialog,
            NodeKind::List | NodeKind::SectionList | NodeKind::VirtualList => Role::List,
            _ if pressable => Role::Button,
            _ => Role::Generic,
        };
        let role = match authored {
            Some(value) => Role::from_wire(value),
            None => Some(default_role),
        };
        let mut hidden = role.is_none()
            || importance == 3
            || importance == 4
            || props.flag(PropKey::Accessible) == Some(false);
        // A picture without a description is decoration.
        if kind == NodeKind::Image
            && authored.is_none()
            && label.is_empty()
            && props.flag(PropKey::Accessible) != Some(true)
        {
            hidden = true;
        }
        let role = role.unwrap_or(Role::Generic);
        let checked = match props.integer(PropKey::AccessibilityCheckedState) {
            Some(2) => Some(Checked::Checked),
            Some(3) => Some(Checked::Mixed),
            Some(_) => Some(Checked::Unchecked),
            None if role.is_checkable() => {
                let on = props
                    .flag(PropKey::Checked)
                    .or_else(|| props.flag(PropKey::Value))
                    .unwrap_or(false);
                Some(if on {
                    Checked::Checked
                } else {
                    Checked::Unchecked
                })
            }
            None => None,
        };
        let range = match (
            props.number(PropKey::AccessibilityValueMin),
            props.number(PropKey::AccessibilityValueMax),
            props.number(PropKey::AccessibilityValueNow),
        ) {
            (Some(minimum), Some(maximum), Some(now)) if minimum <= now && now <= maximum => {
                Some((minimum, maximum, now))
            }
            _ => None,
        };
        let actions = if props.has(PropKey::OnAccessibilityAction) {
            parse_actions(props.text_or_empty(PropKey::AccessibilityActions))
        } else {
            Vec::new()
        };
        let resolved = Self {
            role,
            hidden,
            hide_descendants: importance == 4,
            label: label.to_owned(),
            hint: props.text_or_empty(PropKey::AccessibilityHint).to_owned(),
            identifier: props.text_or_empty(PropKey::TestId).to_owned(),
            value_text: props
                .text_or_empty(PropKey::AccessibilityValueText)
                .to_owned(),
            range,
            checked,
            selected: props
                .flag(PropKey::AccessibilitySelected)
                .or_else(|| props.flag(PropKey::Selected)),
            expanded: props.flag(PropKey::AccessibilityExpanded),
            disabled: props.flag(PropKey::Enabled) == Some(false),
            busy: props.flag(PropKey::AccessibilityBusy) == Some(true)
                || props.flag(PropKey::Loading) == Some(true),
            live: match props.integer_or(PropKey::AccessibilityLiveRegion, 1) {
                2 => LiveRegion::Polite,
                3 => LiveRegion::Assertive,
                _ => LiveRegion::Off,
            },
            actions,
        };
        (resolved != Self::default()).then_some(resolved)
    }
}

/// `AccessibilityActions`: a JSON array of `{name, label}`. Any invalid
/// entry rejects the whole list, like the other hosts do.
fn parse_actions(json: &str) -> Vec<Action> {
    if json.is_empty() || json.len() > 4096 {
        return Vec::new();
    }
    let Ok(serde_json::Value::Array(entries)) = serde_json::from_str::<serde_json::Value>(json)
    else {
        return Vec::new();
    };
    if entries.is_empty() || entries.len() > 8 {
        return Vec::new();
    }
    let mut actions: Vec<Action> = Vec::with_capacity(entries.len());
    for entry in &entries {
        let Some(object) = entry.as_object().filter(|object| object.len() == 2) else {
            return Vec::new();
        };
        let (Some(name), Some(label)) = (
            object.get("name").and_then(serde_json::Value::as_str),
            object.get("label").and_then(serde_json::Value::as_str),
        ) else {
            return Vec::new();
        };
        let valid_name = name.len() <= 64
            && name
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_lowercase())
            && name.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'_' | b'-')
            });
        if !valid_name
            || label.is_empty()
            || label.len() > 128
            || actions.iter().any(|action| action.name == name)
        {
            return Vec::new();
        }
        actions.push(Action {
            name: name.to_owned(),
            label: label.to_owned(),
        });
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::props::PropMap;
    use pam_desktop_wire::PropValue;

    fn resolve(
        kind: NodeKind,
        entries: &[(PropKey, PropValue)],
        pressable: bool,
    ) -> Option<Accessibility> {
        let map: PropMap = entries.iter().cloned().collect();
        Accessibility::resolve(kind, Props::new(&map), pressable)
    }

    #[test]
    fn plain_views_have_nothing_to_say() {
        assert_eq!(resolve(NodeKind::View, &[], false), None);
        assert_eq!(
            resolve(
                NodeKind::Column,
                &[(PropKey::Width, PropValue::Float(10.0))],
                false
            ),
            None
        );
    }

    #[test]
    fn kinds_bring_their_role() {
        assert_eq!(
            resolve(NodeKind::Text, &[], false).map(|a| a.role),
            Some(Role::Text)
        );
        assert_eq!(
            resolve(NodeKind::View, &[], true).map(|a| a.role),
            Some(Role::Button)
        );
        let switch = resolve(
            NodeKind::Switch,
            &[(PropKey::Checked, PropValue::Boolean(true))],
            true,
        )
        .unwrap_or_default();
        assert_eq!(
            (switch.role, switch.checked),
            (Role::Switch, Some(Checked::Checked))
        );
        // Undescribed pictures are decoration; described ones are images.
        assert!(resolve(NodeKind::Image, &[], false).is_some_and(|a| a.hidden));
        let described = resolve(
            NodeKind::Image,
            &[(
                PropKey::AccessibilityLabel,
                PropValue::String("Logo".into()),
            )],
            false,
        )
        .unwrap_or_default();
        assert_eq!(
            (described.hidden, described.role, described.label.as_str()),
            (false, Role::Image, "Logo")
        );
    }

    #[test]
    fn authored_properties_win() {
        let resolved = resolve(
            NodeKind::View,
            &[
                (PropKey::AccessibilityRole, PropValue::Integer(10)),
                (
                    PropKey::AccessibilityHint,
                    PropValue::String("Opens the menu".into()),
                ),
                (PropKey::TestId, PropValue::String("title".into())),
                (PropKey::AccessibilityExpanded, PropValue::Boolean(false)),
                (PropKey::AccessibilityLiveRegion, PropValue::Integer(3)),
                (PropKey::AccessibilityValueMin, PropValue::Float(0.0)),
                (PropKey::AccessibilityValueMax, PropValue::Float(10.0)),
                (PropKey::AccessibilityValueNow, PropValue::Float(4.0)),
                (PropKey::Enabled, PropValue::Boolean(false)),
            ],
            false,
        )
        .unwrap_or_default();
        assert_eq!(resolved.role, Role::Heading);
        assert_eq!(resolved.hint, "Opens the menu");
        assert_eq!(resolved.identifier, "title");
        assert_eq!(resolved.expanded, Some(false));
        assert_eq!(resolved.live, LiveRegion::Assertive);
        assert_eq!(resolved.range, Some((0.0, 10.0, 4.0)));
        assert!(resolved.disabled);
    }

    #[test]
    fn hiding_roles_and_importance() {
        assert!(
            resolve(
                NodeKind::Text,
                &[(PropKey::AccessibilityRole, PropValue::Integer(17))],
                false
            )
            .is_some_and(|a| a.hidden)
        );
        let subtree = resolve(
            NodeKind::View,
            &[(PropKey::AccessibilityImportance, PropValue::Integer(4))],
            false,
        )
        .unwrap_or_default();
        assert!(subtree.hidden && subtree.hide_descendants);
    }

    #[test]
    fn actions_need_the_handler_and_a_valid_list() {
        let list = PropValue::String(
            r#"[{"name":"archive","label":"Archive"},{"name":"mark.read","label":"Mark as read"}]"#
                .into(),
        );
        assert!(
            resolve(
                NodeKind::View,
                &[(PropKey::AccessibilityActions, list.clone())],
                false
            )
            .is_none()
        );
        let resolved = resolve(
            NodeKind::View,
            &[
                (PropKey::AccessibilityActions, list),
                (PropKey::OnAccessibilityAction, PropValue::Boolean(true)),
            ],
            false,
        )
        .unwrap_or_default();
        assert_eq!(resolved.actions.len(), 2);
        assert_eq!(
            resolved.actions[1],
            Action {
                name: "mark.read".into(),
                label: "Mark as read".into()
            }
        );
        assert!(parse_actions(r#"[{"name":"Bad","label":"x"}]"#).is_empty());
        assert!(parse_actions(r#"[{"name":"a","label":"x"},{"name":"a","label":"y"}]"#).is_empty());
        assert!(parse_actions(r#"[{"name":"a","label":"x","extra":1}]"#).is_empty());
    }

    #[test]
    fn fingerprints_follow_changes() {
        let a = Accessibility {
            label: "Save".into(),
            role: Role::Button,
            ..Accessibility::default()
        };
        let mut b = a.clone();
        assert_eq!(a.fingerprint(), b.fingerprint());
        b.disabled = true;
        assert_ne!(a.fingerprint(), b.fingerprint());
    }
}
