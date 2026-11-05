//! Shared element transitions: when a navigation moves between two routes
//! that each hold an element with the same `SharedTransitionTag`, the
//! element flies from one to the other above both routes.
//!
//! The flight is a copy drawn as host chrome, so it is not affected by the
//! transition of the routes themselves. Both tagged elements are hidden
//! while it flies.
//!
//! Between two images the copy is one picture, resized as it flies. For any
//! other pair both elements are copied with what they contain: the copies
//! are scaled to the frame in flight, and the one that leaves fades out over
//! the one that arrives.

use pam_desktop_wire::{NodeKind, PropKey, PropValue};

use crate::animator::{Curve, Easing, Owner, Spring};
use crate::backend::{NodeId, ViewClass};
use crate::context::Ctx;
use crate::geometry::{Affine, Rect};
use crate::props::PropMap;
use crate::scene::{LayerOverride, LayerSlot, Scene};

const MAX_FLIGHTS: usize = 16;
/// The largest element that is copied to fly; a larger one appears with
/// its route.
const MAX_COPIED_NODES: usize = 96;

/// What flies.
enum Copy {
    /// One picture, resized from the frame it leaves to the one it lands on.
    Picture {
        picture: NodeId,
        from_radius: f32,
        to_radius: f32,
    },
    /// Copies of both elements, laid out as they are at rest.
    Elements {
        arriving: NodeId,
        /// `None` without a cross-fade: only the arriving element flies.
        leaving: Option<NodeId>,
        /// Whether the copies stretch to the frame in flight or only move.
        scale: bool,
    },
}

struct Flight {
    copy: Copy,
    source: NodeId,
    destination: NodeId,
    from: Rect,
    to: Rect,
}

impl Flight {
    /// The copies, from the lowest to the topmost.
    fn roots(&self) -> impl Iterator<Item = NodeId> {
        let (first, second) = match self.copy {
            Copy::Picture { picture, .. } => (Some(picture), None),
            Copy::Elements {
                arriving, leaving, ..
            } => (leaving, Some(arriving)),
        };
        first.into_iter().chain(second)
    }

    /// Puts the copies where the flight is at `progress` (0 to 1).
    fn place(&self, scene: &mut Scene, progress: f32) {
        let mix = |from: f32, to: f32| from + (to - from) * progress;
        let frame = Rect::new(
            mix(self.from.x, self.to.x),
            mix(self.from.y, self.to.y),
            mix(self.from.width, self.to.width),
            mix(self.from.height, self.to.height),
        );
        match self.copy {
            Copy::Picture {
                picture,
                from_radius,
                to_radius,
            } => {
                scene.set_synthetic_frame(picture, frame);
                if from_radius != to_radius {
                    scene.set_prop(
                        picture,
                        PropKey::BorderRadius,
                        Some(PropValue::Float(f64::from(mix(from_radius, to_radius)))),
                    );
                }
            }
            Copy::Elements {
                arriving,
                leaving,
                scale,
            } => {
                // A copy keeps the layout it has at `rest`; the transform, in
                // its own coordinates, takes it to the frame in flight.
                let transform = |rest: Rect| {
                    let stretch = if scale {
                        Affine::scaling(frame.width / rest.width, frame.height / rest.height)
                    } else {
                        Affine::IDENTITY
                    };
                    stretch.then(&Affine::translation(frame.x - rest.x, frame.y - rest.y))
                };
                // The arriving copy is opaque by the middle of the flight and
                // the leaving one only fades after that: faded against each
                // other, the two would let the routes show through.
                scene.set_layer_override(
                    arriving,
                    LayerSlot::Presenter,
                    LayerOverride {
                        opacity: if leaving.is_some() {
                            (progress * 2.0).min(1.0)
                        } else {
                            1.0
                        },
                        transform: transform(self.to),
                        hidden: false,
                    },
                );
                if let Some(leaving) = leaving {
                    scene.set_layer_override(
                        leaving,
                        LayerSlot::Presenter,
                        LayerOverride {
                            opacity: (2.0 - progress * 2.0).min(1.0),
                            transform: transform(self.from),
                            hidden: false,
                        },
                    );
                }
            }
        }
    }
}

