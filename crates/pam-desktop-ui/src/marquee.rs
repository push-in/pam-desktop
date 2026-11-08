//! `TextEllipsizeMode::Marquee`: a line that does not fit its box slides
//! from its start to its end and back, resting at each end.
//!
//! The text node stops painting its own text. Inside its padding sits a
//! clipping box with a copy as wide as the whole line, which the frame
//! clock moves. Nothing moves while the
//! line rests, while it is off screen or hidden, or under reduced motion
//! (the line is then cut with an ellipsis, like `Tail`).

use pam_desktop_wire::{NodeKind, PropKey, PropValue};

use crate::animator::{Curve, Easing, Owner, Timer};
use crate::backend::NodeId;
use crate::chrome::Measure;
use crate::context::Ctx;
use crate::geometry::{Affine, Rect};
use crate::props::PropMap;
use crate::scene::{Change, IdMap, LayerOverride, LayerSlot};
use crate::text::{Ellipsize, TextAlign};

/// The wire value of `TextEllipsizeMode::Marquee`.
pub const MARQUEE: i64 = 5;
/// How long the line rests at each end.
const REST_MS: u64 = 1200;
/// Logical units per second.
const SPEED: f32 = 30.0;
/// How often a line that cannot be seen checks whether it can again.
const RECHECK_MS: u64 = 1000;

/// What makes the copy look like the text it stands for.
const TEXT_KEYS: [PropKey; 17] = [
    PropKey::Text,
    PropKey::TextSpans,
    PropKey::TextSpanBoxes,
    PropKey::FontFamily,
    PropKey::FontFeatureSettings,
    PropKey::FontSize,
    PropKey::FontStyle,
    PropKey::FontWeight,
    PropKey::LetterSpacing,
    PropKey::LineHeight,
    PropKey::TextTransform,
    PropKey::TextColor,
    PropKey::TextDecoration,
    PropKey::TextShadowColor,
    PropKey::TextShadowOffsetX,
    PropKey::TextShadowOffsetY,
    PropKey::TextShadowRadius,
];

#[derive(Default)]
struct Line {
    /// The clipping box and the sliding copy in it; `None` while the text fits.
    copy: Option<(NodeId, NodeId)>,
    /// How far the copy travels.
    distance: f32,
    /// Where the copy is, 0 (the start shows) to `distance` (the end shows).
    offset: f32,
    /// Which way it goes next.
    forward: bool,
    dirty: bool,
}

#[derive(Default)]
pub struct Marquees {
    lines: IdMap<Line>,
    dirty: bool,
}

fn place(ctx: &mut Ctx<'_>, copy: NodeId, offset: f32) {
    ctx.scene.set_layer_override(
        copy,
        LayerSlot::Presenter,
        LayerOverride {
            transform: Affine::translation(-offset, 0.0),
            ..LayerOverride::NONE
        },
    );
}

impl Marquees {
    pub fn on_change(&mut self, ctx: &mut Ctx<'_>, change: Change) {
        match change {
            Change::Created(id, NodeKind::Text) | Change::Updated(id, _) => {
                let wanted = ctx.scene.node(id).is_some_and(|node| {
                    node.kind == NodeKind::Text
                        && !node.is_synthetic()
                        && node.props().integer(PropKey::TextEllipsizeMode) == Some(MARQUEE)
                });
                if wanted {
                    self.lines.entry(id).or_default().dirty = true;
                    self.dirty = true;
                } else if self.lines.contains_key(&id) {
                    self.still(ctx, id);
                    self.lines.remove(&id);
                }
            }
            Change::Frame(id) => {
                if let Some(line) = self.lines.get_mut(&id) {
                    line.dirty = true;
                    self.dirty = true;
                }
            }
            Change::Removed(id, _) => {
                let tracked = self.lines.remove(&id).is_some();
                if tracked {
                    ctx.timers.cancel(Timer::Marquee(id));
                }
            }
            _ => {}
        }
    }

    /// The text paints itself again and nothing moves.
    fn still(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        ctx.timers.cancel(Timer::Marquee(id));
        ctx.animator.cancel(Owner::Marquee(id), ctx.now_ms);
        let Some(line) = self.lines.get_mut(&id) else {
            return;
        };
        if let Some((clip, _)) = line.copy.take() {
            ctx.scene.remove_synthetic(clip);
            ctx.scene.set_text_painted(id, true);
        }
    }

