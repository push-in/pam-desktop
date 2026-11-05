//! `NavigationHost`: stack transitions between route screens.
//!
//! The children of a host are the mounted routes (only the top two entries of
//! the stack, plus kept-alive ones). The host shows one route, animates a
//! transition whenever `NavigationRevision` changes and must report
//! `AnimationComplete` afterwards: PHP only releases the outgoing route then.

use pam_desktop_wire::{NodeKind, PropKey};

use crate::animator::{Curve, Easing, Owner};
use crate::backend::NodeId;
use crate::context::{event_kind, Ctx};
use crate::geometry::{Affine, Point};
use crate::scene::{Change, IdMap, LayerOverride, LayerSlot};

const OPERATION_PUSH: i64 = 2;
const OPERATION_POP: i64 = 3;
const OPERATION_REPLACE: i64 = 4;

const TRANSITION_NONE: i64 = 8;

#[derive(Clone, Copy, Debug)]
struct Run {
    operation: i64,
    transition: i64,
    presentation: i64,
    incoming: NodeId,
    outgoing: Option<NodeId>,
}

#[derive(Debug, Default)]
struct Host {
    revision: i64,
    /// The route on screen when no transition runs.
    active: Option<NodeId>,
    /// The previous route, kept visible under a modal presentation.
    underlay: Option<NodeId>,
    run: Option<Run>,
    /// `NavigationRevision` changed in the commit being applied.
    navigate: bool,
    /// Children changed in the commit being applied.
    sync: bool,
}

#[derive(Default)]
pub struct Navigation {
    hosts: IdMap<Host>,
    dirty: bool,
}

/// Engine children of a host, bottom to top (chrome excluded).
fn routes(ctx: &Ctx<'_>, host: NodeId) -> Vec<NodeId> {
    ctx.scene
        .ordered_children(host)
        .into_iter()
        .filter(|child| ctx.scene.node(*child).is_some_and(|node| !node.is_synthetic()))
        .collect()
}

impl Navigation {
    pub fn on_change(&mut self, ctx: &mut Ctx<'_>, change: Change) {
        match change {
            Change::Created(id, NodeKind::NavigationHost) => {
                let revision = ctx.scene.node(id).map_or(0, |node| node.props().integer_or(PropKey::NavigationRevision, 0));
                self.hosts.insert(id, Host { revision, sync: true, ..Host::default() });
                self.dirty = true;
            }
            Change::Removed(id, NodeKind::NavigationHost) => {
                self.hosts.remove(&id);
                ctx.animator.cancel(Owner::Navigation(id), ctx.now_ms);
            }
            Change::Updated(id, PropKey::NavigationRevision) => {
                if let Some(host) = self.hosts.get_mut(&id) {
                    host.navigate = true;
                    self.dirty = true;
                }
            }
            Change::Children(id) => {
                if let Some(host) = self.hosts.get_mut(&id) {
                    host.sync = true;
                    self.dirty = true;
                }
            }
            _ => {}
        }
    }

    /// Runs once the whole commit is applied: transitions read the final
    /// child order, operation, transition and duration.
    pub fn after_commit(&mut self, ctx: &mut Ctx<'_>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let ids: Vec<NodeId> = self.hosts.iter().filter(|(_, host)| host.navigate || host.sync).map(|(id, _)| *id).collect();
        for id in ids {
            let (navigate, sync) = match self.hosts.get_mut(&id) {
                Some(host) => (std::mem::take(&mut host.navigate), std::mem::take(&mut host.sync)),
                None => continue,
            };
            if navigate {
                self.navigate(ctx, id);
            } else if sync {
                self.sync(ctx, id);
            }
        }
    }

    /// Children changed without a navigation: keep exactly one route visible.
    fn sync(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let routes = routes(ctx, id);
        let Some(host) = self.hosts.get_mut(&id) else { return };
        if let Some(run) = host.run {
            // A route of the running transition left: settle it now.
            let gone = !routes.contains(&run.incoming) || run.outgoing.is_some_and(|outgoing| !routes.contains(&outgoing));
            if !gone {
                // New siblings stay hidden until their own navigation runs.
                for route in &routes {
                    if *route != run.incoming && Some(*route) != run.outgoing {
                        ctx.scene.set_layer_override(*route, LayerSlot::Presenter, LayerOverride::HIDDEN);
                    }
                }
                return;
            }
            ctx.animator.cancel(Owner::Navigation(id), ctx.now_ms);
            host.run = None;
        }
        if !host.active.is_some_and(|active| routes.contains(&active)) {
            host.active = routes.last().copied();
        }
        if !host.underlay.is_some_and(|underlay| routes.contains(&underlay)) {
            host.underlay = None;
        }
        let (active, underlay) = (host.active, host.underlay);
        for route in routes {
            // A route under a modal presentation stays visible below the active one.
            let shown = Some(route) == active || Some(route) == underlay;
            ctx.scene.set_layer_override(
                route,
                LayerSlot::Presenter,
                if shown { LayerOverride::NONE } else { LayerOverride::HIDDEN },
            );
        }
    }

