//! Gesture detectors: tap, pan, swipe and long press with the pointer;
//! pinch and rotation with the trackpad (or Control + wheel); the `Drag`
//! contract (a target that follows the pointer on one axis, drives other
//! properties and settles on a snap point); and native transforms (pan,
//! zoom and rotate the detector's content without a round trip to PHP).

use pam_desktop_wire::{PropKey, PropValue};

use crate::animator::{Curve, Easing, MotionProperty, Owner, Spring, Timer};
use crate::backend::{GesturePhase, NodeId, PointerButton, PointerEvent, PointerPhase};
use crate::context::{Ctx, event_kind};
use crate::geometry::{Point, Size};
use crate::props::PropMap;
use crate::scene::{Change, IdMap, StyleLayer};
use crate::wiremap::WireMap;

/// Travel that turns a press into something else when a detector gives no
/// distance of its own.
const TAP_SLOP: f32 = 12.0;
/// A wheel pinch ends after this long without another notch.
const WHEEL_IDLE_MS: u64 = 180;
const NATIVE_RESET_MS: u32 = 180;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Tap = 1,
    Pan = 2,
    Pinch = 3,
    Rotation = 4,
    Swipe = 5,
    LongPress = 6,
}

impl Kind {
    fn from_wire(value: i64) -> Option<Self> {
        Some(match value {
            1 => Self::Tap,
            2 => Self::Pan,
            3 => Self::Pinch,
            4 => Self::Rotation,
            5 => Self::Swipe,
            6 => Self::LongPress,
            _ => return None,
        })
    }
}

/// A length in logical units or as a fraction of the target's extent.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Length {
    Units(f32),
    Percent(f32),
}

impl Length {
    fn parse(token: &str) -> Option<Self> {
        let token = token.trim();
        match token.strip_suffix('%') {
            Some(percent) => percent
                .trim()
                .parse::<f32>()
                .ok()
                .filter(|value| value.is_finite())
                .map(Self::Percent),
            None => token
                .parse::<f32>()
                .ok()
                .filter(|value| value.is_finite())
                .map(Self::Units),
        }
    }

