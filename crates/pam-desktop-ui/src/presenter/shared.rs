//! Shared element transitions: when a navigation moves between two routes
//! that each hold an element with the same `SharedTransitionTag`, the
//! picture flies from one to the other above both routes.
//!
//! The flight is a copy of the image drawn as host chrome, so it is not
//! affected by the transition of the routes themselves. Both tagged
//! elements are hidden while it flies. Elements that are not images do not
//! fly: they appear with their route.

use pam_desktop_wire::{NodeKind, PropKey, PropValue};

use crate::animator::{Curve, Easing, Owner, Spring};
use crate::backend::NodeId;
use crate::context::Ctx;
use crate::geometry::Rect;
use crate::props::PropMap;
use crate::scene::{LayerOverride, LayerSlot, Scene};

const MAX_FLIGHTS: usize = 16;

struct Flight {
    /// The copy that flies.
    picture: NodeId,
    source: NodeId,
    destination: NodeId,
    from: Rect,
    to: Rect,
    from_radius: f32,
    to_radius: f32,
}

#[derive(Default)]
pub struct SharedElements {
    flights: Vec<Flight>,
    roots: Vec<NodeId>,
}

/// Every tagged image under `route`: tag and node.
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
            && matches!(node.kind, NodeKind::Image | NodeKind::ImageBackground)
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
            let picture = ctx.scene.add_synthetic(
                0,
                NodeKind::Image,
                props,
                from,
                self.flights.len() as u32,
                None,
            );
            for node in [source, destination] {
                ctx.scene
                    .set_layer_override(node, LayerSlot::Interaction, LayerOverride::HIDDEN);
            }
            self.flights.push(Flight {
                picture,
                source,
                destination,
                from,
                to,
                from_radius,
                to_radius,
            });
        }
        let Some((duration, curve)) = longest else {
            return had_flights;
        };
        self.roots = self.flights.iter().map(|flight| flight.picture).collect();
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
        let mix = |from: f32, to: f32| from + (to - from) * progress;
        for flight in &self.flights {
            let frame = Rect::new(
                mix(flight.from.x, flight.to.x),
                mix(flight.from.y, flight.to.y),
                mix(flight.from.width, flight.to.width),
                mix(flight.from.height, flight.to.height),
            );
            ctx.scene.set_synthetic_frame(flight.picture, frame);
            if flight.from_radius != flight.to_radius {
                ctx.scene.set_prop(
                    flight.picture,
                    PropKey::BorderRadius,
                    Some(PropValue::Float(f64::from(mix(
                        flight.from_radius,
                        flight.to_radius,
                    )))),
                );
            }
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
            ctx.scene.remove_synthetic(flight.picture);
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
}