    fn navigate(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        // A newer navigation first settles the running one.
        if self.hosts.get(&id).is_some_and(|host| host.run.is_some()) {
            ctx.animator.cancel(Owner::Navigation(id), ctx.now_ms);
            self.finish(ctx, id);
        }
        let routes = routes(ctx, id);
        let Some(node) = ctx.scene.node(id) else { return };
        let props = node.props();
        let revision = props.integer_or(PropKey::NavigationRevision, 0);
        let operation = props.integer_or(PropKey::NavigationOperation, 1);
        let transition = match props.integer_or(PropKey::NavigationTransition, 1) {
            1 => 2,
            other => other,
        };
        let duration = props.integer_or(PropKey::NavigationDurationMs, 240).clamp(0, 2000) as u32;
        let presentation = props.integer_or(PropKey::NavigationPresentation, 1);
        let Some(host) = self.hosts.get_mut(&id) else { return };
        if host.revision == revision {
            return self.sync(ctx, id);
        }
        host.revision = revision;

        let previous = host.active.filter(|active| routes.contains(active));
        let (incoming, outgoing) = match operation {
            OPERATION_PUSH | OPERATION_REPLACE => (routes.last().copied(), previous),
            OPERATION_POP => {
                let outgoing = routes.last().copied();
                let incoming = if routes.len() >= 2 { routes.get(routes.len() - 2).copied() } else { None };
                (incoming, outgoing)
            }
            // Idle and Reset show the top route without a transition or an event.
            _ => {
                host.active = routes.last().copied();
                return self.sync(ctx, id);
            }
        };
        let Some(incoming) = incoming else {
            host.active = routes.last().copied();
            self.sync(ctx, id);
            ctx.emit_if(id, PropKey::OnAnimationComplete, event_kind::ANIMATION_COMPLETE, Vec::new());
            return;
        };
        let outgoing = outgoing.filter(|outgoing| *outgoing != incoming);
        host.run = Some(Run { operation, transition, presentation, incoming, outgoing });

        // Only the two routes of the transition are visible while it runs.
        for route in &routes {
            if *route != incoming && Some(*route) != outgoing {
                ctx.scene.set_layer_override(*route, LayerSlot::Presenter, LayerOverride::HIDDEN);
            }
        }
        let instant = transition == TRANSITION_NONE || duration == 0 || ctx.reduced_motion || outgoing.is_none();
        if instant {
            self.finish(ctx, id);
        } else {
            self.apply(ctx, id, 0.0);
            // The curve the Android host uses: 1 - (1 - t)^3.5.
            ctx.animator.start(Owner::Navigation(id), 0.0, 1.0, Curve::timing(duration, Easing::Decelerate(1.75)), 0, ctx.now_ms);
        }
    }

    /// Frame-clock sample of a host's transition.
    pub fn on_sample(&mut self, ctx: &mut Ctx<'_>, id: NodeId, progress: f32, finished: bool) {
        if finished {
            self.finish(ctx, id);
        } else {
            self.apply(ctx, id, progress);
        }
    }