    fn resolve(self, extent: f32) -> f32 {
        match self {
            Self::Units(value) => value,
            Self::Percent(percent) => percent / 100.0 * extent,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Settle {
    Spring(Spring),
    Timing(u32, Easing),
}

impl Settle {
    fn parse(token: &str) -> Option<Self> {
        if let Some(spring) = Spring::from_token(token) {
            return Some(Self::Spring(spring));
        }
        let mut fields = token
            .trim()
            .strip_prefix("timing")?
            .trim_start_matches(':')
            .splitn(2, ':');
        let duration = fields
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(200)
            .min(10_000);
        let easing = fields.next().map_or(Easing::OutCubic, Easing::from_token);
        Some(Self::Timing(duration, easing))
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Driven {
    Property(MotionProperty),
    Scale,
    BorderRadius,
    /// A text label: format (1 number, 2 clock, 3 clock with total) and decimals.
    Text(u8, u8),
}

#[derive(Clone, Debug, PartialEq)]
struct Driver {
    /// `nativeRef` of the driven node; empty = the drag target.
    reference: String,
    driven: Driven,
    /// Reads the raw pointer position and extrapolates past the end points.
    raw: bool,
    input: Vec<Length>,
    output: Vec<Length>,
}

/// The parsed `GestureDrag` property.
#[derive(Clone, Debug, PartialEq)]
struct Drag {
    horizontal: bool,
    min: Option<f32>,
    max: Option<f32>,
    rubber: f32,
    target: String,
    touch: Option<f32>,
    snap_on_release: bool,
    snaps: Vec<Length>,
    settle: Settle,
    settles: Vec<(usize, Settle)>,
    holds: Vec<usize>,
    threshold: f32,
    velocity: f32,
    group: String,
    drivers: Vec<Driver>,
}

fn lengths(list: &str) -> Vec<Length> {
    list.split(',').filter_map(Length::parse).collect()
}

impl Drag {
    fn parse(text: &str) -> Option<Self> {
        if text.trim().is_empty() {
            return None;
        }
        let mut drag = Self {
            horizontal: true,
            min: None,
            max: None,
            rubber: 0.0,
            target: String::new(),
            touch: None,
            snap_on_release: true,
            snaps: vec![Length::Units(0.0)],
            settle: Settle::Spring(Spring {
                stiffness: 260.0,
                damping: 22.0,
                mass: 1.0,
                velocity: 0.0,
            }),
            settles: Vec::new(),
            holds: Vec::new(),
            threshold: 0.0,
            velocity: 0.0,
            group: String::new(),
            drivers: Vec::new(),
        };
        let mut unbounded = false;
        for line in text.split(['\n', ';']) {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            let number = || {
                value
                    .parse::<f32>()
                    .ok()
                    .filter(|number| number.is_finite())
            };
            match key {
                "axis" => drag.horizontal = value != "y",
                "min" => drag.min = number(),
                "max" => drag.max = number(),
                "rubber" => drag.rubber = number().unwrap_or(0.0).clamp(0.0, 1.0),
                "target" => drag.target = value.to_owned(),
                "touch" => drag.touch = number(),
                "snapOnRelease" => drag.snap_on_release = value != "0",
                "snaps" => {
                    let snaps = lengths(value);
                    if !snaps.is_empty() {
                        drag.snaps = snaps.into_iter().take(16).collect();
                    }
                }
                "settle" => drag.settle = Settle::parse(value).unwrap_or(drag.settle),
                "hold" => {
                    drag.holds = value
                        .split(',')
                        .filter_map(|index| index.trim().parse().ok())
                        .collect()
                }
                "threshold" => drag.threshold = number().unwrap_or(0.0).max(0.0),
                "velocity" => drag.velocity = number().unwrap_or(0.0).max(0.0),
                "group" => drag.group = value.to_owned(),
                "driveUnbounded" => unbounded = value == "1",
                "drive" | "driveRaw" if drag.drivers.len() < 24 => {
                    let fields: Vec<&str> = value.split('|').collect();
                    if let [reference, property, input, output] = fields[..] {
                        let driven = match property {
                            "opacity" => Driven::Property(MotionProperty::Opacity),
                            "translateX" => Driven::Property(MotionProperty::TranslateX),
                            "translateY" => Driven::Property(MotionProperty::TranslateY),
                            "scaleX" => Driven::Property(MotionProperty::ScaleX),
                            "scaleY" => Driven::Property(MotionProperty::ScaleY),
                            "rotate" => Driven::Property(MotionProperty::Rotate),
                            "scale" => Driven::Scale,
                            "borderRadius" => Driven::BorderRadius,
                            _ => continue,
                        };
                        let (input, output) = (lengths(input), lengths(output));
                        if input.len() >= 2 && input.len() == output.len() {
                            drag.drivers.push(Driver {
                                reference: reference.to_owned(),
                                driven,
                                raw: key == "driveRaw",
                                input,
                                output,
                            });
                        }
                    }
                }
                "text" if drag.drivers.len() < 24 => {
                    let fields: Vec<&str> = value.split('|').collect();
                    if let [reference, format, decimals, input, output] = fields[..] {
                        let (input, output) = (lengths(input), lengths(output));
                        if input.len() >= 2 && input.len() == output.len() {
                            let driven = Driven::Text(
                                format.parse().unwrap_or(1),
                                decimals.parse::<u8>().unwrap_or(0).min(3),
                            );
                            drag.drivers.push(Driver {
                                reference: reference.to_owned(),
                                driven,
                                raw: false,
                                input,
                                output,
                            });
                        }
                    }
                }
                _ => {
                    if let Some(index) = key
                        .strip_prefix("settle.")
                        .and_then(|index| index.parse::<usize>().ok())
                        && let Some(settle) = Settle::parse(value)
                    {
                        drag.settles.push((index, settle));
                    }
                }
            }
        }
        if unbounded {
            for driver in &mut drag.drivers {
                driver.raw = true;
            }
        }
        Some(drag)
    }

    fn clamp(&self, raw: f32) -> f32 {
        let mut position = raw;
        if let Some(max) = self.max
            && position > max
        {
            position = max + (position - max) * self.rubber;
        }
        if let Some(min) = self.min
            && position < min
        {
            position = min + (position - min) * self.rubber;
        }
        position
    }

    fn settle_for(&self, index: usize) -> Settle {
        self.settles
            .iter()
            .find(|(snap, _)| *snap == index)
            .map_or(self.settle, |(_, settle)| *settle)
    }
}

/// Piecewise-linear interpolation; `extrapolate` continues the outer segments.
fn interpolate(input: &[f32], output: &[f32], value: f32, extrapolate: bool) -> f32 {
    let last = input.len() - 1;
    let segment = |index: usize| {
        let span = input[index + 1] - input[index];
        if span.abs() < f32::EPSILON {
            return output[index + 1];
        }
        output[index] + (output[index + 1] - output[index]) * (value - input[index]) / span
    };
    let ascending = input[last] >= input[0];
    let before = if ascending {
        value <= input[0]
    } else {
        value >= input[0]
    };
    let after = if ascending {
        value >= input[last]
    } else {
        value <= input[last]
    };
    if before {
        return if extrapolate { segment(0) } else { output[0] };
    }
    if after {
        return if extrapolate {
            segment(last - 1)
        } else {
            output[last]
        };
    }
    for index in 0..last {
        let (low, high) = if ascending {
            (input[index], input[index + 1])
        } else {
            (input[index + 1], input[index])
        };
        if value >= low && value <= high {
            return segment(index);
        }
    }
    output[last]
}

fn clock(seconds: f32) -> String {
    crate::media::format_clock(f64::from(seconds))
}

/// The transform a native gesture applies to the detector's content.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pose {
    x: f32,
    y: f32,
    scale: f32,
    /// Degrees.
    rotation: f32,
}

impl Pose {
    const IDENTITY: Self = Self {
        x: 0.0,
        y: 0.0,
        scale: 1.0,
        rotation: 0.0,
    };

    fn lerp(self, other: Self, t: f32) -> Self {
        let mix = |from: f32, to: f32| from + (to - from) * t;
        Self {
            x: mix(self.x, other.x),
            y: mix(self.y, other.y),
            scale: mix(self.scale, other.scale),
            rotation: mix(self.rotation, other.rotation),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Native {
    min_scale: f32,
    max_scale: f32,
    limit_x: f32,
    reset_on_end: bool,
    focal: bool,
    bounds: bool,
    aspect: f32,
}

struct Detector {
    kind: Kind,
    enabled: bool,
    min_pointers: i64,
    direction: i64,
    min_distance: f32,
    min_duration: u64,
    drag: Option<Drag>,
    native: Option<Native>,
    // ---- drag state
    /// Current translation of the target on the drag axis.
    position: f32,
    /// Raw (unclamped) position, read by raw drivers.
    raw: f32,
    start: f32,
    origin: usize,
    threshold_reached: bool,
    /// Snap index a settle animation is heading to, with its end points.
    settling: Option<(i64, f32, f32)>,
    snap_request: i64,
    // ---- native transform state
    pose: Pose,
    base: Pose,
    reset_key: i64,
    /// A release or reset animation of the pose.
    returning: Option<(Pose, Pose)>,
    dirty: bool,
}

/// A pointer gesture in progress.
struct Active {
    id: NodeId,
    start: Point,
    started_ms: u64,
    last: Point,
    last_ms: u64,
    velocity: Point,
    recognised: bool,
    /// The latest update, sent once per pump.
    pending: Option<Vec<u8>>,
}

/// A trackpad (or wheel) pinch or rotation in progress.
struct Continuous {
    id: NodeId,
    position: Point,
    value: f32,
}

/// What a pointer event meant to the detectors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing recognised: the event goes on to presses.
    Ignored,
    /// A gesture recognised now: the press in progress must be cancelled.
    Recognised,
    /// A recognised gesture owns the pointer.
    Consumed,
}

#[derive(Default)]
pub struct Gestures {
    detectors: IdMap<Detector>,
    active: Option<Active>,
    pinch: Option<Continuous>,
    rotation: Option<Continuous>,
    dirty: bool,
}

struct Sample {
    kind: Kind,
    state: i64,
    local: Point,
    page: Point,
    translation: Point,
    velocity: Point,
    scale: f32,
    rotation: f32,
    snap_index: i64,
    threshold: bool,
    native: Option<Pose>,
}

fn payload(sample: &Sample, timestamp: u64) -> Vec<u8> {
    let mut map = WireMap::new()
        .with("type", sample.kind as i64)
        .with("state", sample.state)
        .with("x", sample.local.x)
        .with("y", sample.local.y)
        .with("pageX", sample.page.x)
        .with("pageY", sample.page.y)
        .with("translationX", sample.translation.x)
        .with("translationY", sample.translation.y)
        .with("velocityX", sample.velocity.x)
        .with("velocityY", sample.velocity.y)
        .with("scale", sample.scale)
        .with("rotation", sample.rotation)
        .with("pointerCount", 1_i64)
        .with("timestamp", i64::try_from(timestamp).unwrap_or(i64::MAX))
        .with("snapIndex", sample.snap_index)
        .with("thresholdReached", sample.threshold);
    if let Some(pose) = sample.native {
        map = map
            .with("nativeScale", pose.scale)
            .with("nativeTranslationX", pose.x)
            .with("nativeTranslationY", pose.y);
    }
    map.encode()
}

fn direction_ok(direction: i64, dx: f32, dy: f32) -> bool {
    match direction {
        2 => dx < 0.0 && dx.abs() >= dy.abs(),
        3 => dx > 0.0 && dx.abs() >= dy.abs(),
        4 => dy < 0.0 && dy.abs() >= dx.abs(),
        5 => dy > 0.0 && dy.abs() >= dx.abs(),
        6 => dx.abs() >= dy.abs(),
        7 => dy.abs() >= dx.abs(),
        _ => true,
    }
}

impl Gestures {
    pub fn on_change(&mut self, ctx: &mut Ctx<'_>, change: Change) {
        match change {
            Change::Created(id, _) => {
                if ctx
                    .scene
                    .node(id)
                    .is_some_and(|node| node.handles(PropKey::GestureType))
                {
                    self.track(id);
                }
            }
            Change::Updated(id, key) => {
                if key == PropKey::GestureType && !self.detectors.contains_key(&id) {
                    self.track(id);
                } else if let Some(detector) = self.detectors.get_mut(&id) {
                    detector.dirty = true;
                    self.dirty = true;
                }
            }
            Change::Removed(id, _) => {
                self.detectors.remove(&id);
                if self.active.as_ref().is_some_and(|active| active.id == id) {
                    self.active = None;
                }
                if self.pinch.as_ref().is_some_and(|pinch| pinch.id == id) {
                    self.pinch = None;
                }
                if self
                    .rotation
                    .as_ref()
                    .is_some_and(|rotation| rotation.id == id)
                {
                    self.rotation = None;
                }
            }
            _ => {}
        }
    }

    fn track(&mut self, id: NodeId) {
        self.detectors.insert(
            id,
            Detector {
                kind: Kind::Tap,
                enabled: false,
                min_pointers: 1,
                direction: 1,
                min_distance: TAP_SLOP,
                min_duration: 0,
                drag: None,
                native: None,
                position: 0.0,
                raw: 0.0,
                start: 0.0,
                origin: 0,
                threshold_reached: false,
                settling: None,
                snap_request: -1,
                pose: Pose::IDENTITY,
                base: Pose::IDENTITY,
                reset_key: 0,
                returning: None,
                dirty: true,
            },
        );
        self.dirty = true;
    }

    pub fn after_commit(&mut self, ctx: &mut Ctx<'_>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let dirty: Vec<NodeId> = self
            .detectors
            .iter()
            .filter(|(_, detector)| detector.dirty)
            .map(|(id, _)| *id)
            .collect();
        for id in dirty {
            self.sync(ctx, id);
        }
    }

    /// Reads the detector's properties.
    fn sync(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(node) = ctx.scene.node(id) else {
            return;
        };
        let props = node.props();
        let Some(kind) = props
            .integer(PropKey::GestureType)
            .and_then(Kind::from_wire)
        else {
            self.detectors.remove(&id);
            return;
        };
        let drag = (kind == Kind::Pan)
            .then(|| Drag::parse(props.text_or_empty(PropKey::GestureDrag)))
            .flatten();
        let native = (props.flag_or(PropKey::GestureNativeTransform, false) && drag.is_none())
            .then(|| {
                let min_scale = props
                    .number_or(PropKey::GestureNativeMinScale, 1.0)
                    .max(0.01);
                Native {
                    min_scale,
                    max_scale: props
                        .number_or(PropKey::GestureNativeMaxScale, 4.0)
                        .max(min_scale),
                    limit_x: props
                        .number_or(PropKey::GestureNativeTranslationLimitX, 0.0)
                        .max(0.0),
                    reset_on_end: props.flag_or(PropKey::GestureNativeResetOnEnd, false),
                    focal: props.flag_or(PropKey::GestureNativeFocalZoom, false),
                    bounds: props.flag_or(PropKey::GestureNativeBounds, false),
                    aspect: props
                        .number_or(PropKey::GestureNativeContentAspect, 0.0)
                        .max(0.0),
                }
            });
        let default_distance = match kind {
            Kind::Swipe => 48.0,
            Kind::Pan => 8.0,
            _ => TAP_SLOP,
        };
        let (enabled, min_pointers, direction) = (
            props.flag_or(PropKey::GestureEnabled, true),
            props.integer_or(PropKey::GestureMinPointers, 1),
            props.integer_or(PropKey::GestureDirection, 1),
        );
        let min_distance = props
            .number(PropKey::GestureMinDistance)
            .unwrap_or(default_distance)
            .max(0.0);
        let min_duration = props
            .integer_or(
                PropKey::GestureMinDurationMs,
                if kind == Kind::LongPress { 500 } else { 0 },
            )
            .clamp(0, 60_000) as u64;
        let snap_request = props.integer(PropKey::GestureDragSnapIndex).unwrap_or(-1);
        let reset_key = props.integer_or(PropKey::GestureNativeResetKey, 0);
        let Some(detector) = self.detectors.get_mut(&id) else {
            return;
        };
        detector.dirty = false;
        (
            detector.kind,
            detector.enabled,
            detector.min_pointers,
            detector.direction,
        ) = (kind, enabled, min_pointers, direction);
        (detector.min_distance, detector.min_duration) = (min_distance, min_duration);
        let drag_changed = detector.drag != drag;
        detector.drag = drag;
        detector.native = native;
        if detector.reset_key != reset_key {
            detector.reset_key = reset_key;
            let surface = self.surface(ctx, id);
            if self
                .detectors
                .get(&surface)
                .is_some_and(|owner| owner.pose != Pose::IDENTITY)
            {
                self.return_pose(ctx, surface, Pose::IDENTITY);
            }
        }
        let Some(detector) = self.detectors.get_mut(&id) else {
            return;
        };
        if detector.snap_request != snap_request {
            detector.snap_request = snap_request;
            if snap_request >= 0 && detector.drag.is_some() {
                self.settle_to(ctx, id, (snap_request % 64) as usize, 0.0);
                return;
            }
        }
        if drag_changed
            && self
                .detectors
                .get(&id)
                .is_some_and(|detector| detector.drag.is_some())
        {
            // A new contract keeps the target where it is and refreshes the drivers.
            self.write_drag(ctx, id);
        }
    }

    // ---- pointer gestures --------------------------------------------------

    fn detector_at(
        &self,
        ctx: &Ctx<'_>,
        position: Point,
        accepts: impl Fn(&Detector) -> bool,
    ) -> Option<NodeId> {
        let mut current = ctx.scene.hit_test(position)?;
        loop {
            if self
                .detectors
                .get(&current)
                .is_some_and(|detector| detector.enabled && accepts(detector))
            {
                return Some(current);
            }
            let node = ctx.scene.node(current)?;
            if node.parent == 0 {
                return None;
            }
            current = node.parent;
        }
    }

    fn sample(&self, ctx: &Ctx<'_>, active: &Active, state: i64, position: Point) -> Sample {
        let origin = ctx.scene.window_origin(active.id);
        let detector = self.detectors.get(&active.id);
        Sample {
            kind: detector.map_or(Kind::Pan, |detector| detector.kind),
            state,
            local: Point::new(position.x - origin.x, position.y - origin.y),
            page: position,
            translation: Point::new(position.x - active.start.x, position.y - active.start.y),
            velocity: active.velocity,
            scale: 1.0,
            rotation: 0.0,
            snap_index: -1,
            threshold: false,
            native: self.native_pose(ctx, active.id),
        }
    }

    pub fn on_pointer(&mut self, ctx: &mut Ctx<'_>, pointer: PointerEvent) -> Outcome {
        match pointer.phase {
            PointerPhase::Down if pointer.button == PointerButton::Primary => {
                // A mouse is one pointer: detectors that need more never start.
                let id = self.detector_at(ctx, pointer.position, |detector| {
                    !matches!(detector.kind, Kind::Pinch | Kind::Rotation)
                        && detector.min_pointers <= 1
                });
                self.active = id.map(|id| Active {
                    id,
                    start: pointer.position,
                    started_ms: ctx.now_ms,
                    last: pointer.position,
                    last_ms: ctx.now_ms,
                    velocity: Point::ZERO,
                    recognised: false,
                    pending: None,
                });
                if let Some(detector) = id.and_then(|id| self.detectors.get(&id))
                    && detector.kind == Kind::LongPress
                {
                    ctx.timers.schedule(
                        Timer::GestureHold,
                        ctx.now_ms + detector.min_duration.max(1),
                    );
                }
                Outcome::Ignored
            }
            PointerPhase::Move => self.on_move(ctx, pointer.position),
            PointerPhase::Up if pointer.button == PointerButton::Primary => {
                self.on_up(ctx, pointer.position)
            }
            PointerPhase::Cancel => {
                let Some(active) = self.active.take() else {
                    return Outcome::Ignored;
                };
                ctx.timers.cancel(Timer::GestureHold);
                if !active.recognised {
                    return Outcome::Ignored;
                }
                let sample = self.sample(ctx, &active, 4, active.last);
                ctx.emit_if(
                    active.id,
                    PropKey::OnGestureCancel,
                    event_kind::GESTURE_CANCEL,
                    payload(&sample, ctx.now_ms),
                );
                self.release_drag(ctx, active.id, 0.0, true);
                self.release_native(ctx, active.id);
                Outcome::Consumed
            }
            _ => Outcome::Ignored,
        }
    }

    fn on_move(&mut self, ctx: &mut Ctx<'_>, position: Point) -> Outcome {
        let Some(active) = &mut self.active else {
            return Outcome::Ignored;
        };
        let elapsed = ctx.now_ms.saturating_sub(active.last_ms);
        if elapsed > 0 {
            // Smoothed, in logical units per second.
            let instant = Point::new(
                (position.x - active.last.x) * 1000.0 / elapsed as f32,
                (position.y - active.last.y) * 1000.0 / elapsed as f32,
            );
            active.velocity = Point::new(
                active.velocity.x * 0.4 + instant.x * 0.6,
                active.velocity.y * 0.4 + instant.y * 0.6,
            );
            active.last_ms = ctx.now_ms;
        }
        let previous = active.last;
        active.last = position;
        let id = active.id;
        let (dx, dy) = (position.x - active.start.x, position.y - active.start.y);
        let held = ctx.now_ms.saturating_sub(active.started_ms);
        let recognised = active.recognised;
        let Some(detector) = self.detectors.get(&id) else {
            return Outcome::Ignored;
        };
        let kind = detector.kind;
        if !recognised {
            match kind {
                Kind::Pan => {
                    if held < detector.min_duration
                        || dx.hypot(dy) < detector.min_distance
                        || !direction_ok(detector.direction, dx, dy)
                    {
                        return Outcome::Ignored;
                    }
                }
                Kind::LongPress => {
                    // Moving away before the hold completes gives the press up.
                    if dx.hypot(dy) > TAP_SLOP {
                        ctx.timers.cancel(Timer::GestureHold);
                        self.active = None;
                    }
                    return Outcome::Ignored;
                }
                _ => return Outcome::Ignored,
            }
            self.begin(ctx, id, position);
            return Outcome::Recognised;
        }
        // Recognised: the target follows.
        let local = {
            let origin = ctx.scene.window_origin(id);
            Point::new(position.x - origin.x, position.y - origin.y)
        };
        self.update_drag(ctx, id, Point::new(dx, dy), local);
        self.pan_native(
            ctx,
            id,
            Point::new(position.x - previous.x, position.y - previous.y),
        );
        if let Some(active) = self.active.take() {
            let sample = self.sample(ctx, &active, 2, position);
            self.active = Some(Active {
                pending: Some(payload(&sample, ctx.now_ms)),
                ..active
            });
        }
        Outcome::Consumed
    }

    /// The gesture recognised: `Begin`, and the drag or transform starts.
    fn begin(&mut self, ctx: &mut Ctx<'_>, id: NodeId, position: Point) {
        ctx.timers.cancel(Timer::GestureHold);
        let local = {
            let origin = ctx.scene.window_origin(id);
            Point::new(position.x - origin.x, position.y - origin.y)
        };
        self.begin_drag(ctx, id, local);
        self.begin_native(ctx, id);
        if let Some(active) = self.active.take() {
            let sample = self.sample(ctx, &active, 1, position);
            ctx.emit_if(
                id,
                PropKey::OnGestureBegin,
                event_kind::GESTURE_BEGIN,
                payload(&sample, ctx.now_ms),
            );
            self.active = Some(Active {
                recognised: true,
                ..active
            });
        }
    }

    fn on_up(&mut self, ctx: &mut Ctx<'_>, position: Point) -> Outcome {
        let Some(mut active) = self.active.take() else {
            return Outcome::Ignored;
        };
        ctx.timers.cancel(Timer::GestureHold);
        let id = active.id;
        let Some(detector) = self.detectors.get(&id) else {
            return Outcome::Ignored;
        };
        let (dx, dy) = (position.x - active.start.x, position.y - active.start.y);
        let held = ctx.now_ms.saturating_sub(active.started_ms);
        if !active.recognised {
            // Discrete gestures are decided on release.
            let recognised = match detector.kind {
                Kind::Tap => {
                    held >= detector.min_duration
                        && dx.hypot(dy) <= detector.min_distance.max(TAP_SLOP)
                        && direction_ok(detector.direction, dx, dy)
                }
                Kind::Swipe => {
                    dx.hypot(dy) >= detector.min_distance
                        && direction_ok(detector.direction, dx, dy)
                }
                _ => false,
            };
            if recognised {
                let begin = self.sample(ctx, &active, 1, position);
                ctx.emit_if(
                    id,
                    PropKey::OnGestureBegin,
                    event_kind::GESTURE_BEGIN,
                    payload(&begin, ctx.now_ms),
                );
                let end = Sample { state: 3, ..begin };
                ctx.emit_if(
                    id,
                    PropKey::OnGestureEnd,
                    event_kind::GESTURE_END,
                    payload(&end, ctx.now_ms),
                );
            }
            return Outcome::Ignored;
        }
        if let Some(update) = active.pending.take() {
            ctx.emit_if(
                id,
                PropKey::OnGestureUpdate,
                event_kind::GESTURE_UPDATE,
                update,
            );
        }
        let axis_velocity = if self
            .detectors
            .get(&id)
            .and_then(|detector| detector.drag.as_ref())
            .is_some_and(|drag| drag.horizontal)
        {
            active.velocity.x
        } else {
            active.velocity.y
        };
        let snap_index = self.release_drag(ctx, id, axis_velocity, false);
        self.release_native(ctx, id);
        let mut sample = self.sample(ctx, &active, 3, position);
        sample.snap_index = snap_index;
        sample.threshold = self
            .detectors
            .get(&id)
            .is_some_and(|detector| detector.threshold_reached);
        ctx.emit_if(
            id,
            PropKey::OnGestureEnd,
            event_kind::GESTURE_END,
            payload(&sample, ctx.now_ms),
        );
        Outcome::Consumed
    }

    /// The long press was held long enough. `true` means the press in
    /// progress must be cancelled.
    pub fn on_hold(&mut self, ctx: &mut Ctx<'_>) -> bool {
        let Some(active) = &self.active else {
            return false;
        };
        if active.recognised
            || self
                .detectors
                .get(&active.id)
                .is_none_or(|detector| detector.kind != Kind::LongPress)
        {
            return false;
        }
        let (id, position) = (active.id, active.last);
        self.begin(ctx, id, position);
        true
    }

    /// Sends the coalesced update of the gesture in progress.
    pub fn flush(&mut self, ctx: &mut Ctx<'_>) {
        if let Some(active) = &mut self.active
            && let Some(update) = active.pending.take()
        {
            ctx.emit_if(
                active.id,
                PropKey::OnGestureUpdate,
                event_kind::GESTURE_UPDATE,
                update,
            );
        }
    }

    // ---- trackpad gestures -------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn continuous(
        &self,
        ctx: &Ctx<'_>,
        id: NodeId,
        kind: Kind,
        state: i64,
        position: Point,
        scale: f32,
        rotation: f32,
    ) -> Sample {
        let origin = ctx.scene.window_origin(id);
        Sample {
            kind,
            state,
            local: Point::new(position.x - origin.x, position.y - origin.y),
            page: position,
            translation: Point::ZERO,
            velocity: Point::ZERO,
            scale,
            rotation,
            snap_index: -1,
            threshold: false,
            native: self.native_pose(ctx, id),
        }
    }

    /// A pinch from the trackpad: `scale` is relative to the gesture start.
    pub fn on_magnify(
        &mut self,
        ctx: &mut Ctx<'_>,
        position: Point,
        phase: GesturePhase,
        scale: f32,
    ) {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        match phase {
            GesturePhase::Begin => {
                let Some(id) =
                    self.detector_at(ctx, position, |detector| detector.kind == Kind::Pinch)
                else {
                    return;
                };
                self.begin_native(ctx, id);
                self.pinch = Some(Continuous {
                    id,
                    position,
                    value: 1.0,
                });
                let sample = self.continuous(ctx, id, Kind::Pinch, 1, position, 1.0, 0.0);
                ctx.emit_if(
                    id,
                    PropKey::OnGestureBegin,
                    event_kind::GESTURE_BEGIN,
                    payload(&sample, ctx.now_ms),
                );
            }
            GesturePhase::Update => {
                let Some(pinch) = &mut self.pinch else { return };
                pinch.value = scale;
                pinch.position = position;
                let id = pinch.id;
                self.zoom_native(ctx, id, scale, position);
                let sample = self.continuous(ctx, id, Kind::Pinch, 2, position, scale, 0.0);
                ctx.emit_if(
                    id,
                    PropKey::OnGestureUpdate,
                    event_kind::GESTURE_UPDATE,
                    payload(&sample, ctx.now_ms),
                );
            }
            GesturePhase::End | GesturePhase::Cancel => {
                let Some(pinch) = self.pinch.take() else {
                    return;
                };
                self.release_native(ctx, pinch.id);
                let cancelled = phase == GesturePhase::Cancel;
                let sample = self.continuous(
                    ctx,
                    pinch.id,
                    Kind::Pinch,
                    if cancelled { 4 } else { 3 },
                    pinch.position,
                    pinch.value,
                    0.0,
                );
                let (marker, kind) = if cancelled {
                    (PropKey::OnGestureCancel, event_kind::GESTURE_CANCEL)
                } else {
                    (PropKey::OnGestureEnd, event_kind::GESTURE_END)
                };
                ctx.emit_if(pinch.id, marker, kind, payload(&sample, ctx.now_ms));
            }
        }
    }

    /// Control + wheel zooms like a pinch around the pointer. `true` means a
    /// pinch detector took the wheel.
    pub fn on_wheel_zoom(&mut self, ctx: &mut Ctx<'_>, position: Point, delta_y: f32) -> bool {
        let factor = (-delta_y * 0.0025).exp();
        if self.pinch.is_none() {
            self.on_magnify(ctx, position, GesturePhase::Begin, 1.0);
        }
        let Some(current) = self.pinch.as_ref().map(|pinch| pinch.value) else {
            return false;
        };
        self.on_magnify(ctx, position, GesturePhase::Update, current * factor);
        ctx.timers
            .schedule(Timer::GestureIdle, ctx.now_ms + WHEEL_IDLE_MS);
        true
    }

    /// The wheel stopped: the pinch it was driving ends.
    pub fn on_idle(&mut self, ctx: &mut Ctx<'_>) {
        if let Some(position) = self.pinch.as_ref().map(|pinch| pinch.position) {
            self.on_magnify(ctx, position, GesturePhase::End, 1.0);
        }
    }

    /// A two-finger rotation from the trackpad, in radians since its start.
    pub fn on_rotate(
        &mut self,
        ctx: &mut Ctx<'_>,
        position: Point,
        phase: GesturePhase,
        radians: f32,
    ) {
        let radians = if radians.is_finite() { radians } else { 0.0 };
        match phase {
            GesturePhase::Begin => {
                let Some(id) =
                    self.detector_at(ctx, position, |detector| detector.kind == Kind::Rotation)
                else {
                    return;
                };
                self.begin_native(ctx, id);
                self.rotation = Some(Continuous {
                    id,
                    position,
                    value: 0.0,
                });
                let sample = self.continuous(ctx, id, Kind::Rotation, 1, position, 1.0, 0.0);
                ctx.emit_if(
                    id,
                    PropKey::OnGestureBegin,
                    event_kind::GESTURE_BEGIN,
                    payload(&sample, ctx.now_ms),
                );
            }
            GesturePhase::Update => {
                let Some(rotation) = &mut self.rotation else {
                    return;
                };
                rotation.value = radians;
                rotation.position = position;
                let id = rotation.id;
                self.rotate_native(ctx, id, radians);
                let sample = self.continuous(ctx, id, Kind::Rotation, 2, position, 1.0, radians);
                ctx.emit_if(
                    id,
                    PropKey::OnGestureUpdate,
                    event_kind::GESTURE_UPDATE,
                    payload(&sample, ctx.now_ms),
                );
            }
            GesturePhase::End | GesturePhase::Cancel => {
                let Some(rotation) = self.rotation.take() else {
                    return;
                };
                self.release_native(ctx, rotation.id);
                let cancelled = phase == GesturePhase::Cancel;
                let sample = self.continuous(
                    ctx,
                    rotation.id,
                    Kind::Rotation,
                    if cancelled { 4 } else { 3 },
                    rotation.position,
                    1.0,
                    rotation.value,
                );
                let (marker, kind) = if cancelled {
                    (PropKey::OnGestureCancel, event_kind::GESTURE_CANCEL)
                } else {
                    (PropKey::OnGestureEnd, event_kind::GESTURE_END)
                };
                ctx.emit_if(rotation.id, marker, kind, payload(&sample, ctx.now_ms));
            }
        }
    }

    // ---- drag ----------------------------------------------------------------

    /// The node a drag moves: the `target` reference, or the detector's first child.
    fn drag_target(ctx: &Ctx<'_>, id: NodeId, reference: &str) -> Option<NodeId> {
        if reference.is_empty() {
            return ctx.scene.node(id)?.children.first().copied();
        }
        ctx.scene.find_native_ref(id, reference)
    }

    fn extent(ctx: &Ctx<'_>, target: Option<NodeId>, horizontal: bool) -> f32 {
        target
            .and_then(|target| ctx.scene.node(target))
            .map_or(0.0, |node| {
                if horizontal {
                    node.frame.width
                } else {
                    node.frame.height
                }
            })
    }

    fn snaps(ctx: &Ctx<'_>, id: NodeId, drag: &Drag) -> Vec<f32> {
        let extent = Self::extent(
            ctx,
            Self::drag_target(ctx, id, &drag.target),
            drag.horizontal,
        );
        drag.snaps.iter().map(|snap| snap.resolve(extent)).collect()
    }

    fn begin_drag(&mut self, ctx: &mut Ctx<'_>, id: NodeId, local: Point) {
        let Some(drag) = self
            .detectors
            .get(&id)
            .and_then(|detector| detector.drag.clone())
        else {
            return;
        };
        let position = ctx
            .animator
            .cancel(Owner::Gesture(id), ctx.now_ms)
            .or_else(|| self.detectors.get(&id).map(|detector| detector.position))
            .unwrap_or(0.0);
        let snaps = Self::snaps(ctx, id, &drag);
        let start = match drag.touch {
            Some(inset) => (if drag.horizontal { local.x } else { local.y }) - inset,
            None => position,
        };
        let origin = snaps
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1 - start).abs().total_cmp(&(b.1 - start).abs()))
            .map_or(0, |(index, _)| index);
        if let Some(detector) = self.detectors.get_mut(&id) {
            detector.settling = None;
            detector.start = start;
            detector.position = position;
            detector.origin = origin;
            detector.threshold_reached = false;
        }
        // Only one member of a group rests away from its first snap.
        if !drag.group.is_empty() {
            let others: Vec<NodeId> = self
                .detectors
                .iter()
                .filter(|(other, detector)| {
                    **other != id
                        && detector
                            .drag
                            .as_ref()
                            .is_some_and(|other| other.group == drag.group)
                        && detector.position != 0.0
                })
                .map(|(other, _)| *other)
                .collect();
            for other in others {
                self.settle_to(ctx, other, 0, 0.0);
            }
        }
    }

    fn update_drag(&mut self, ctx: &mut Ctx<'_>, id: NodeId, translation: Point, local: Point) {
        let Some(detector) = self.detectors.get_mut(&id) else {
            return;
        };
        let Some(drag) = &detector.drag else { return };
        let delta = if drag.horizontal {
            translation.x
        } else {
            translation.y
        };
        let raw = match drag.touch {
            Some(inset) => (if drag.horizontal { local.x } else { local.y }) - inset,
            None => detector.start + delta,
        };
        detector.raw = raw;
        detector.position = drag.clamp(raw);
        let (threshold, origin, position, drag) = (
            drag.threshold,
            detector.origin,
            detector.position,
            drag.clone(),
        );
        if threshold > 0.0 {
            let snap = Self::snaps(ctx, id, &drag)
                .get(origin)
                .copied()
                .unwrap_or(0.0);
            if let Some(detector) = self.detectors.get_mut(&id) {
                detector.threshold_reached = (position - snap).abs() >= threshold;
            }
        }
        self.write_drag(ctx, id);
    }

    /// Decides where a released drag rests and starts the settle animation.
    /// Returns the snap index reported by `End` (-1 for a free drag).
    fn release_drag(
        &mut self,
        ctx: &mut Ctx<'_>,
        id: NodeId,
        velocity: f32,
        cancelled: bool,
    ) -> i64 {
        let Some(detector) = self.detectors.get(&id) else {
            return -1;
        };
        let Some(drag) = detector.drag.clone() else {
            return -1;
        };
        let (position, origin) = (detector.position, detector.origin);
        if !drag.snap_on_release {
            ctx.emit_if(
                id,
                PropKey::OnGestureSettle,
                event_kind::GESTURE_SETTLE,
                WireMap::new()
                    .with("snapIndex", -1_i64)
                    .with("position", position)
                    .encode(),
            );
            return -1;
        }
        let snaps = Self::snaps(ctx, id, &drag);
        let origin_snap = snaps.get(origin).copied().unwrap_or(0.0);
        let displacement = position - origin_snap;
        let fast = drag.velocity > 0.0
            && velocity.abs() >= drag.velocity
            && (displacement == 0.0 || velocity.signum() == displacement.signum());
        let far = drag.threshold > 0.0 && displacement.abs() >= drag.threshold;
        let index = if cancelled || !(fast || far) {
            origin
        } else {
            // The nearest snap strictly beyond the origin in the direction of travel.
            let direction = if fast {
                velocity.signum()
            } else {
                displacement.signum()
            };
            snaps
                .iter()
                .enumerate()
                .filter(|(_, snap)| (**snap - origin_snap) * direction > 0.0)
                .min_by(|a, b| {
                    (a.1 - origin_snap)
                        .abs()
                        .total_cmp(&(b.1 - origin_snap).abs())
                })
                .map_or(origin, |(index, _)| index)
        };
        if drag.holds.contains(&index) {
            // Stay where the pointer let go; the application takes over.
            ctx.emit_if(
                id,
                PropKey::OnGestureSettle,
                event_kind::GESTURE_SETTLE,
                WireMap::new()
                    .with("snapIndex", index as i64)
                    .with("position", position)
                    .encode(),
            );
            return index as i64;
        }
        self.settle_to(ctx, id, index, velocity);
        index as i64
    }

    /// Animates the drag target to a snap point.
    fn settle_to(&mut self, ctx: &mut Ctx<'_>, id: NodeId, index: usize, velocity: f32) {
        let Some(detector) = self.detectors.get(&id) else {
            return;
        };
        let Some(drag) = detector.drag.clone() else {
            return;
        };
        let snaps = Self::snaps(ctx, id, &drag);
        let Some(target) = snaps.get(index).copied() else {
            return;
        };
        let from = ctx
            .animator
            .cancel(Owner::Gesture(id), ctx.now_ms)
            .unwrap_or(detector.position);
        if let Some(detector) = self.detectors.get_mut(&id) {
            detector.settling = Some((index as i64, from, target));
        }
        if ctx.reduced_motion || (from - target).abs() < 0.01 {
            return self.on_sample(ctx, id, target, true);
        }
        let curve = match drag.settle_for(index) {
            Settle::Spring(spring) => Curve::spring(Spring { velocity, ..spring }, from, target),
            Settle::Timing(duration, easing) => Curve::timing(duration.max(1), easing),
        };
        ctx.animator
            .start(Owner::Gesture(id), from, target, curve, 0, ctx.now_ms);
    }

    /// A frame of a settle animation.
    pub fn on_sample(&mut self, ctx: &mut Ctx<'_>, id: NodeId, value: f32, finished: bool) {
        let Some(detector) = self.detectors.get_mut(&id) else {
            return;
        };
        detector.position = value;
        detector.raw = value;
        let settled = finished.then(|| detector.settling.take()).flatten();
        self.write_drag(ctx, id);
        if let Some((index, _, target)) = settled {
            ctx.emit_if(
                id,
                PropKey::OnGestureSettle,
                event_kind::GESTURE_SETTLE,
                WireMap::new()
                    .with("snapIndex", index)
                    .with("position", target)
                    .encode(),
            );
        }
    }

    /// Writes the drag position to the target and runs the drivers.
    fn write_drag(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(detector) = self.detectors.get(&id) else {
            return;
        };
        let Some(drag) = &detector.drag else { return };
        let (position, raw) = (detector.position, detector.raw);
        let Some(target) = Self::drag_target(ctx, id, &drag.target) else {
            return;
        };
        let mut values = ctx.scene.motion(target);
        values.set(
            if drag.horizontal {
                MotionProperty::TranslateX
            } else {
                MotionProperty::TranslateY
            },
            Some(position),
        );
        ctx.scene.set_motion(target, values);
        let extent = Self::extent(ctx, Some(target), drag.horizontal);
        for driver in &drag.drivers {
            let driven = if driver.reference.is_empty() {
                Some(target)
            } else {
                ctx.scene.find_native_ref(id, &driver.reference)
            };
            let Some(driven) = driven else { continue };
            let input: Vec<f32> = driver
                .input
                .iter()
                .map(|point| point.resolve(extent))
                .collect();
            // Outputs given in percent refer to the driven node itself.
            let driven_size = ctx
                .scene
                .node(driven)
                .map_or(Size::ZERO, |node| node.frame.size());
            let output_extent = match driver.driven {
                Driven::Property(MotionProperty::TranslateY) => driven_size.height,
                _ => driven_size.width,
            };
            let output: Vec<f32> = driver
                .output
                .iter()
                .map(|point| point.resolve(output_extent))
                .collect();
            let value = interpolate(
                &input,
                &output,
                if driver.raw { raw } else { position },
                driver.raw,
            );
            match &driver.driven {
                Driven::Property(property) => {
                    let mut values = ctx.scene.motion(driven);
                    values.set(
                        *property,
                        Some(if *property == MotionProperty::Opacity {
                            value.clamp(0.0, 1.0)
                        } else {
                            value
                        }),
                    );
                    ctx.scene.set_motion(driven, values);
                }
                Driven::Scale => {
                    let mut values = ctx.scene.motion(driven);
                    values.set(MotionProperty::ScaleX, Some(value));
                    values.set(MotionProperty::ScaleY, Some(value));
                    ctx.scene.set_motion(driven, values);
                }
                Driven::BorderRadius => {
                    let mut overlay = PropMap::new();
                    overlay.insert(
                        PropKey::BorderRadius,
                        PropValue::Float(f64::from(value.max(0.0))),
                    );
                    overlay.insert(PropKey::Overflow, PropValue::Integer(2));
                    ctx.scene
                        .set_style_overlay(driven, StyleLayer::Presenter, Some(overlay));
                }
                Driven::Text(format, decimals) => {
                    let text = match format {
                        2 => clock(value),
                        3 => format!(
                            "{} / {}",
                            clock(value),
                            clock(output.last().copied().unwrap_or(0.0))
                        ),
                        _ => format!("{value:.*}", usize::from(*decimals)),
                    };
                    if ctx.scene.plain_text(driven).as_deref() != Some(text.as_str()) {
                        let mut overlay = PropMap::new();
                        overlay.insert(PropKey::Text, PropValue::String(text));
                        ctx.scene
                            .set_style_overlay(driven, StyleLayer::Presenter, Some(overlay));
                    }
                }
            }
        }
    }

    // ---- native transform ----------------------------------------------------

    /// The detector that holds the pose of the surface `id` transforms:
    /// nested native-transform detectors (a pan around a pinch) share the
    /// innermost one, so both gestures move the same content.
    fn surface(&self, ctx: &Ctx<'_>, id: NodeId) -> NodeId {
        let mut current = id;
        for _ in 0..4 {
            let inner = ctx
                .scene
                .node(current)
                .and_then(|node| node.children.first().copied());
            match inner {
                Some(inner)
                    if self
                        .detectors
                        .get(&inner)
                        .is_some_and(|detector| detector.native.is_some()) =>
                {
                    current = inner
                }
                _ => break,
            }
        }
        current
    }

    fn native_of(&self, id: NodeId) -> Option<Native> {
        self.detectors.get(&id).and_then(|detector| detector.native)
    }

    /// The pose reported in gesture events of `id`, when it transforms natively.
    fn native_pose(&self, ctx: &Ctx<'_>, id: NodeId) -> Option<Pose> {
        self.native_of(id)?;
        self.detectors
            .get(&self.surface(ctx, id))
            .map(|detector| detector.pose)
    }

    fn begin_native(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        if self.native_of(id).is_none() {
            return;
        }
        let surface = self.surface(ctx, id);
        let Some(owner) = self.detectors.get_mut(&surface) else {
            return;
        };
        if owner.returning.take().is_some() {
            // Take over a release animation where it is.
            ctx.animator.cancel(Owner::GesturePose(surface), ctx.now_ms);
        }
        owner.base = owner.pose;
    }

    fn pan_native(&mut self, ctx: &mut Ctx<'_>, id: NodeId, delta: Point) {
        let Some(native) = self.native_of(id) else {
            return;
        };
        let surface = self.surface(ctx, id);
        let Some(owner) = self.detectors.get_mut(&surface) else {
            return;
        };
        owner.pose.x += delta.x;
        owner.pose.y += delta.y;
        if native.limit_x > 0.0 {
            owner.pose.x = owner.pose.x.clamp(-native.limit_x, native.limit_x);
        }
        if native.bounds {
            self.bound(ctx, id, surface, native);
        }
        self.write_pose(ctx, surface);
    }

    fn zoom_native(&mut self, ctx: &mut Ctx<'_>, id: NodeId, scale: f32, focal: Point) {
        let Some(native) = self.native_of(id) else {
            return;
        };
        let surface = self.surface(ctx, id);
        let Some(target) = ctx
            .scene
            .node(surface)
            .and_then(|node| node.children.first().copied())
        else {
            return;
        };
        // The pivot of the content: its untranslated centre, in window coordinates.
        let pivot = ctx.scene.node(target).map_or(Point::ZERO, |node| {
            let own = ctx.scene.window_origin(target);
            Point::new(
                own.x + node.frame.width / 2.0,
                own.y + node.frame.height / 2.0,
            )
        });
        let Some(owner) = self.detectors.get_mut(&surface) else {
            return;
        };
        let (base, previous) = (owner.base, owner.pose);
        let next = (base.scale * scale).clamp(native.min_scale, native.max_scale);
        if native.focal && previous.scale > 0.0 {
            // The content point under the pointer stays under it.
            let (fx, fy) = (focal.x - pivot.x, focal.y - pivot.y);
            let ratio = next / previous.scale;
            owner.pose.x = fx - (fx - previous.x) * ratio;
            owner.pose.y = fy - (fy - previous.y) * ratio;
        }
        owner.pose.scale = next;
        if native.bounds {
            self.bound(ctx, id, surface, native);
        }
        self.write_pose(ctx, surface);
    }

    fn rotate_native(&mut self, ctx: &mut Ctx<'_>, id: NodeId, radians: f32) {
        if self.native_of(id).is_none() {
            return;
        }
        let surface = self.surface(ctx, id);
        if let Some(owner) = self.detectors.get_mut(&surface) {
            owner.pose.rotation = owner.base.rotation + radians.to_degrees();
            self.write_pose(ctx, surface);
        }
    }

    /// Keeps the scaled content covering the viewport of detector `id` (or
    /// centred when smaller).
    fn bound(&mut self, ctx: &mut Ctx<'_>, id: NodeId, surface: NodeId, native: Native) {
        let Some(viewport) = ctx.scene.node(id).map(|node| node.frame.size()) else {
            return;
        };
        let child = ctx
            .scene
            .node(surface)
            .and_then(|node| node.children.first().copied())
            .and_then(|target| ctx.scene.node(target))
            .map(|node| node.frame.size());
        let Some(child) = child else { return };
        let Some(owner) = self.detectors.get_mut(&surface) else {
            return;
        };
        // The picture inside the child, contain-fit to the given aspect ratio.
        let content = if native.aspect > 0.0 && child.height > 0.0 {
            if child.width / child.height > native.aspect {
                Size::new(child.height * native.aspect, child.height)
            } else {
                Size::new(child.width, child.width / native.aspect)
            }
        } else {
            child
        };
        let scale = owner.pose.scale;
        let axis = |viewport: f32, content: f32, translation: f32| {
            let scaled = content * scale;
            if scaled <= viewport {
                0.0
            } else {
                let slack = (scaled - viewport) / 2.0;
                translation.clamp(-slack, slack)
            }
        };
        owner.pose.x = axis(viewport.width, content.width, owner.pose.x);
        owner.pose.y = axis(viewport.height, content.height, owner.pose.y);
    }

    /// Applies the pose held by `surface` to its content.
    fn write_pose(&mut self, ctx: &mut Ctx<'_>, surface: NodeId) {
        let Some(pose) = self.detectors.get(&surface).map(|detector| detector.pose) else {
            return;
        };
        let Some(target) = ctx
            .scene
            .node(surface)
            .and_then(|node| node.children.first().copied())
        else {
            return;
        };
        let mut values = ctx.scene.motion(target);
        let identity = pose == Pose::IDENTITY;
        values.set(MotionProperty::TranslateX, (!identity).then_some(pose.x));
        values.set(MotionProperty::TranslateY, (!identity).then_some(pose.y));
        values.set(MotionProperty::ScaleX, (!identity).then_some(pose.scale));
        values.set(MotionProperty::ScaleY, (!identity).then_some(pose.scale));
        values.set(MotionProperty::Rotate, (!identity).then_some(pose.rotation));
        ctx.scene.set_motion(target, values);
    }

    /// The gesture ended: the content goes back, or settles inside its bounds.
    fn release_native(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(native) = self.native_of(id) else {
            return;
        };
        let surface = self.surface(ctx, id);
        let Some(owner) = self.detectors.get(&surface) else {
            return;
        };
        if native.reset_on_end {
            let base = owner.base;
            return self.return_pose(ctx, surface, base);
        }
        if native.bounds {
            let current = owner.pose;
            let mut rest = current;
            rest.scale = current.scale.clamp(1.0, native.max_scale.max(1.0));
            if rest.scale <= 1.02 {
                rest.scale = 1.0;
            }
            if let Some(owner) = self.detectors.get_mut(&surface) {
                owner.pose = rest;
            }
            self.bound(ctx, id, surface, native);
            let Some(owner) = self.detectors.get_mut(&surface) else {
                return;
            };
            let rest = owner.pose;
            owner.pose = current;
            if rest != current {
                self.return_pose(ctx, surface, rest);
            }
        }
    }

    /// Animates the content of `surface` to `rest`.
    fn return_pose(&mut self, ctx: &mut Ctx<'_>, surface: NodeId, rest: Pose) {
        let Some(owner) = self.detectors.get_mut(&surface) else {
            return;
        };
        if ctx.reduced_motion {
            owner.pose = rest;
            owner.returning = None;
            return self.write_pose(ctx, surface);
        }
        owner.returning = Some((owner.pose, rest));
        ctx.animator.start(
            Owner::GesturePose(surface),
            0.0,
            1.0,
            Curve::timing(NATIVE_RESET_MS, Easing::OutCubic),
            0,
            ctx.now_ms,
        );
    }

    /// A frame of a release or reset animation of the content.
    pub fn on_pose_sample(
        &mut self,
        ctx: &mut Ctx<'_>,
        surface: NodeId,
        progress: f32,
        finished: bool,
    ) {
        let Some(owner) = self.detectors.get_mut(&surface) else {
            return;
        };
        let Some((from, to)) = owner.returning else {
            return;
        };
        owner.pose = if finished {
            to
        } else {
            from.lerp(to, progress)
        };
        if finished {
            owner.returning = None;
        }
        self.write_pose(ctx, surface);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drags_parse_their_contract() {
        let drag = Drag::parse(
            "axis=y\nmin=0\nrubber=0.5\nsnaps=0,100%\nsettle=spring:230:22:0.72\nsettle.1=timing:190:ease-out\nhold=1\nthreshold=120\nvelocity=900\ndrive=|scale|0,50%|1,0.955\ndriveRaw=card|opacity|0,100|1,0\ntext=label|2|0|0,100|0,60",
        );
        let drag = drag.unwrap_or_else(|| panic!("the drag must parse"));
        assert!(!drag.horizontal);
        assert_eq!((drag.min, drag.max, drag.rubber), (Some(0.0), None, 0.5));
        assert_eq!(drag.snaps, vec![Length::Units(0.0), Length::Percent(100.0)]);
        assert!(
            matches!(drag.settle_for(0), Settle::Spring(spring) if spring.stiffness == 230.0 && spring.mass == 0.72)
        );
        assert!(matches!(drag.settle_for(1), Settle::Timing(190, _)));
        assert_eq!(
            (drag.holds.clone(), drag.threshold, drag.velocity),
            (vec![1], 120.0, 900.0)
        );
        assert_eq!(drag.drivers.len(), 3);
        assert!(drag.drivers[1].raw && drag.drivers[1].reference == "card");
        assert_eq!(drag.drivers[2].driven, Driven::Text(2, 0));
        // Past the minimum the target follows at half speed.
        assert_eq!(drag.clamp(-40.0), -20.0);
        assert_eq!(drag.clamp(30.0), 30.0);
        assert!(Drag::parse("  ").is_none());
    }

    #[test]
    fn drivers_interpolate_and_extrapolate() {
        let (input, output) = ([0.0, 100.0, 200.0], [1.0, 0.5, 0.0]);
        assert_eq!(interpolate(&input, &output, 50.0, false), 0.75);
        assert_eq!(interpolate(&input, &output, 150.0, false), 0.25);
        assert_eq!(interpolate(&input, &output, -50.0, false), 1.0);
        assert_eq!(interpolate(&input, &output, 300.0, false), 0.0);
        assert_eq!(interpolate(&input, &output, -100.0, true), 1.5);
        assert_eq!(interpolate(&input, &output, 300.0, true), -0.5);
        // Descending inputs (a sheet dragged upwards).
        assert_eq!(interpolate(&[0.0, -100.0], &[0.0, 1.0], -25.0, false), 0.25);
    }

    #[test]
    fn directions_follow_the_dominant_axis() {
        assert!(direction_ok(2, -10.0, 3.0) && !direction_ok(2, 10.0, 3.0));
        assert!(direction_ok(5, 2.0, 9.0) && !direction_ok(5, 9.0, 2.0));
        assert!(direction_ok(6, 9.0, -2.0) && direction_ok(7, 1.0, -8.0));
        assert!(direction_ok(1, 0.0, 0.0));
    }
}
