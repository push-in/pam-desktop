//! `RouteCovered` / `RouteFocused`: nodes that ask to know when the route or
//! tab scene they live in is hidden or shown again.

use pam_desktop_wire::PropKey;

use crate::backend::NodeId;
use crate::context::{event_kind, Ctx};
use crate::scene::{Change, IdMap};

#[derive(Default)]
pub struct Coverage {
    /// Listener node -> whether it was last reported as covered.
    listeners: IdMap<bool>,
    dirty: bool,
}

impl Coverage {
    pub fn on_change(&mut self, ctx: &mut Ctx<'_>, change: Change) {
        match change {
            Change::Created(id, _) | Change::Updated(id, PropKey::OnRouteCovered | PropKey::OnRouteFocused) => {
                let listens = ctx
                    .scene
                    .node(id)
                    .is_some_and(|node| node.handles(PropKey::OnRouteCovered) || node.handles(PropKey::OnRouteFocused));
                if listens {
                    // Listeners start "focused": one mounted in a hidden route hears about it once.
                    self.listeners.entry(id).or_insert(false);
                    self.dirty = true;
                } else {
                    self.listeners.remove(&id);
                }
            }
            Change::Removed(id, _) => {
                self.listeners.remove(&id);
            }
            _ => {}
        }
    }

    /// Call after anything that may have hidden or shown a route.
    pub fn invalidate(&mut self) {
        self.dirty = !self.listeners.is_empty();
    }

    /// Edge-triggered: reports only listeners whose state changed.
    pub fn evaluate(&mut self, ctx: &mut Ctx<'_>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let ids: Vec<NodeId> = self.listeners.keys().copied().collect();
        for id in ids {
            let covered = !ctx.scene.is_effectively_visible(id);
            let Some(reported) = self.listeners.get_mut(&id) else { continue };
            if *reported == covered {
                continue;
            }
            *reported = covered;
            if covered {
                ctx.emit_if(id, PropKey::OnRouteCovered, event_kind::ROUTE_COVERED, Vec::new());
            } else {
                ctx.emit_if(id, PropKey::OnRouteFocused, event_kind::ROUTE_FOCUSED, Vec::new());
            }
        }
    }
}