/// The number of nodes under `id`, itself included, counted up to `limit`.
fn subtree_size(scene: &Scene, id: NodeId, limit: usize) -> usize {
    let mut count = 0;
    let mut stack = vec![id];
    while let Some(current) = stack.pop() {
        count += 1;
        if count > limit {
            break;
        }
        if let Some(node) = scene.node(current) {
            stack.extend(&node.children);
        }
    }
    count
}

/// Copies `id` and what it contains as host chrome under `parent`, moved by
/// `(dx, dy)`. Text fields, players, web pages and scrolling content hold
/// state of their own that a copy would not have: they fly as their box.
fn copy_subtree(
    scene: &mut Scene,
    id: NodeId,
    parent: NodeId,
    order: u32,
    (dx, dy): (f32, f32),
) -> Option<NodeId> {
    let node = scene.node(id)?;
    let plain = matches!(node.class, ViewClass::Box | ViewClass::Surface);
    let kind = if plain { node.kind } else { NodeKind::View };
    let frame = node.frame.translate(dx, dy);
    let mut props = node.props.clone();
    props.remove(&PropKey::SharedTransitionTag);
    props.remove(&PropKey::SharedTransitionConfig);
    if parent == 0 {
        // Nothing to press and nothing to read out: the elements are there.
        props.insert(PropKey::PointerEvents, PropValue::Integer(2));
        props.insert(PropKey::AccessibilityImportance, PropValue::Integer(4));
    }
    let children = if plain {
        scene.ordered_children(id)
    } else {
        Vec::new()
    };
    let copy = scene.add_synthetic(parent, kind, props, frame, order, None);
    for (index, child) in children.into_iter().enumerate() {
        copy_subtree(scene, child, copy, index as u32, (dx, dy));
    }
    Some(copy)
}

/// `resizeMode` and `crossFade` of a `SharedTransitionConfig`: whether the
/// copies stretch (the default) and whether the leaving one is shown.
fn element_style(config: &str) -> (bool, bool) {
    let value =
        serde_json::from_str::<serde_json::Value>(config).unwrap_or(serde_json::Value::Null);
    let scale = value
        .get("resizeMode")
        .and_then(serde_json::Value::as_i64)
        .is_none_or(|mode| mode == 1);
    let cross_fade = value
        .get("crossFade")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    (scale, cross_fade)
}

#[derive(Default)]
pub struct SharedElements {
    flights: Vec<Flight>,
    roots: Vec<NodeId>,
}

/// Every tagged element under `route`: tag and node.
fn tagged(scene: &Scene, route: NodeId) -> Vec<(String, NodeId)> {
    let mut found = Vec::new();
    let mut stack = vec![route];
    while let Some(current) = stack.pop() {
        let Some(node) = scene.node(current) else {
            continue;
        };
        if let Some(tag) = node
            .props()
            .text(PropKey::SharedTransitionTag)
            .filter(|tag| !tag.is_empty())
        {
            found.push((tag.to_owned(), current));
        }
        stack.extend(node.children.iter().rev());
    }
    found
}

/// The frame of a node as it sits in the window, ignoring presentation
/// transforms (the incoming route is still off screen).
fn window_frame(scene: &Scene, id: NodeId) -> Option<Rect> {
    let node = scene.node(id)?;
    let origin = scene.window_origin(id);
    Some(Rect::new(
        origin.x,
        origin.y,
        node.frame.width,
        node.frame.height,
    ))
}

