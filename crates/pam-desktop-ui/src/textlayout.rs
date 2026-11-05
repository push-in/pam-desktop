//! Two text behaviours that need the platform's typesetter: `TextLayout`
//! (how a paragraph wrapped: line counts and truncation) and `SpanPress`
//! (a press on a pressable run inside a paragraph, such as a link).

use pam_desktop_wire::PropKey;

use crate::backend::NodeId;
use crate::chrome::Measure;
use crate::context::{Ctx, event_kind};
use crate::scene::{Change, IdMap};
use crate::text::utf16_len;
use crate::wiremap::WireMap;

#[derive(Default)]
pub struct TextLayouts {
    /// Listener -> the layout last reported (lines, visible lines, width, height).
    listeners: IdMap<Option<(u32, u32, f32, f32)>>,
    dirty: bool,
    /// The pressable run under the pointer when the button went down.
    pressed: Option<(NodeId, u32)>,
}

impl TextLayouts {
    pub fn on_change(&mut self, ctx: &mut Ctx<'_>, change: Change) {
        match change {
            Change::Created(id, _) | Change::Updated(id, PropKey::OnTextLayout) => {
                if ctx
                    .scene
                    .node(id)
                    .is_some_and(|node| node.handles(PropKey::OnTextLayout))
                {
                    self.listeners.entry(id).or_insert(None);
                    self.dirty = true;
                } else {
                    self.listeners.remove(&id);
                }
            }
            Change::Removed(id, _) => {
                self.listeners.remove(&id);
            }
            Change::Updated(id, _) | Change::Frame(id) if self.listeners.contains_key(&id) => {
                self.dirty = true;
            }
            _ => {}
        }
    }

    /// Reports the paragraphs whose wrapping changed.
    pub fn evaluate(&mut self, ctx: &mut Ctx<'_>, measure: Measure<'_>) {
        if !std::mem::take(&mut self.dirty) {
            return;
        }
        let ids: Vec<NodeId> = self.listeners.keys().copied().collect();
        for id in ids {
            let Some(node) = ctx.scene.node(id) else {
                continue;
            };
            let padding = node.props().padding();
            let width = (node.frame.width - padding.horizontal()).max(0.0);
            let Some(block) = ctx.scene.text_block(id) else {
                continue;
            };
            if width <= 0.0 {
                continue;
            }
            let visible = measure(&block, width);
            // The same paragraph without the line limit tells how much was cut.
            let total = if block.max_lines > 0 {
                let mut unlimited = block.clone();
                unlimited.max_lines = 0;
                measure(&unlimited, width).line_count
            } else {
                visible.line_count
            };
            let layout = (total, visible.line_count, visible.width, visible.height);
            let Some(reported) = self.listeners.get_mut(&id) else {
                continue;
            };
            if *reported == Some(layout) {
                continue;
            }
            *reported = Some(layout);
            let payload = WireMap::new()
                .with("lines", i64::from(total))
                .with("visibleLines", i64::from(visible.line_count))
                .with("truncated", total > visible.line_count)
                .with("width", visible.width)
                .with("height", visible.height)
                .encode();
            ctx.emit(id, event_kind::TEXT_LAYOUT, payload);
        }
    }

    /// The press slot of the run that holds the UTF-16 offset `index`.
    fn slot_at(ctx: &Ctx<'_>, id: NodeId, index: u32) -> Option<u32> {
        let block = ctx.scene.text_block(id)?;
        let mut start = 0_u32;
        for run in &block.runs {
            let end = start + utf16_len(&run.text);
            if index >= start && index < end {
                return run.press;
            }
            start = end;
        }
        None
    }

    /// A text node that reports span presses, when `id` is one.
    #[must_use]
    pub fn listens(ctx: &Ctx<'_>, id: NodeId) -> bool {
        ctx.scene
            .node(id)
            .is_some_and(|node| node.is_text_bearing() && node.handles(PropKey::OnSpanPress))
    }

    /// The button went down on a paragraph at `index`. Returns whether a
    /// pressable run took the press.
    pub fn press_down(&mut self, ctx: &Ctx<'_>, id: NodeId, index: u32) -> bool {
        self.pressed = Self::slot_at(ctx, id, index).map(|slot| (id, slot));
        self.pressed.is_some()
    }

    /// The button went up. A release on the run that was pressed reports it.
    pub fn press_up(&mut self, ctx: &mut Ctx<'_>, released: Option<(NodeId, u32)>) -> bool {
        let Some((id, slot)) = self.pressed.take() else {
            return false;
        };
        if released.is_some_and(|(node, index)| {
            node == id && Self::slot_at(ctx, node, index) == Some(slot)
        }) {
            ctx.emit_if(
                id,
                PropKey::OnSpanPress,
                event_kind::SPAN_PRESS,
                slot.to_string().into_bytes(),
            );
        }
        true
    }

    #[must_use]
    pub fn is_pressing(&self) -> bool {
        self.pressed.is_some()
    }
}
