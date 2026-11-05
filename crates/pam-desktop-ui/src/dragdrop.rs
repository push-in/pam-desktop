//! Dragging text between elements (and other applications): a node marked
//! draggable starts a system drag once the pointer moves past the drag
//! threshold; a node that accepts drops receives the text.

use pam_desktop_wire::PropKey;

use crate::backend::{NodeId, PointerButton, PointerEvent, PointerPhase};
use crate::context::{event_kind, Ctx, HostCommand};
use crate::geometry::Point;
use crate::scene::Scene;
use crate::wiremap::WireMap;

/// How far the pointer travels with the button down before a drag starts.
const DRAG_THRESHOLD: f32 = 4.0;

#[derive(Default)]
pub struct DragDrop {
    /// A press on a draggable node that may still become a drag.
    candidate: Option<(NodeId, Point)>,
}

/// The nearest node at or above `id` for which `accepts` holds.
fn nearest(scene: &Scene, id: NodeId, accepts: impl Fn(&crate::scene::SceneNode) -> bool) -> Option<NodeId> {
    let mut current = id;
    loop {
        let node = scene.node(current)?;
        if accepts(node) {
            return Some(current);
        }
        if node.parent == 0 {
            return None;
        }
        current = node.parent;
    }
}

impl DragDrop {
    /// Watches presses on draggable nodes. `true` means a drag just
    /// started: the press in progress must be cancelled.
    pub fn on_pointer(&mut self, ctx: &mut Ctx<'_>, pointer: PointerEvent) -> bool {
        match pointer.phase {
            PointerPhase::Down if pointer.button == PointerButton::Primary => {
                self.candidate = ctx
                    .scene
                    .hit_test(pointer.position)
                    .and_then(|hit| nearest(ctx.scene, hit, |node| node.props().flag_or(PropKey::Draggable, false) && node.props().flag_or(PropKey::Enabled, true)))
                    .map(|id| (id, pointer.position));
                false
            }
            PointerPhase::Move => {
                let Some((id, start)) = self.candidate else { return false };
                if (pointer.position.x - start.x).hypot(pointer.position.y - start.y) < DRAG_THRESHOLD {
                    return false;
                }
                self.candidate = None;
                let Some(text) = ctx.scene.node(id).map(|node| node.props().text_or_empty(PropKey::DragData).to_owned()) else { return false };
                ctx.emit_if(id, PropKey::OnDragStart, event_kind::DRAG_START, Vec::new());
                ctx.commands.push(HostCommand::BeginTextDrag { source: id, text });
                true
            }
            PointerPhase::Up | PointerPhase::Cancel => {
                self.candidate = None;
                false
            }
            _ => false,
        }
    }

    /// Text was dropped at a window position: the nearest node that
    /// accepts drops receives it.
    pub fn on_text_dropped(&mut self, ctx: &mut Ctx<'_>, position: Point, text: &str) {
        let target = ctx.scene.hit_test(position).and_then(|hit| nearest(ctx.scene, hit, |node| node.props().flag_or(PropKey::DropEnabled, false)));
        if let Some(target) = target {
            ctx.emit_if(target, PropKey::OnDrop, event_kind::DROP, WireMap::new().with("data", text).encode());
        }
    }

    /// The drag of `source` ended, dropped or not.
    pub fn on_drag_finished(&mut self, ctx: &mut Ctx<'_>, source: NodeId) {
        ctx.emit_if(source, PropKey::OnDragEnd, event_kind::DRAG_END, Vec::new());
    }
}
