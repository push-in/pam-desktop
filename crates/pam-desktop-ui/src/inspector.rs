//! The visual inspector of `pam-desktop dev` (F12 or Ctrl+Shift+I): the
//! node under the pointer is outlined and a card lists what the host knows
//! about it (kind, frame, every effective property, state styles, events).
//! A click pins the node (and prints the full description on stderr); F12
//! or Escape leaves. While it is on, the pointer never reaches the app.

use pam_desktop_wire::{NodeKind, PropKey, PropValue};

use crate::backend::{NodeId, PointerButton, PointerEvent, PointerPhase};
use crate::chrome::{Builder, Measure, TextStyle};
use crate::context::Ctx;
use crate::geometry::Rect;
use crate::props::PropMap;
use crate::scene::{Scene, StyleLayer};

const ACCENT: i64 = 0xFF25_63EB;
const FILL: i64 = 0x3325_63EB;
const SURFACE: i64 = 0xF014_1820;
const TEXT: i64 = 0xFFE6_EDF3;
const MUTED: i64 = 0xFF8B_949E;
const NONE: i64 = 2;

#[derive(Debug, Default)]
pub struct Inspector {
    enabled: bool,
    target: Option<NodeId>,
    pinned: bool,
    roots: Vec<NodeId>,
}

impl Inspector {
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn toggle(&mut self) {
        self.enabled = !self.enabled;
        self.target = None;
        self.pinned = false;
    }

    /// The pointer moved or clicked while inspecting; `true` when the
    /// overlay must be rebuilt.
    pub fn pointer(&mut self, scene: &Scene, event: &PointerEvent) -> bool {
        match event.phase {
            PointerPhase::Move if !self.pinned => {
                let target = inspectable(scene, scene.hit_test(event.position));
                std::mem::replace(&mut self.target, target) != target
            }
            PointerPhase::Down if event.button == PointerButton::Primary => {
                if self.pinned {
                    self.pinned = false;
                    self.target = inspectable(scene, scene.hit_test(event.position));
                } else if let Some(target) = self.target {
                    self.pinned = true;
                    eprintln!(
                        "pam-desktop inspector:\n  {}",
                        describe(scene, target).join("\n  ")
                    );
                }
                true
            }
            _ => false,
        }
    }

    /// The node being shown, if it is still in the scene.
    #[must_use]
    pub fn target(&self) -> Option<NodeId> {
        self.target
    }

    pub fn rebuild(&mut self, ctx: &mut Ctx<'_>, measure: Measure<'_>) {
        for root in self.roots.drain(..) {
            ctx.scene.remove_synthetic(root);
        }
        if !self.enabled {
            return;
        }
        if self.target.is_some_and(|id| ctx.scene.node(id).is_none()) {
            self.target = None;
            self.pinned = false;
        }
        let viewport = ctx.viewport;
        let lines = self
            .target
            .map(|id| (frame_of(ctx.scene, id), describe(ctx.scene, id)));
        let mut builder = Builder::new(ctx, measure);
        let banner_text = if self.pinned {
            "Inspector · pinned, click to release · F12 to close"
        } else {
            "Inspector · hover an element, click to pin · F12 to close"
        };
        let banner_style = TextStyle::new(12.0, 600, TEXT).lines(1);
        let banner_width = builder.text_width(banner_text, &banner_style) + 24.0;
        let banner = builder.view(
            0,
            NodeKind::View,
            Rect::new(
                (viewport.width - banner_width) / 2.0,
                8.0,
                banner_width,
                26.0,
            ),
            &[
                (PropKey::BackgroundColor, PropValue::Integer(ACCENT)),
                (PropKey::BorderRadius, PropValue::Float(13.0)),
                (PropKey::PointerEvents, PropValue::Integer(NONE)),
            ],
            None,
        );
        builder.text(
            banner,
            (viewport.width - banner_width) / 2.0 + 12.0,
            13.0,
            banner_width - 24.0,
            banner_text,
            &banner_style,
        );
        self.roots.push(banner);
        let Some((frame, lines)) = lines else {
            return;
        };
        let outline = builder.view(
            0,
            NodeKind::View,
            frame,
            &[
                (PropKey::BackgroundColor, PropValue::Integer(FILL)),
                (PropKey::BorderWidth, PropValue::Float(1.0)),
                (PropKey::BorderColor, PropValue::Integer(ACCENT)),
                (PropKey::PointerEvents, PropValue::Integer(NONE)),
            ],
            None,
        );
        self.roots.push(outline);

        let width = (viewport.width - 16.0).clamp(160.0, 380.0);
        let title = TextStyle::new(12.0, 700, TEXT).mono().lines(1);
        let body = TextStyle::new(11.0, 400, MUTED).mono();
        let inner = width - 20.0;
        let mut heights = vec![builder.text_height(inner, &lines[0], &title)];
        for line in &lines[1..] {
            heights.push(builder.text_height(inner, line, &body));
        }
        let height = heights.iter().sum::<f32>() + 16.0 + 2.0 * (lines.len() as f32 - 1.0);
        // Below the node when it fits, else above, else inside the window.
        let y = if frame.bottom() + 6.0 + height <= viewport.height {
            frame.bottom() + 6.0
        } else if frame.y - 6.0 - height >= 0.0 {
            frame.y - 6.0 - height
        } else {
            (viewport.height - height - 8.0).max(8.0)
        };
        let x = frame.x.clamp(8.0, (viewport.width - width - 8.0).max(8.0));
        let card = builder.view(
            0,
            NodeKind::View,
            Rect::new(x, y, width, height),
            &[
                (PropKey::BackgroundColor, PropValue::Integer(SURFACE)),
                (PropKey::BorderRadius, PropValue::Float(6.0)),
                (PropKey::BorderWidth, PropValue::Float(1.0)),
                (PropKey::BorderColor, PropValue::Integer(ACCENT)),
                (PropKey::PointerEvents, PropValue::Integer(NONE)),
            ],
            None,
        );
        let mut top = y + 8.0;
        for (index, line) in lines.iter().enumerate() {
            let style = if index == 0 { &title } else { &body };
            builder.text(card, x + 10.0, top, inner, line, style);
            top += heights[index] + 2.0;
        }
        self.roots.push(card);
    }