/// The duration and curve of a flight from its `SharedTransitionConfig`.
fn timing(config: &str, pop: bool, fallback_ms: u32) -> (u32, Curve) {
    let value =
        serde_json::from_str::<serde_json::Value>(config).unwrap_or(serde_json::Value::Null);
    let number = |key: &str| value.get(key).and_then(serde_json::Value::as_f64);
    let mut duration = number("durationMs").map_or(
        if config.is_empty() { fallback_ms } else { 500 },
        |duration| duration.clamp(0.0, 2000.0) as u32,
    );
    let mut easing = number("easing").map_or(2, |easing| easing as i64);
    if pop {
        // The way back may have a curve of its own.
        if let Some(back) = number("returnDurationMs") {
            duration = back.clamp(0.0, 2000.0) as u32;
        }
        if let Some(back) = number("returnEasing") {
            easing = back as i64;
        }
    }
    let curve = match easing {
        1 => Curve::timing(duration, Easing::Linear),
        3 => {
            let (stiffness, mass) = (
                number("stiffness").unwrap_or(220.0).clamp(1.0, 1000.0) as f32,
                number("mass").unwrap_or(1.0).clamp(0.1, 10.0) as f32,
            );
            let ratio = number("damping").unwrap_or(0.82).clamp(0.01, 1.0) as f32;
            Curve::spring(
                Spring {
                    stiffness,
                    damping: ratio * 2.0 * (stiffness * mass).sqrt(),
                    mass,
                    velocity: 0.0,
                },
                0.0,
                1.0,
            )
        }
        4 => Curve::timing(duration, Easing::InCubic),
        _ => Curve::timing(duration, Easing::EASE_IN_OUT),
    };
    (duration, curve)
}

impl SharedElements {
    #[must_use]
    pub fn roots(&self) -> &[NodeId] {
        &self.roots
    }

