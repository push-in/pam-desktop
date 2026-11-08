//! Element observers: `Intersect` (the node entered or left the screen),
//! `Resize` (its size changed) and `Mutate` (its frame in the parent
//! changed). Each is enabled by its handler marker and reports once when
//! installed.

use pam_desktop_wire::PropKey;

use crate::backend::NodeId;
use crate::context::{Ctx, event_kind};
use crate::geometry::{Rect, Size};
use crate::scene::{Change, IdMap};
use crate::wiremap::WireMap;

#[derive(Default)]
struct Watch {
    intersect: bool,
    resize: bool,
    mutate: bool,
    /// Last values reported; `None` before the first report.
    visible: Option<bool>,
    size: Option<Size>,
    frame: Option<Rect>,
}

#[derive(Default)]
pub struct Observers {
    watched: IdMap<Watch>,
    /// Frames or handlers changed since the last evaluation.
    dirty: bool,
}

impl Observers {
    pub fn on_change(&mut self, ctx: &mut Ctx<'_>, change: Change) {
        match change {
            Change::Created(id, _)
            | Change::Updated(id, PropKey::OnIntersect | PropKey::OnResize | PropKey::OnMutate) => {
                let Some(node) = ctx.scene.node(id) else {
                    return;
                };
                let (intersect, resize, mutate) = (
                    node.handles(PropKey::OnIntersect),
                    node.handles(PropKey::OnResize),
                    node.handles(PropKey::OnMutate),
                );
                if !(intersect || resize || mutate) {
                    self.watched.remove(&id);
                    return;
                }
                let watch = self.watched.entry(id).or_default();
                // A handler that just arrived reports the current state again.
                if intersect && !watch.intersect {
                    watch.visible = None;
                }
                if resize && !watch.resize {
                    watch.size = None;
                }
                if mutate && !watch.mutate {
                    watch.frame = None;
                }
                (watch.intersect, watch.resize, watch.mutate) = (intersect, resize, mutate);
                self.dirty = true;
            }
            Change::Removed(id, _) => {
                self.watched.remove(&id);
            }
            Change::Frame(_) => self.dirty = !self.watched.is_empty(),
            _ => {}
        }
    }

    /// Something that can hide or reveal nodes happened (scroll, route change).
    pub fn invalidate(&mut self) {
        self.dirty = !self.watched.is_empty();
    }

    /// Reports what changed since the last evaluation.
    pub fn evaluate(&mut self, ctx: &mut Ctx<'_>) {
        if !std::mem::take(&mut self.dirty) {
            return;
        }
        let ids: Vec<NodeId> = self.watched.keys().copied().collect();
        for id in ids {
            let Some(node) = ctx.scene.node(id) else {
                continue;
            };
            let frame = node.frame;
            let origin = ctx
                .scene
                .node(node.parent)
                .map_or(crate::geometry::Point::ZERO, |parent| parent.frame.origin());
            let relative = Rect::new(
                frame.x - origin.x,
                frame.y - origin.y,
                frame.width,
                frame.height,
            );
            let visible = ctx.scene.visible_fraction(id, ctx.viewport) > 0.0;
            let Some(watch) = self.watched.get_mut(&id) else {
                continue;
            };
            if watch.intersect && watch.visible != Some(visible) {
                watch.visible = Some(visible);
                ctx.emit(
                    id,
                    event_kind::INTERSECT,
                    if visible {
                        b"1".to_vec()
                    } else {
                        b"0".to_vec()
                    },
                );
            }
            if watch.resize && watch.size != Some(frame.size()) {
                // An empty node has no size worth a first report.
                let first = watch.size.is_none();
                watch.size = Some(frame.size());
                if !(first && frame.is_empty()) {
                    ctx.emit(
                        id,
                        event_kind::RESIZE,
                        WireMap::new()
                            .with("width", frame.width)
                            .with("height", frame.height)
                            .encode(),
                    );
                }
            }
            if watch.mutate && watch.frame != Some(relative) {
                let first = watch.frame.is_none();
                watch.frame = Some(relative);
                if !(first && frame.is_empty()) {
                    let payload = WireMap::new()
                        .with("x", relative.x)
                        .with("y", relative.y)
                        .with("width", relative.width)
                        .with("height", relative.height)
                        .encode();
                    ctx.emit(id, event_kind::MUTATE, payload);
                }
            }
        }
    }
}