    /// Measures the lines that changed and starts or stops their copies.
    pub fn evaluate(&mut self, ctx: &mut Ctx<'_>, measure: Measure<'_>) {
        if !std::mem::take(&mut self.dirty) {
            return;
        }
        let ids: Vec<NodeId> = self
            .lines
            .iter()
            .filter(|(_, line)| line.dirty)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            if let Some(line) = self.lines.get_mut(&id) {
                line.dirty = false;
            }
            // Measured again from scratch: the text, its style or its box changed.
            self.still(ctx, id);
            if ctx.reduced_motion {
                continue;
            }
            let Some(node) = ctx.scene.node(id) else {
                continue;
            };
            let Some(mut block) = ctx.scene.text_block(id) else {
                continue;
            };
            // A marquee is one line; a paragraph that wraps has nothing to slide.
            if block.max_lines != 1 {
                continue;
            }
            let props = node.props();
            let padding = props.padding();
            let border = node.decoration.border.widths;
            let frame = node.frame;
            let (left, top) = (padding.left + border[0], padding.top + border[1]);
            let available = frame.width - left - padding.right - border[2];
            let height = frame.height - top - padding.bottom - border[3];
            if available <= 0.0 || height <= 0.0 {
                continue;
            }
            block.ellipsize = Ellipsize::Clip;
            block.align = TextAlign::Start;
            let natural = measure(&block, f32::INFINITY).width.ceil();
            if natural <= available + 0.5 {
                continue;
            }

            let mut copied = PropMap::new();
            for key in TEXT_KEYS {
                if let Some(value) = props.raw(key) {
                    copied.insert(key, value.clone());
                }
            }
            copied.insert(PropKey::NumberOfLines, PropValue::Integer(1));
            copied.insert(PropKey::TextEllipsizeMode, PropValue::Integer(4));
            copied.insert(PropKey::TextAlign, PropValue::Integer(1));
            // The text node itself is what is pressed and read out.
            copied.insert(PropKey::PointerEvents, PropValue::Integer(2));
            copied.insert(PropKey::AccessibilityImportance, PropValue::Integer(4));
            // The line slides inside the padding, not under it.
            let content = Rect::new(frame.x + left, frame.y + top, available, height);
            let mut clipping = PropMap::new();
            clipping.insert(PropKey::Overflow, PropValue::Integer(2));
            clipping.insert(PropKey::PointerEvents, PropValue::Integer(2));
            let clip = ctx
                .scene
                .add_synthetic(id, NodeKind::View, clipping, content, 0, None);
            let copy = ctx.scene.add_synthetic(
                clip,
                NodeKind::Text,
                copied,
                // A little wider than measured, so that rounding never wraps it.
                Rect::new(content.x, content.y, natural + 1.0, height),
                0,
                None,
            );
            ctx.scene.set_text_painted(id, false);
            place(ctx, copy, 0.0);
            if let Some(line) = self.lines.get_mut(&id) {
                *line = Line {
                    copy: Some((clip, copy)),
                    distance: natural - available,
                    offset: 0.0,
                    forward: true,
                    dirty: false,
                };
            }
            ctx.timers
                .schedule(Timer::Marquee(id), ctx.now_ms + REST_MS);
        }
    }

    /// The rest at one end is over: the line slides to the other.
    pub fn on_timer(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(line) = self.lines.get(&id) else {
            return;
        };
        if line.copy.is_none() {
            return;
        }
        let seen = ctx.scene.is_effectively_visible(id)
            && ctx.scene.visible_fraction(id, ctx.viewport) > 0.0;
        if !seen {
            // Nothing runs for a line nobody sees.
            ctx.timers
                .schedule(Timer::Marquee(id), ctx.now_ms + RECHECK_MS);
            return;
        }
        let target = if line.forward { line.distance } else { 0.0 };
        let duration = ((target - line.offset).abs() / SPEED * 1000.0).round() as u32;
        ctx.animator.start(
            Owner::Marquee(id),
            line.offset,
            target,
            Curve::timing(duration.max(1), Easing::Linear),
            0,
            ctx.now_ms,
        );
    }

    pub fn on_sample(&mut self, ctx: &mut Ctx<'_>, id: NodeId, value: f32, finished: bool) {
        let Some(line) = self.lines.get_mut(&id) else {
            return;
        };
        let Some((_, copy)) = line.copy else {
            return;
        };
        // The line moves a whole unit at a time: at this speed that is every
        // other frame, and the frames in between repaint nothing.
        let offset = if finished {
            value.clamp(0.0, line.distance)
        } else {
            value.clamp(0.0, line.distance).round()
        };
        let moved = offset != line.offset;
        line.offset = offset;
        if finished {
            line.forward = !line.forward;
            ctx.timers
                .schedule(Timer::Marquee(id), ctx.now_ms + REST_MS);
        }
        if moved {
            place(ctx, copy, offset);
        }
    }
}