    /// Starts the flights of a navigation from `outgoing` to `incoming`.
    /// Returns whether the host overlays changed.
    pub fn start(
        &mut self,
        ctx: &mut Ctx<'_>,
        outgoing: NodeId,
        incoming: NodeId,
        pop: bool,
        navigation_ms: u32,
    ) -> bool {
        let had_flights = self.finish(ctx);
        if ctx.reduced_motion || navigation_ms == 0 {
            return had_flights;
        }
        let sources = tagged(ctx.scene, outgoing);
        if sources.is_empty() {
            return had_flights;
        }
        let destinations = tagged(ctx.scene, incoming);
        let mut longest: Option<(u32, Curve)> = None;
        for (tag, source) in sources {
            if self.flights.len() == MAX_FLIGHTS {
                break;
            }
            let Some((_, destination)) = destinations.iter().find(|(other, _)| *other == tag)
            else {
                continue;
            };
            let destination = *destination;
            // Only what the user can see leaves from where it is.
            if ctx.scene.visible_fraction(source, ctx.viewport) < 0.5 {
                continue;
            }
            let (Some(from), Some(to)) = (
                window_frame(ctx.scene, source),
                window_frame(ctx.scene, destination),
            ) else {
                continue;
            };
            if from.is_empty() || to.is_empty() {
                continue;
            }
            let (Some(source_node), Some(destination_node)) =
                (ctx.scene.node(source), ctx.scene.node(destination))
            else {
                continue;
            };
            // The route on top decides how the flight moves.
            let top = if pop { source_node } else { destination_node };
            let other = if pop { destination_node } else { source_node };
            let config = top
                .props()
                .text(PropKey::SharedTransitionConfig)
                .or_else(|| other.props().text(PropKey::SharedTransitionConfig))
                .unwrap_or_default();
            let flight_timing = timing(config, pop, navigation_ms);
            if longest
                .as_ref()
                .is_none_or(|(duration, _)| flight_timing.0 > *duration)
            {
                longest = Some(flight_timing);
            }
            let picture =
                |kind: NodeKind| matches!(kind, NodeKind::Image | NodeKind::ImageBackground);
            let copy = if picture(source_node.kind) && picture(destination_node.kind) {
                let (from_radius, to_radius) = (
                    source_node.props().number_or(PropKey::BorderRadius, 0.0),
                    destination_node
                        .props()
                        .number_or(PropKey::BorderRadius, 0.0),
                );
                // The picture that will be on screen at the end is the one that flies.
                let mut props = PropMap::new();
                for key in [
                    PropKey::Source,
                    PropKey::ImageSourceSet,
                    PropKey::ImageFit,
                    PropKey::TintColor,
                    PropKey::ImageRequestHeaders,
                    PropKey::ImageCachePolicy,
                ] {
                    if let Some(value) = destination_node
                        .props()
                        .raw(key)
                        .or_else(|| source_node.props().raw(key))
                    {
                        props.insert(key, value.clone());
                    }
                }
                props.insert(PropKey::ImageFadeDurationMs, PropValue::Integer(0));
                props.insert(
                    PropKey::BorderRadius,
                    PropValue::Float(f64::from(from_radius)),
                );
                props.insert(PropKey::Overflow, PropValue::Integer(2));
                props.insert(PropKey::PointerEvents, PropValue::Integer(2));
                Copy::Picture {
                    picture: ctx.scene.add_synthetic(
                        0,
                        NodeKind::Image,
                        props,
                        from,
                        self.flights.len() as u32 * 2,
                        None,
                    ),
                    from_radius,
                    to_radius,
                }
            } else {
                if subtree_size(ctx.scene, source, MAX_COPIED_NODES) > MAX_COPIED_NODES
                    || subtree_size(ctx.scene, destination, MAX_COPIED_NODES) > MAX_COPIED_NODES
                {
                    continue;
                }
                let (scale, cross_fade) = element_style(config);
                // Frames are laid out before scrolling: the copies start
                // from where the elements show.
                let offset = |node: NodeId, shown: Rect, scene: &Scene| {
                    let laid_out = scene.node(node).map_or(shown, |node| node.frame);
                    (shown.x - laid_out.x, shown.y - laid_out.y)
                };
                let order = self.flights.len() as u32 * 2;
                let leaving = if cross_fade {
                    let moved = offset(source, from, ctx.scene);
                    copy_subtree(ctx.scene, source, 0, order, moved)
                } else {
                    None
                };
                let moved = offset(destination, to, ctx.scene);
                let Some(arriving) = copy_subtree(ctx.scene, destination, 0, order + 1, moved)
                else {
                    if let Some(leaving) = leaving {
                        ctx.scene.remove_synthetic(leaving);
                    }
                    continue;
                };
                Copy::Elements {
                    arriving,
                    leaving,
                    scale,
                }
            };
            for node in [source, destination] {
                ctx.scene
                    .set_layer_override(node, LayerSlot::Interaction, LayerOverride::HIDDEN);
            }
            let flight = Flight {
                copy,
                source,
                destination,
                from,
                to,
            };
            flight.place(ctx.scene, 0.0);
            self.flights.push(flight);
        }
        let Some((duration, curve)) = longest else {
            return had_flights;
        };
        self.roots = self.flights.iter().flat_map(Flight::roots).collect();
        // Every flight of a transition shares the longest duration and its curve.
        if duration == 0 {
            return self.finish(ctx) || had_flights;
        }
        ctx.animator
            .start(Owner::SharedFlight, 0.0, 1.0, curve, 0, ctx.now_ms);
        true
    }

    /// A frame of the flights. Returns whether the host overlays changed.
    pub fn on_sample(&mut self, ctx: &mut Ctx<'_>, progress: f32, finished: bool) -> bool {
        if finished {
            return self.finish(ctx);
        }
        let progress = progress.clamp(0.0, 1.0);
        for flight in &self.flights {
            flight.place(ctx.scene, progress);
        }
        false
    }