    /// Places both routes for eased progress `p` (0 = start, 1 = end).
    fn apply(&self, ctx: &mut Ctx<'_>, id: NodeId, p: f32) {
        let Some(run) = self.hosts.get(&id).and_then(|host| host.run) else { return };
        let Some(frame) = ctx.scene.node(id).map(|node| node.frame) else { return };
        let (width, height) = (frame.width, frame.height);
        let pop = run.operation == OPERATION_POP;
        let rtl = ctx.scene.node(id).is_some_and(|node| node.props().integer(PropKey::LayoutDirection) == Some(2));
        let side = if rtl { -1.0 } else { 1.0 };
        let direction = if run.transition == 3 { -1.0 } else { side };
        let center = Point::new(width / 2.0, height / 2.0);

        let mut incoming = LayerOverride::NONE;
        let mut outgoing = LayerOverride::NONE;
        let shift = |x: f32, y: f32| Affine::translation(x, y);
        match run.transition {
            2 | 3 => {
                if pop {
                    incoming.transform = shift(-direction * width * 0.28 * (1.0 - p), 0.0);
                    outgoing.transform = shift(direction * width * p, 0.0);
                } else {
                    incoming.transform = shift(direction * width * (1.0 - p), 0.0);
                    outgoing.transform = shift(-direction * width * 0.28 * p, 0.0);
                }
                outgoing.opacity = 1.0 - 0.18 * p;
            }
            4 | 9 => {
                let sign = if run.transition == 9 { -1.0 } else { 1.0 };
                if pop {
                    outgoing.transform = shift(0.0, sign * height * p);
                } else {
                    incoming.transform = shift(0.0, sign * height * (1.0 - p));
                }
                outgoing.opacity = 1.0 - 0.12 * p;
            }
            5 => {
                // A transparent modal route only fades itself; the route below stays.
                if matches!(run.presentation, 5 | 6) {
                    if pop {
                        outgoing.opacity = 1.0 - p;
                    } else {
                        incoming.opacity = p;
                    }
                } else {
                    incoming.opacity = p;
                    outgoing.opacity = 1.0 - p;
                }
            }
            6 => {
                incoming.opacity = p;
                incoming.transform = shift(0.0, 0.08 * height * (1.0 - p));
            }
            7 => {
                incoming.opacity = p;
                let scale = 0.94 + 0.06 * p;
                incoming.transform = Affine::scaling(scale, scale).around(center);
            }
            10 => {
                let sign = if pop { -1.0 } else { 1.0 };
                incoming.opacity = p;
                incoming.transform = shift(sign * side * width * 0.12 * (1.0 - p), 0.0);
                outgoing.opacity = 1.0 - p;
                outgoing.transform = shift(-sign * side * width * 0.08 * p, 0.0);
            }
            11 => {
                let sign = if pop { -1.0 } else { 1.0 };
                incoming.opacity = p;
                incoming.transform = shift(0.0, sign * height * 0.08 * (1.0 - p));
                outgoing.opacity = 1.0 - p;
                outgoing.transform = shift(0.0, -sign * height * 0.05 * p);
            }
            12 => {
                // A flip around the vertical axis, seen head on: each half of
                // the turn shows one route squeezed horizontally.
                incoming.opacity = p;
                let turn_in = (std::f32::consts::FRAC_PI_2 * p).sin().max(0.001);
                let turn_out = (std::f32::consts::FRAC_PI_2 * p).cos().max(0.001);
                incoming.transform = Affine::scaling(turn_in, 1.0).around(center);
                outgoing.transform = Affine::scaling(turn_out, 1.0).around(center);
            }
            13 => {
                if pop {
                    outgoing.transform = shift(direction * width * p, 0.0);
                } else {
                    incoming.transform = shift(direction * width * (1.0 - p), 0.0);
                }
            }
            _ => {}
        }
        ctx.scene.set_layer_override(run.incoming, LayerSlot::Presenter, incoming);
        if let Some(route) = run.outgoing {
            ctx.scene.set_layer_override(route, LayerSlot::Presenter, outgoing);
        }
    }

    /// Settles the transition and tells PHP it is over.
    fn finish(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(host) = self.hosts.get_mut(&id) else { return };
        let Some(run) = host.run.take() else { return };
        host.active = Some(run.incoming);
        if host.underlay == Some(run.incoming) || run.outgoing.is_none() {
            host.underlay = None;
        }
        ctx.scene.set_layer_override(run.incoming, LayerSlot::Presenter, LayerOverride::NONE);
        if let Some(outgoing) = run.outgoing {
            // Modal presentations keep the previous route visible underneath.
            let stays = run.operation != OPERATION_POP && (3..=7).contains(&run.presentation);
            host.underlay = stays.then_some(outgoing);
            ctx.scene.set_layer_override(
                outgoing,
                LayerSlot::Presenter,
                if stays { LayerOverride::NONE } else { LayerOverride::HIDDEN },
            );
        }
        ctx.emit_if(id, PropKey::OnAnimationComplete, event_kind::ANIMATION_COMPLETE, Vec::new());
    }

    /// True while any route transition runs (tests and diagnostics).
    #[must_use]
    pub fn is_transitioning(&self, id: NodeId) -> bool {
        self.hosts.get(&id).is_some_and(|host| host.run.is_some())
    }

    #[must_use]
    pub fn active_route(&self, id: NodeId) -> Option<NodeId> {
        self.hosts.get(&id).and_then(|host| host.active)
    }
}
