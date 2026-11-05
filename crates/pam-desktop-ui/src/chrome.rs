//! Building blocks of the host's own chrome (error reports, toasts): small
//! trees of synthetic nodes laid out by hand, with text measured by the
//! platform so paragraphs wrap exactly as they are drawn.

use pam_desktop_wire::{NodeKind, PropKey, PropValue};

use crate::backend::NodeId;
use crate::context::Ctx;
use crate::geometry::Rect;
use crate::props::PropMap;
use crate::scene::Action;
use crate::text::{TextBlock, TextMetrics};

/// The fixed-pitch family of each system.
pub const MONOSPACE: &str = if cfg!(target_os = "macos") {
    "Menlo"
} else if cfg!(target_os = "windows") {
    "Consolas"
} else {
    "monospace"
};

/// Measures a paragraph with the platform's text engine.
pub type Measure<'a> = &'a mut dyn FnMut(&TextBlock, f32) -> TextMetrics;

/// Builds synthetic nodes with frames in window coordinates.
pub struct Builder<'a, 'b, 'c> {
    ctx: &'a mut Ctx<'b>,
    measure: Measure<'c>,
    order: u32,
}

pub struct TextStyle {
    pub size: f32,
    pub weight: i64,
    pub color: i64,
    pub mono: bool,
    pub max_lines: i64,
    pub centered: bool,
    /// A font family; empty keeps the system's.
    pub family: String,
}

impl TextStyle {
    #[must_use]
    pub const fn new(size: f32, weight: i64, color: i64) -> Self {
        Self { size, weight, color, mono: false, max_lines: 0, centered: false, family: String::new() }
    }

    #[must_use]
    pub const fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    #[must_use]
    pub const fn lines(mut self, lines: i64) -> Self {
        self.max_lines = lines;
        self
    }

    #[must_use]
    pub const fn centered(mut self) -> Self {
        self.centered = true;
        self
    }
}

impl<'a, 'b, 'c> Builder<'a, 'b, 'c> {
    pub fn new(ctx: &'a mut Ctx<'b>, measure: Measure<'c>) -> Self {
        Self { ctx, measure, order: 0 }
    }

    pub fn set_frame(&mut self, id: NodeId, frame: Rect) {
        self.ctx.scene.set_synthetic_frame(id, frame);
    }

    /// Height of a paragraph in `style` wrapped to `width`, without adding it.
    pub fn text_height(&mut self, width: f32, content: &str, style: &TextStyle) -> f32 {
        let id = self.text_node(0, 0.0, 0.0, width, content, style);
        let height = self.ctx.scene.text_block(id).map_or(style.size * 1.4, |block| (self.measure)(&block, width).height);
        self.ctx.scene.remove_synthetic(id);
        height
    }

    /// Width a single-line paragraph needs, without adding it.
    pub fn text_width(&mut self, content: &str, style: &TextStyle) -> f32 {
        let id = self.text_node(0, 0.0, 0.0, 10_000.0, content, style);
        let width = self.ctx.scene.text_block(id).map_or(0.0, |block| (self.measure)(&block, f32::INFINITY).width);
        self.ctx.scene.remove_synthetic(id);
        width
    }

    pub fn view(&mut self, parent: NodeId, kind: NodeKind, frame: Rect, props: &[(PropKey, PropValue)], command: Option<u16>) -> NodeId {
        self.order += 1;
        let map: PropMap = props.iter().cloned().collect();
        self.ctx.scene.add_synthetic(parent, kind, map, frame, self.order, command.map(|command| Action::Chrome { command }))
    }

    fn text_node(&mut self, parent: NodeId, x: f32, y: f32, width: f32, content: &str, style: &TextStyle) -> NodeId {
        let mut props = vec![
            (PropKey::Text, PropValue::String(content.to_owned())),
            (PropKey::FontSize, PropValue::Float(f64::from(style.size))),
            (PropKey::FontWeight, PropValue::Integer(style.weight)),
            (PropKey::TextColor, PropValue::Integer(style.color)),
            // Chrome never takes the pointer away from the row it sits in.
            (PropKey::PointerEvents, PropValue::Integer(2)),
        ];
        if style.mono {
            props.push((PropKey::FontFamily, PropValue::String(MONOSPACE.to_owned())));
        } else if !style.family.is_empty() {
            props.push((PropKey::FontFamily, PropValue::String(style.family.clone())));
        }
        if style.max_lines > 0 {
            props.push((PropKey::NumberOfLines, PropValue::Integer(style.max_lines)));
        }
        if style.centered {
            props.push((PropKey::TextAlign, PropValue::Integer(2)));
        }
        self.view(parent, NodeKind::Text, Rect::new(x, y, width, style.size * 1.4), &props, None)
    }

    /// A paragraph at `(x, y)` wrapped to `width`; returns its height.
    pub fn text(&mut self, parent: NodeId, x: f32, y: f32, width: f32, content: &str, style: &TextStyle) -> f32 {
        let id = self.text_node(parent, x, y, width, content, style);
        let height = self.ctx.scene.text_block(id).map_or(style.size * 1.4, |block| (self.measure)(&block, width).height);
        self.ctx.scene.set_synthetic_frame(id, Rect::new(x, y, width, height));
        height
    }

    /// A push button with a centered label.
    pub fn button(&mut self, parent: NodeId, frame: Rect, label: &str, background: i64, color: i64, border: Option<i64>, command: u16) {
        let mut props = vec![
            (PropKey::Text, PropValue::String(label.to_owned())),
            (PropKey::FontSize, PropValue::Float(14.0)),
            (PropKey::FontWeight, PropValue::Integer(600)),
            (PropKey::TextColor, PropValue::Integer(color)),
            (PropKey::TextAlign, PropValue::Integer(2)),
            (PropKey::BackgroundColor, PropValue::Integer(background)),
            (PropKey::BorderRadius, PropValue::Float(10.0)),
            (PropKey::Cursor, PropValue::String("pointer".into())),
            (PropKey::AccessibilityRole, PropValue::Integer(2)),
        ];
        if let Some(border) = border {
            props.push((PropKey::BorderWidth, PropValue::Float(1.0)));
            props.push((PropKey::BorderColor, PropValue::Integer(border)));
        }
        self.view(parent, NodeKind::Text, frame, &props, Some(command));
    }
}