    /// Lands every flight: the copies go away and the elements show again.
    pub fn finish(&mut self, ctx: &mut Ctx<'_>) -> bool {
        if self.flights.is_empty() {
            return false;
        }
        ctx.animator.cancel(Owner::SharedFlight, ctx.now_ms);
        for flight in self.flights.drain(..) {
            for copy in flight.roots() {
                ctx.scene.remove_synthetic(copy);
            }
            for node in [flight.source, flight.destination] {
                ctx.scene
                    .set_layer_override(node, LayerSlot::Interaction, LayerOverride::NONE);
            }
        }
        self.roots.clear();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flights_read_their_timing() {
        let (duration, curve) = timing(r#"{"durationMs":320,"easing":1}"#, false, 240);
        assert_eq!((duration, curve), (320, Curve::timing(320, Easing::Linear)));
        // The way back uses its own curve when given.
        let (duration, curve) = timing(
            r#"{"durationMs":320,"easing":1,"returnDurationMs":190,"returnEasing":4}"#,
            true,
            240,
        );
        assert_eq!(
            (duration, curve),
            (190, Curve::timing(190, Easing::InCubic))
        );
        // Without a configuration the flight lasts as long as the navigation.
        assert_eq!(timing("", false, 240).0, 240);
        assert!(matches!(
            timing(r#"{"easing":3}"#, false, 240).1,
            Curve::Spring { .. }
        ));
    }

    #[test]
    fn elements_read_their_resize_mode_and_cross_fade() {
        assert_eq!(element_style(""), (true, true));
        assert_eq!(
            element_style(r#"{"resizeMode":1,"crossFade":true}"#),
            (true, true)
        );
        assert_eq!(
            element_style(r#"{"resizeMode":3,"crossFade":false}"#),
            (false, false)
        );
        assert_eq!(element_style(r#"{"resizeMode":2}"#), (false, true));
    }

    #[test]
    fn copies_follow_the_frame_in_flight() {
        let mut scene = Scene::new();
        let add = |scene: &mut Scene, frame: Rect| {
            scene.add_synthetic(0, NodeKind::View, PropMap::new(), frame, 0, None)
        };
        let (from, to) = (
            Rect::new(10.0, 20.0, 100.0, 50.0),
            Rect::new(0.0, 100.0, 400.0, 200.0),
        );
        let (leaving, arriving) = (add(&mut scene, from), add(&mut scene, to));
        let flight = Flight {
            copy: Copy::Elements {
                arriving,
                leaving: Some(leaving),
                scale: true,
            },
            source: 0,
            destination: 0,
            from,
            to,
        };
        // The frame a copy shows at: its transform works in its own coordinates.
        let shown = |scene: &Scene, id: NodeId, rest: Rect| {
            let transform = scene.layer_override(id, LayerSlot::Presenter).transform;
            transform
                .bounds(Rect::new(0.0, 0.0, rest.width, rest.height))
                .translate(rest.x, rest.y)
        };

        flight.place(&mut scene, 0.0);
        assert_eq!(shown(&scene, arriving, to), from);
        assert_eq!(shown(&scene, leaving, from), from);
        assert_eq!(
            scene.layer_override(arriving, LayerSlot::Presenter).opacity,
            0.0
        );
        assert_eq!(
            scene.layer_override(leaving, LayerSlot::Presenter).opacity,
            1.0
        );

        flight.place(&mut scene, 0.5);
        let middle = Rect::new(5.0, 60.0, 250.0, 125.0);
        assert_eq!(shown(&scene, arriving, to), middle);
        assert_eq!(shown(&scene, leaving, from), middle);
        // Never see-through: one of the two is opaque at every point.
        assert_eq!(
            scene.layer_override(arriving, LayerSlot::Presenter).opacity,
            1.0
        );
        assert_eq!(
            scene.layer_override(leaving, LayerSlot::Presenter).opacity,
            1.0
        );

        flight.place(&mut scene, 1.0);
        assert_eq!(shown(&scene, arriving, to), to);
        assert_eq!(
            scene.layer_override(leaving, LayerSlot::Presenter).opacity,
            0.0
        );
        assert_eq!(flight.roots().collect::<Vec<_>>(), [leaving, arriving]);
    }
}