    #[must_use]
    pub fn roots(&self) -> &[NodeId] {
        &self.roots
    }
}

/// The nearest application node at or above `id` (chrome is skipped).
fn inspectable(scene: &Scene, mut id: Option<NodeId>) -> Option<NodeId> {
    while let Some(current) = id {
        let node = scene.node(current)?;
        if !node.is_synthetic() {
            return Some(current);
        }
        id = (node.parent != 0).then_some(node.parent);
    }
    None
}

fn frame_of(scene: &Scene, id: NodeId) -> Rect {
    let origin = scene.window_origin(id);
    let size = scene.node(id).map(|node| node.frame).unwrap_or_default();
    Rect::new(origin.x, origin.y, size.width, size.height)
}

/// The node's properties with its style layers applied, as the host uses them.
fn effective(scene: &Scene, id: NodeId) -> PropMap {
    let mut props = scene
        .node(id)
        .map(|node| node.props.clone())
        .unwrap_or_default();
    for layer in [
        StyleLayer::Presenter,
        StyleLayer::Interaction,
        StyleLayer::Transition,
    ] {
        if let Some(overlay) = scene.style_overlay(id, layer) {
            props.extend(overlay.iter().map(|(key, value)| (*key, value.clone())));
        }
    }
    props
}

/// The lines of the card: a title, then one line per property group.
#[must_use]
pub fn describe(scene: &Scene, id: NodeId) -> Vec<String> {
    let Some(node) = scene.node(id) else {
        return vec![String::from("(removed)")];
    };
    let frame = frame_of(scene, id);
    let props = effective(scene, id);
    let mut title = format!(
        "{:?}  {}×{}  at {},{}",
        node.kind,
        frame.width.round(),
        frame.height.round(),
        frame.x.round(),
        frame.y.round()
    );
    if let Some(PropValue::String(test_id)) = props.get(&PropKey::TestId) {
        title.push_str(&format!("  #{test_id}"));
    }
    let mut lines = vec![title];
    let mut events = Vec::new();
    let mut states = None;
    let mut fields = Vec::new();
    for (key, value) in &props {
        let name = format!("{key:?}");
        if let Some(event) = name.strip_prefix("On") {
            events.push(event.to_owned());
            continue;
        }
        match key {
            PropKey::NativeStateStyles => {
                states = Some(state_names(value));
                continue;
            }
            PropKey::TestId => continue,
            _ => {}
        }
        fields.push(format!("{name} {}", value_text(&name, value)));
    }
    // Several short fields per line, as wide as the card allows.
    let mut line = String::new();
    for field in fields {
        if !line.is_empty() && line.len() + field.len() + 3 > 52 {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push_str(" · ");
        }
        line.push_str(&field);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    if let Some(states) = states.filter(|states| !states.is_empty()) {
        lines.push(format!("states: {states}"));
    }
    if !events.is_empty() {
        lines.push(format!("events: {}", events.join(" ")));
    }
    let mut ancestors = Vec::new();
    let mut parent = node.parent;
    while parent != 0 && ancestors.len() < 6 {
        let Some(above) = scene.node(parent) else {
            break;
        };
        ancestors.push(format!("{:?}", above.kind));
        parent = above.parent;
    }
    if !ancestors.is_empty() {
        lines.push(format!("in: {}", ancestors.join(" < ")));
    }
    lines
}

fn value_text(name: &str, value: &PropValue) -> String {
    match value {
        PropValue::Integer(argb) if name.ends_with("Color") => format!("#{:08X}", *argb as u32),
        PropValue::Integer(number) => number.to_string(),
        PropValue::Float(number) => {
            let rounded = (number * 100.0).round() / 100.0;
            format!("{rounded}")
        }
        PropValue::Boolean(flag) => flag.to_string(),
        PropValue::String(text) => {
            let text = text.replace('\n', "⏎");
            if text.chars().count() > 40 {
                format!("\"{}…\"", text.chars().take(40).collect::<String>())
            } else {
                format!("\"{text}\"")
            }
        }
        PropValue::Bytes(bytes) => format!("<{} bytes>", bytes.len()),
    }
}

/// `{"3":{…},"5":{…}}` → `hover focus`.
fn state_names(value: &PropValue) -> String {
    let PropValue::String(json) = value else {
        return String::new();
    };
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(json) else {
        return String::new();
    };
    map.keys()
        .filter_map(|key| {
            Some(match key.as_str() {
                "1" => "pressed",
                "2" => "focus",
                "3" => "hover",
                "10" => "focus-within",
                "11" => "focus-visible",
                "12" => "group-hover",
                "g" => "hover-group",
                _ => return None,
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_read_like_css() {
        assert_eq!(
            value_text("BackgroundColor", &PropValue::Integer(0xFF11_2233)),
            "#FF112233"
        );
        assert_eq!(value_text("Opacity", &PropValue::Float(0.333_333)), "0.33");
        assert_eq!(
            state_names(&PropValue::String(r#"{"3":{},"g":1}"#.into())),
            "hover hover-group"
        );
    }
}
