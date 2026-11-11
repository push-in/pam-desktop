//! Dragging text between elements (and other applications): a node marked
//! draggable starts a system drag once the pointer moves past the drag
//! threshold; a node that accepts drops receives the text.

use pam_desktop_wire::PropKey;

use crate::backend::{NodeId, PointerButton, PointerEvent, PointerPhase, WindowId};
use crate::context::{Ctx, HostCommand, event_kind};
use crate::geometry::Point;
use crate::scene::Scene;
use crate::wiremap::WireMap;

/// How far the pointer travels with the button down before a drag starts.
const DRAG_THRESHOLD: f32 = 4.0;

#[derive(Default)]
pub struct DragDrop {
    /// A press on a draggable node that may still become a drag.
    candidate: Option<(NodeId, Point)>,
    active: Option<ActiveDrag>,
    hovered: Option<NodeId>,
    position: Option<Point>,
}

#[derive(Clone)]
struct ActiveDrag {
    source: NodeId,
    window: WindowId,
    kind: i64,
    data: String,
    nonce: String,
}
impl ActiveDrag {
    fn payload(&self) -> Vec<u8> {
        WireMap::new()
            .with("type", self.kind)
            .with("data", self.data.as_str())
            .encode()
    }
}

/// The nearest node at or above `id` for which `accepts` holds.
fn nearest(
    scene: &Scene,
    id: NodeId,
    accepts: impl Fn(&crate::scene::SceneNode) -> bool,
) -> Option<NodeId> {
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
                    .and_then(|hit| {
                        let mut current = hit;
                        loop {
                            let node = ctx.scene.node(current)?;
                            if node.props().flag_or(PropKey::Draggable, false)
                                && node.props().flag_or(PropKey::Enabled, true)
                            {
                                break Some(current);
                            }
                            // A pressable inside a drag source (a clickable
                            // card) drags it once the pointer travels; a click
                            // stays a press. Text fields keep their selection.
                            if matches!(
                                node.class,
                                crate::backend::ViewClass::Input
                                    | crate::backend::ViewClass::TextArea
                            ) || node.parent == 0
                            {
                                break None;
                            }
                            current = node.parent;
                        }
                    })
                    .map(|id| (id, pointer.position));
                false
            }
            PointerPhase::Move => {
                let Some((id, start)) = self.candidate else {
                    return false;
                };
                if (pointer.position.x - start.x).hypot(pointer.position.y - start.y)
                    < DRAG_THRESHOLD
                {
                    return false;
                }
                self.candidate = None;
                let Some(text) = ctx
                    .scene
                    .node(id)
                    .map(|node| node.props().text_or_empty(PropKey::DragData).to_owned())
                else {
                    return false;
                };
                if text.len() > 65536 {
                    return false;
                }
                let kind = ctx
                    .scene
                    .node(id)
                    .map_or(0, |node| node.props().integer_or(PropKey::DragType, 0));
                let text = if kind > 0 {
                    if kind > 65535 || text.len() > 512 {
                        return false;
                    }
                    let active = ActiveDrag {
                        source: id,
                        window: pointer.window,
                        kind,
                        data: text,
                        nonce: crate::modules::support::uuid(),
                    };
                    ctx.emit_if(
                        id,
                        PropKey::OnDragStart,
                        event_kind::DRAG_START,
                        active.payload(),
                    );
                    let token = active.nonce.clone();
                    self.active = Some(active);
                    token
                } else {
                    self.active = None;
                    ctx.emit_if(id, PropKey::OnDragStart, event_kind::DRAG_START, Vec::new());
                    text
                };
                ctx.commands.push(HostCommand::BeginTextDrag {
                    window: pointer.window,
                    source: id,
                    text,
                });
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
    pub fn on_text_dropped(
        &mut self,
        ctx: &mut Ctx<'_>,
        window: WindowId,
        position: Point,
        text: &str,
    ) {
        if text.len() > 65536 {
            return;
        }
        let target = ctx.scene.hit_test(position).and_then(|hit| {
            nearest(ctx.scene, hit, |node| {
                node.props().flag_or(PropKey::DropEnabled, false)
                    && node.props().flag_or(PropKey::Enabled, true)
            })
        });
        if let Some(target) = target {
            let kind = ctx
                .scene
                .node(target)
                .map_or(0, |node| node.props().integer_or(PropKey::DropType, 0));
            if kind > 0 {
                if let Some(active) = self.active.as_ref().filter(|active| {
                    active.kind == kind
                        && active.window == window
                        && active.nonce == text
                        && ctx.scene.is_effectively_visible(active.source)
                }) {
                    ctx.emit_if(target, PropKey::OnDrop, event_kind::DROP, active.payload());
                }
                return;
            }
            // An opaque internal drag must never leak its token to an untyped target.
            if self.active.is_some() {
                return;
            }
            ctx.emit_if(
                target,
                PropKey::OnDrop,
                event_kind::DROP,
                WireMap::new().with("data", text).encode(),
            );
        }
    }

    /// The drag of `source` ended, dropped or not.
    pub fn on_motion(&mut self, ctx: &mut Ctx<'_>, window: WindowId, position: Option<Point>) {
        self.position = position;
        ctx.timers.cancel(crate::animator::Timer::DragScroll);
        let Some(active) = self.active.clone() else {
            return;
        };
        if position.is_some() && active.window == window {
            ctx.timers
                .schedule(crate::animator::Timer::DragScroll, ctx.now_ms + 40);
        }
        let target = if active.window == window && ctx.scene.is_effectively_visible(active.source) {
            position
                .and_then(|point| ctx.scene.hit_test(point))
                .and_then(|hit| {
                    nearest(ctx.scene, hit, |node| {
                        node.props().flag_or(PropKey::DropEnabled, false)
                            && node.props().flag_or(PropKey::Enabled, true)
                            && node.props().integer_or(PropKey::DropType, 0) == active.kind
                    })
                })
        } else {
            None
        };
        if target == self.hovered {
            return;
        }
        if let Some(previous) = self.hovered.take() {
            ctx.emit_if(
                previous,
                PropKey::OnDragLeave,
                event_kind::DRAG_LEAVE,
                active.payload(),
            );
        }
        self.hovered = target;
        if let Some(target) = target {
            ctx.emit_if(
                target,
                PropKey::OnDragEnter,
                event_kind::DRAG_ENTER,
                active.payload(),
            );
        }
    }
    pub fn cancel(&mut self, ctx: &mut Ctx<'_>) {
        self.candidate = None;
        if let Some(source) = self.active.as_ref().map(|active| active.source) {
            self.on_drag_finished(ctx, source);
        } else {
            ctx.timers.cancel(crate::animator::Timer::DragScroll);
        }
    }
    pub fn scrolled(&mut self, ctx: &mut Ctx<'_>) {
        if let Some(window) = self.window() {
            self.on_motion(ctx, window, self.position);
        }
    }
    pub fn window(&self) -> Option<WindowId> {
        self.active.as_ref().map(|active| active.window)
    }
    pub fn scroll_tick(&mut self, ctx: &mut Ctx<'_>, scrolling: &mut crate::scrolling::Scrolling) {
        let (Some(active), Some(position)) = (self.active.as_ref(), self.position) else {
            return;
        };
        if !ctx.scene.is_effectively_visible(active.source) {
            return;
        }
        let mut current = ctx.scene.hit_test(position).unwrap_or(0);
        let mut delta = Point::ZERO;
        while let Some(node) = ctx.scene.node(current) {
            if node.class == crate::backend::ViewClass::Scroll
                && node.props().flag_or(PropKey::ScrollEnabled, true)
            {
                let origin = ctx.scene.window_origin(current);
                let horizontal = node.props().flag_or(PropKey::ScrollHorizontal, false)
                    || node.props().flag_or(PropKey::ListHorizontal, false);
                let (point, start, size) = if horizontal {
                    (position.x, origin.x, node.frame.width)
                } else {
                    (position.y, origin.y, node.frame.height)
                };
                let edge = 32.0_f32.min(size / 4.0);
                let amount = if point < start + edge {
                    -12.0
                } else if point > start + size - edge {
                    12.0
                } else {
                    0.0
                };
                if horizontal && delta.x == 0.0 {
                    delta.x = amount;
                } else if !horizontal && delta.y == 0.0 {
                    delta.y = amount;
                }
            }
            current = node.parent;
        }
        if (delta.x != 0.0 || delta.y != 0.0) && scrolling.on_wheel(ctx, position, delta) {
            ctx.timers
                .schedule(crate::animator::Timer::DragScroll, ctx.now_ms + 40);
        }
    }
    pub fn on_drag_finished(&mut self, ctx: &mut Ctx<'_>, source: NodeId) {
        self.position = None;
        ctx.timers.cancel(crate::animator::Timer::DragScroll);
        if let Some(active) = self.active.take()
            && let Some(target) = self.hovered.take()
        {
            ctx.emit_if(
                target,
                PropKey::OnDragLeave,
                event_kind::DRAG_LEAVE,
                active.payload(),
            );
        }
        ctx.emit_if(source, PropKey::OnDragEnd, event_kind::DRAG_END, Vec::new());
    }
}
