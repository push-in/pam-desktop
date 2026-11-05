//! Declarative motion: implicit transitions, presets, keyframes, motion
//! programs, worklets and shimmer.
//!
//! Everything is sampled by the frame clock and written to the scene as
//! presentation values; nothing per frame ever reaches PHP. Only completion
//! is reported.

use pam_desktop_wire::{NodeKind, PropKey, PropValue};

use crate::animator::{Curve, Easing, MotionProperty, Owner, Spring};
use crate::backend::NodeId;
use crate::context::{Ctx, event_kind};
use crate::geometry::{Rect, Size};
use crate::props::{PropMap, Props};
use crate::scene::{Change, IdMap, LayerOverride, LayerSlot, StyleLayer, motion_default};

const SLOT_PRESET: u32 = 0;
const SLOT_KEYFRAMES: u32 = 1;
const SLOT_PROGRAM: u32 = 2;
const SLOT_WORKLET: u32 = 3;
const SLOT_SHIMMER: u32 = 4;
const SLOTS: usize = 4;

const PROPERTIES: [MotionProperty; 6] = [
    MotionProperty::Opacity,
    MotionProperty::TranslateX,
    MotionProperty::TranslateY,
    MotionProperty::ScaleX,
    MotionProperty::ScaleY,
    MotionProperty::Rotate,
];

/// One sampled frame of an animation: the six presentation properties plus
/// the corner radius programs may animate.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    pub values: [Option<f32>; 6],
    pub border_radius: Option<f32>,
}

fn index(property: MotionProperty) -> usize {
    PROPERTIES
        .iter()
        .position(|candidate| *candidate == property)
        .unwrap_or(0)
}

// ---- implicit transitions ---------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
struct Rule {
    property: String,
    duration_ms: u32,
    delay_ms: u32,
    timing: String,
}

/// `@property a,b` / `@duration …` / `@timing …` / `@delay …` lines; entry
/// `i` of the property list uses entry `i % len` of the other lists.
fn parse_transition_spec(spec: &str) -> Vec<Rule> {
    let mut properties = vec!["all"];
    let mut durations = vec!["0"];
    let mut timings = vec!["ease"];
    let mut delays = vec!["0"];
    for line in spec.lines() {
        let Some((list, values)) = line.strip_prefix('@').and_then(|line| line.split_once(' '))
        else {
            continue;
        };
        let values: Vec<&str> = values
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect();
        if values.is_empty() {
            continue;
        }
        match list {
            "property" => properties = values,
            "duration" => durations = values,
            "timing" => timings = values,
            "delay" => delays = values,
            _ => {}
        }
    }
    let milliseconds = |value: &str| value.parse::<i64>().unwrap_or(0).clamp(0, 60_000) as u32;
    let mut rules: Vec<Rule> = Vec::new();
    for (position, property) in properties.iter().enumerate() {
        if *property == "none" {
            continue;
        }
        let rule = Rule {
            property: (*property).to_owned(),
            duration_ms: milliseconds(durations[position % durations.len()]),
            delay_ms: milliseconds(delays[position % delays.len()]),
            timing: timings[position % timings.len()].to_owned(),
        };
        // A repeated property name: the last one wins.
        rules.retain(|existing| existing.property != rule.property);
        rules.push(rule);
    }
    rules
}

fn rule_for(rules: &[Rule], property: MotionProperty) -> Option<&Rule> {
    let names: &[&str] = match property {
        MotionProperty::Opacity => &["opacity", "all"],
        MotionProperty::TranslateX | MotionProperty::TranslateY => {
            &["translate", "transform", "all"]
        }
        MotionProperty::ScaleX | MotionProperty::ScaleY => &["scale", "transform", "all"],
        MotionProperty::Rotate => &["rotate", "transform", "all"],
    };
    names
        .iter()
        .find_map(|name| rules.iter().find(|rule| rule.property == *name))
}

// ---- keyframes ----------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
struct Keyframe {
    offset: f32,
    values: [Option<f32>; 6],
    easing: Easing,
}

/// JSON array of 2..64 frames with strictly increasing offsets. Unspecified
/// values carry the previous frame's (leading frames use the first one given).
fn parse_keyframes(json: &[u8], parent_width: f32) -> Option<Vec<Keyframe>> {
    let serde_json::Value::Array(entries) =
        serde_json::from_slice::<serde_json::Value>(json).ok()?
    else {
        return None;
    };
    if !(2..=64).contains(&entries.len()) {
        return None;
    }
    let mut frames = Vec::with_capacity(entries.len());
    let mut previous_offset = -1.0_f32;
    for entry in &entries {
        let number = |key: &str| {
            entry
                .get(key)
                .and_then(serde_json::Value::as_f64)
                .map(|value| value as f32)
        };
        let offset = number("offset")?;
        if !(0.0..=1.0).contains(&offset) || offset <= previous_offset {
            return None;
        }
        previous_offset = offset;
        let translate_x = number("translationXPercent")
            .map(|percent| percent / 100.0 * parent_width)
            .or_else(|| number("translationX"));
        frames.push(Keyframe {
            offset,
            values: [
                number("opacity"),
                translate_x,
                number("translationY"),
                number("scaleX"),
                number("scaleY"),
                number("rotation"),
            ],
            easing: entry
                .get("easing")
                .and_then(serde_json::Value::as_str)
                .map_or(Easing::Linear, Easing::from_token),
        });
    }
    for property in 0..6 {
        let Some(first) = frames.iter().find_map(|frame| frame.values[property]) else {
            continue;
        };
        let mut carried = first;
        for frame in &mut frames {
            match frame.values[property] {
                Some(value) => carried = value,
                None => frame.values[property] = Some(carried),
            }
        }
    }
    Some(frames)
}

fn sample_keyframes(frames: &[Keyframe], timeline: Easing, fraction: f32) -> Frame {
    let eased = timeline.apply(fraction);
    let mut right = frames.len() - 1;
    for (position, frame) in frames.iter().enumerate() {
        if frame.offset >= eased {
            right = position;
            break;
        }
    }
    let left = right.saturating_sub(1);
    let (from, to) = (&frames[left], &frames[right]);
    let span = to.offset - from.offset;
    let local = if left == right || span <= 0.0 {
        1.0
    } else {
        from.easing
            .apply(((eased - from.offset) / span).clamp(0.0, 1.0))
    };
    let mut frame = Frame::default();
    for property in 0..6 {
        if let (Some(start), Some(end)) = (from.values[property], to.values[property]) {
            frame.values[property] = Some(start + (end - start) * local);
        }
    }
    frame
}

// ---- motion programs ------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum SegmentKind {
    Set,
    Timing { easing: Easing },
    Spring { spring: Spring },
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Segment {
    start: f32,
    length: f32,
    from: f32,
    to: f32,
    kind: SegmentKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Track {
    Property(MotionProperty),
    /// `scale` writes both axes.
    Scale,
    BorderRadius,
}

impl Track {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "opacity" => Self::Property(MotionProperty::Opacity),
            "translateX" => Self::Property(MotionProperty::TranslateX),
            "translateY" => Self::Property(MotionProperty::TranslateY),
            "scaleX" => Self::Property(MotionProperty::ScaleX),
            "scaleY" => Self::Property(MotionProperty::ScaleY),
            "rotate" => Self::Property(MotionProperty::Rotate),
            "scale" => Self::Scale,
            "borderRadius" => Self::BorderRadius,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Program {
    id: i64,
    /// -1 is infinite.
    iterations: i64,
    tracks: Vec<(Track, f32, Vec<Segment>)>,
    duration: f32,
}

#[derive(Clone, Debug, PartialEq)]
enum Step {
    Set(StepValue),
    Wait(f32),
    Timing {
        value: StepValue,
        duration: f32,
        easing: Easing,
        delay: f32,
    },
    Spring {
        value: StepValue,
        spring: Spring,
        delay: f32,
    },
}

/// A number in the property's unit, or a percentage resolved at build time.
#[derive(Clone, Copy, Debug, PartialEq)]
enum StepValue {
    Absolute(f32),
    Percent(f32),
}

impl StepValue {
    fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        match text.strip_suffix('%') {
            Some(percent) => percent
                .parse::<f32>()
                .ok()
                .filter(|value| value.is_finite())
                .map(Self::Percent),
            None => text
                .parse::<f32>()
                .ok()
                .filter(|value| value.is_finite())
                .map(Self::Absolute),
        }
    }

    fn resolve(self, track: Track, size: Size) -> f32 {
        match self {
            Self::Absolute(value) => value,
            Self::Percent(percent) => match track {
                Track::Property(MotionProperty::TranslateX) => percent / 100.0 * size.width,
                Track::Property(MotionProperty::TranslateY) => percent / 100.0 * size.height,
                Track::BorderRadius => percent / 100.0 * size.width.min(size.height),
                _ => percent / 100.0,
            },
        }
    }
}

fn parse_step(text: &str) -> Option<Step> {
    let (name, arguments) = text.strip_suffix(')')?.split_once('(')?;
    let arguments: Vec<&str> = if arguments.is_empty() {
        Vec::new()
    } else {
        arguments.split(',').collect()
    };
    let number =
        |position: usize, default: f32| match arguments.get(position).map(|value| value.trim()) {
            Some("") | None => Some(default),
            Some(value) => value.parse::<f32>().ok().filter(|value| value.is_finite()),
        };
    let milliseconds = |position: usize, default: f32| {
        number(position, default).map(|value| value.clamp(0.0, 60_000.0))
    };
    match name {
        "set" => Some(Step::Set(StepValue::parse(arguments.first()?)?)),
        "wait" => Some(Step::Wait(milliseconds(0, 0.0)?)),
        "timing" => Some(Step::Timing {
            value: StepValue::parse(arguments.first()?)?,
            duration: milliseconds(1, 300.0)?,
            easing: arguments
                .get(2)
                .map(|token| token.trim())
                .filter(|token| !token.is_empty())
                .map_or(Easing::EASE_IN_OUT, Easing::from_token),
            delay: milliseconds(3, 0.0)?,
        }),
        "spring" => {
            let spring = Spring {
                stiffness: number(1, 100.0)?,
                damping: number(2, 10.0)?,
                mass: number(3, 1.0)?,
                velocity: number(4, 0.0)?,
            };
            if spring.stiffness <= 0.0 || spring.damping < 0.0 || spring.mass <= 0.0 {
                return None;
            }
            Some(Step::Spring {
                value: StepValue::parse(arguments.first()?)?,
                spring,
                delay: milliseconds(5, 0.0)?,
            })
        }
        _ => None,
    }
}

/// What a motion program declares: its id, its iterations and its tracks.
type ParsedProgram = (i64, i64, Vec<(u32, Track, Vec<Step>)>);

/// `pam-motion 1 id=<int> iterations=<int>` followed by
/// `<phase> <property> <step> <step> …` lines. Any malformed line rejects
/// the whole program.
fn parse_program(text: &str) -> Option<ParsedProgram> {
    let mut lines = text.lines();
    let header = lines.next()?;
    let mut tokens = header.split(' ');
    if tokens.next()? != "pam-motion" || tokens.next()? != "1" {
        return None;
    }
    let (mut id, mut iterations) = (0_i64, 1_i64);
    for attribute in tokens {
        match attribute.split_once('=')? {
            ("id", value) => id = value.parse().ok()?,
            ("iterations", value) => iterations = value.parse::<i64>().ok()?.clamp(-1, 10_000),
            _ => {}
        }
    }
    let mut tracks: Vec<(u32, Track, Vec<Step>)> = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let mut tokens = line.split(' ');
        let phase = tokens
            .next()?
            .parse::<u32>()
            .ok()
            .filter(|phase| *phase <= 63)?;
        let track = Track::parse(tokens.next()?)?;
        let steps: Vec<Step> = tokens.map(parse_step).collect::<Option<_>>()?;
        if steps.is_empty() {
            return None;
        }
        match tracks
            .iter_mut()
            .find(|(existing_phase, existing_track, _)| {
                *existing_phase == phase && *existing_track == track
            }) {
            Some((_, _, existing)) => existing.extend(steps),
            None => tracks.push((phase, track, steps)),
        }
        if tracks.iter().any(|(_, _, steps)| steps.len() > 64) {
            return None;
        }
    }
    Some((id, iterations, tracks))
}

impl Program {
    /// Lays the steps out on a timeline. Phases run one after another; the
    /// tracks of a phase run in parallel. `initial` gives the value each
    /// track shows before its first segment.
    fn build(
        id: i64,
        iterations: i64,
        mut parsed: Vec<(u32, Track, Vec<Step>)>,
        size: Size,
        initial: impl Fn(Track) -> f32,
    ) -> Self {
        parsed.sort_by_key(|(phase, _, _)| *phase);
        let mut tracks: Vec<(Track, f32, Vec<Segment>)> = Vec::new();
        let mut phase_start = 0.0_f32;
        let mut position = 0;
        while position < parsed.len() {
            let phase = parsed[position].0;
            let mut phase_end = phase_start;
            while position < parsed.len() && parsed[position].0 == phase {
                let (_, track, steps) = &parsed[position];
                let slot = match tracks.iter().position(|(existing, _, _)| existing == track) {
                    Some(slot) => slot,
                    None => {
                        tracks.push((*track, initial(*track), Vec::new()));
                        tracks.len() - 1
                    }
                };
                let mut value = tracks[slot]
                    .2
                    .last()
                    .map_or(tracks[slot].1, |segment| segment.to);
                let mut cursor = phase_start;
                for step in steps {
                    match step {
                        Step::Set(target) => {
                            let to = target.resolve(*track, size);
                            tracks[slot].2.push(Segment {
                                start: cursor,
                                length: 0.0,
                                from: value,
                                to,
                                kind: SegmentKind::Set,
                            });
                            value = to;
                        }
                        Step::Wait(milliseconds) => cursor += milliseconds,
                        Step::Timing {
                            value: target,
                            duration,
                            easing,
                            delay,
                        } => {
                            cursor += delay;
                            let to = target.resolve(*track, size);
                            tracks[slot].2.push(Segment {
                                start: cursor,
                                length: *duration,
                                from: value,
                                to,
                                kind: SegmentKind::Timing { easing: *easing },
                            });
                            cursor += duration;
                            value = to;
                        }
                        Step::Spring {
                            value: target,
                            spring,
                            delay,
                        } => {
                            cursor += delay;
                            let to = target.resolve(*track, size);
                            let length = spring.duration_ms(value, to) as f32;
                            tracks[slot].2.push(Segment {
                                start: cursor,
                                length,
                                from: value,
                                to,
                                kind: SegmentKind::Spring { spring: *spring },
                            });
                            cursor += length;
                            value = to;
                        }
                    }
                }
                phase_end = phase_end.max(cursor);
                position += 1;
            }
            phase_start = phase_end;
        }
        Self {
            id,
            iterations,
            tracks,
            duration: phase_start,
        }
    }

    fn sample(&self, milliseconds: f32) -> Frame {
        let mut frame = Frame::default();
        for (track, initial, segments) in &self.tracks {
            let mut value = *initial;
            for segment in segments {
                if milliseconds < segment.start {
                    break;
                }
                let elapsed = (milliseconds - segment.start).clamp(0.0, segment.length);
                value = match segment.kind {
                    SegmentKind::Set => segment.to,
                    SegmentKind::Timing { easing } => {
                        if segment.length <= 0.0 {
                            segment.to
                        } else {
                            segment.from
                                + (segment.to - segment.from)
                                    * easing.apply(elapsed / segment.length)
                        }
                    }
                    SegmentKind::Spring { spring } => {
                        if elapsed >= segment.length {
                            segment.to
                        } else {
                            spring.position(segment.from, segment.to, elapsed)
                        }
                    }
                };
            }
            match track {
                Track::Property(property) => frame.values[index(*property)] = Some(value),
                Track::Scale => {
                    frame.values[index(MotionProperty::ScaleX)] = Some(value);
                    frame.values[index(MotionProperty::ScaleY)] = Some(value);
                }
                Track::BorderRadius => frame.border_radius = Some(value.max(0.0)),
            }
        }
        frame
    }
}

// ---- worklets -------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Instruction {
    Input,
    Constant(f64),
    Add(f64),
    Subtract(f64),
    Multiply(f64),
    Divide(f64),
    Clamp(f64, f64),
    Interpolate(f64, f64, f64, f64),
}

/// `"PNW1"`, `u16` count, then `{u8 opcode, u8 operand count, f64…}` each.
fn parse_worklet(bytes: &[u8]) -> Option<Vec<Instruction>> {
    let body = bytes.strip_prefix(b"PNW1")?;
    let count = usize::from(u16::from_le_bytes(body.get(0..2)?.try_into().ok()?));
    if !(1..=256).contains(&count) {
        return None;
    }
    let mut offset = 2;
    let mut instructions = Vec::with_capacity(count);
    for _ in 0..count {
        let opcode = *body.get(offset)?;
        let operand_count = usize::from(*body.get(offset + 1)?);
        offset += 2;
        let mut operands = [0.0_f64; 4];
        if operand_count > 4 {
            return None;
        }
        for operand in operands.iter_mut().take(operand_count) {
            *operand = f64::from_le_bytes(body.get(offset..offset + 8)?.try_into().ok()?);
            if !operand.is_finite() {
                return None;
            }
            offset += 8;
        }
        let [a, b, c, d] = operands;
        instructions.push(match (opcode, operand_count) {
            (1, 0) => Instruction::Input,
            (2, 1) => Instruction::Constant(a),
            (3, 1) => Instruction::Add(a),
            (4, 1) => Instruction::Subtract(a),
            (5, 1) => Instruction::Multiply(a),
            (6, 1) if a != 0.0 => Instruction::Divide(a),
            (7, 2) if a <= b => Instruction::Clamp(a, b),
            (8, 4) if a != b => Instruction::Interpolate(a, b, c, d),
            _ => return None,
        });
    }
    (offset == body.len()).then_some(instructions)
}

fn run_worklet(instructions: &[Instruction], input: f64) -> Option<f64> {
    let mut value = 0.0_f64;
    for instruction in instructions {
        value = match *instruction {
            Instruction::Input => input,
            Instruction::Constant(a) => a,
            Instruction::Add(a) => value + a,
            Instruction::Subtract(a) => value - a,
            Instruction::Multiply(a) => value * a,
            Instruction::Divide(a) => value / a,
            Instruction::Clamp(a, b) => value.max(a).min(b),
            Instruction::Interpolate(a, b, c, d) => c + (value - a) / (b - a) * (d - c),
        };
        if !value.is_finite() {
            return None;
        }
    }
    Some(value)
}

// ---- runners ----------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Clip {
    /// `(property, from, to)` tracks sharing one easing.
    Preset {
        tracks: Vec<(MotionProperty, f32, f32)>,
        easing: Easing,
    },
    Keyframes {
        frames: Vec<Keyframe>,
        timeline: Easing,
    },
    Program(Program),
    Worklet {
        instructions: Vec<Instruction>,
        target: i64,
    },
}

#[derive(Clone, Debug)]
struct Runner {
    clip: Clip,
    /// Milliseconds of one iteration.
    duration: f32,
    /// `None` repeats forever.
    iterations: Option<u32>,
    /// Odd iterations play backwards.
    alternate: bool,
    /// Keep the last frame once finished; otherwise the node's own values return.
    holds: bool,
    /// Iterations already completed (infinite runners restart their clock).
    completed: u32,
    /// Payload of `AnimationComplete`; `None` sends no event.
    completion: Option<Vec<u8>>,
    /// Which properties this runner wrote (cleared when it does not hold).
    wrote: [bool; 6],
    wrote_radius: bool,
}

impl Runner {
    fn sample(&self, milliseconds: f32) -> Frame {
        let milliseconds = milliseconds.clamp(0.0, self.duration);
        match &self.clip {
            Clip::Preset { tracks, easing } => {
                let progress = if self.duration > 0.0 {
                    easing.apply(milliseconds / self.duration)
                } else {
                    1.0
                };
                let mut frame = Frame::default();
                for (property, from, to) in tracks {
                    frame.values[index(*property)] = Some(from + (to - from) * progress);
                }
                frame
            }
            Clip::Keyframes { frames, timeline } => sample_keyframes(
                frames,
                *timeline,
                if self.duration > 0.0 {
                    milliseconds / self.duration
                } else {
                    1.0
                },
            ),
            Clip::Program(program) => program.sample(milliseconds),
            Clip::Worklet {
                instructions,
                target,
            } => {
                let mut frame = Frame::default();
                let Some(value) = run_worklet(instructions, f64::from(milliseconds)) else {
                    return frame;
                };
                let value = value as f32;
                match target {
                    1 => frame.values[index(MotionProperty::Opacity)] = Some(value.clamp(0.0, 1.0)),
                    2 => {
                        frame.values[index(MotionProperty::TranslateX)] =
                            Some(value.clamp(-1e6, 1e6))
                    }
                    3 => {
                        frame.values[index(MotionProperty::TranslateY)] =
                            Some(value.clamp(-1e6, 1e6))
                    }
                    4 => {
                        let scale = value.clamp(-100.0, 100.0);
                        frame.values[index(MotionProperty::ScaleX)] = Some(scale);
                        frame.values[index(MotionProperty::ScaleY)] = Some(scale);
                    }
                    5 => {
                        frame.values[index(MotionProperty::Rotate)] =
                            Some(value.clamp(-36_000.0, 36_000.0))
                    }
                    _ => {}
                }
                frame
            }
        }
    }
}

#[derive(Debug)]
struct Shimmer {
    holder: NodeId,
    strip: NodeId,
    duration: u32,
}

#[derive(Debug, Default)]
struct NodeMotion {
    runners: [Option<Runner>; SLOTS],
    program_id: Option<i64>,
    restart_key: Option<i64>,
    shimmer: Option<Shimmer>,
    /// What the commit being applied asked for.
    preset: bool,
    keyframes: bool,
    program: bool,
    worklet: bool,
    shimmers: bool,
    restart: bool,
}

#[derive(Default)]
pub struct Motion {
    nodes: IdMap<NodeMotion>,
    /// Nodes mounted by the commit being applied: they never transition.
    fresh: Vec<NodeId>,
    dirty: bool,
}

fn has_motion_props(props: Props<'_>) -> bool {
    [
        PropKey::AnimationKind,
        PropKey::AnimationKeyframes,
        PropKey::AnimationProgram,
        PropKey::WorkletProgram,
        PropKey::ShimmerEnabled,
        PropKey::AnimationRestartKey,
    ]
    .iter()
    .any(|key| props.has(*key))
}

impl Motion {
    pub fn on_change(&mut self, ctx: &mut Ctx<'_>, change: Change) {
        match change {
            Change::Created(id, _) => {
                self.fresh.push(id);
                self.dirty = true;
                let Some(node) = ctx.scene.node(id) else {
                    return;
                };
                let props = node.props();
                if node.is_synthetic() || !has_motion_props(props) {
                    return;
                }
                let state = NodeMotion {
                    preset: props.has(PropKey::AnimationKind),
                    keyframes: props.has(PropKey::AnimationKeyframes),
                    program: props.has(PropKey::AnimationProgram),
                    worklet: props.has(PropKey::WorkletProgram),
                    shimmers: props.has(PropKey::ShimmerEnabled),
                    restart_key: props.integer(PropKey::AnimationRestartKey),
                    ..NodeMotion::default()
                };
                self.nodes.insert(id, state);
            }
            Change::Removed(id, _) => {
                self.nodes.remove(&id);
            }
            Change::Updated(id, key) => {
                let flag: fn(&mut NodeMotion) = match key {
                    PropKey::AnimationKind => |state| state.preset = true,
                    PropKey::AnimationKeyframes
                    | PropKey::AnimationIterations
                    | PropKey::AnimationDelayMs
                    | PropKey::AnimationFillMode
                    | PropKey::AnimationPlayState
                    | PropKey::AnimationAutoReverse => |state| state.keyframes = true,
                    PropKey::AnimationProgram => |state| state.program = true,
                    PropKey::AnimationRestartKey => |state| state.restart = true,
                    PropKey::WorkletProgram
                    | PropKey::WorkletTarget
                    | PropKey::WorkletDurationMs
                    | PropKey::WorkletIterations => |state| state.worklet = true,
                    PropKey::ShimmerEnabled
                    | PropKey::ShimmerGradientColor
                    | PropKey::ShimmerDurationMs => |state| state.shimmers = true,
                    _ => return,
                };
                flag(self.nodes.entry(id).or_default());
                self.dirty = true;
            }
            Change::Motion {
                id,
                property,
                previous,
            } => self.transition(ctx, id, property, previous),
            Change::Frame(id) => {
                if let Some(state) = self.nodes.get_mut(&id)
                    && state.shimmer.is_some()
                {
                    state.shimmers = true;
                    self.dirty = true;
                }
            }
            Change::Children(_) => {}
        }
    }

    /// A presentation property of a node changed: animate towards it when a
    /// transition applies, starting from what is on screen right now.
    fn transition(
        &mut self,
        ctx: &mut Ctx<'_>,
        id: NodeId,
        property: MotionProperty,
        previous: Option<f32>,
    ) {
        let owner = Owner::Transition(id, property);
        let running = ctx.animator.cancel(owner, ctx.now_ms);
        let Some(node) = ctx.scene.node(id) else {
            return;
        };
        let props = node.props();
        let target = props
            .number(crate::scene::motion_prop_key(property))
            .unwrap_or_else(|| motion_default(property));
        let rules = props
            .text(PropKey::TransitionSpec)
            .map(parse_transition_spec);
        let curve_and_delay = match rules.as_deref().and_then(|rules| rule_for(rules, property)) {
            Some(rule) => match Spring::from_token(&rule.timing) {
                Some(spring) => Some((None, Some(spring), rule.delay_ms)),
                None if rule.duration_ms == 0 && rule.delay_ms == 0 => None,
                None => Some((
                    Some(Curve::timing(
                        rule.duration_ms,
                        Easing::from_token(&rule.timing),
                    )),
                    None,
                    rule.delay_ms,
                )),
            },
            None if props.flag_or(PropKey::AnimateChanges, false) => {
                let duration = props
                    .integer_or(PropKey::AnimationDurationMs, 180)
                    .clamp(1, 10_000) as u32;
                Some((
                    Some(Curve::timing(
                        duration,
                        Easing::from_legacy(props.integer_or(PropKey::AnimationEasing, 4)),
                    )),
                    None,
                    0,
                ))
            }
            None => None,
        };
        let mut values = ctx.scene.motion(id);
        let from = running
            .or(values.get(property))
            .or(previous)
            .unwrap_or_else(|| motion_default(property));
        let mounting = self.fresh.contains(&id);
        match curve_and_delay {
            Some((curve, spring, delay)) if !mounting && !ctx.reduced_motion && from != target => {
                let curve = curve
                    .unwrap_or_else(|| Curve::spring(spring.unwrap_or_default(), from, target));
                values.set(property, Some(from));
                ctx.scene.set_motion(id, values);
                ctx.animator
                    .start(owner, from, target, curve, delay, ctx.now_ms);
            }
            _ => {
                // No transition: the node's own value shows at once.
                if running.is_some() {
                    values.set(property, None);
                    ctx.scene.set_motion(id, values);
                }
            }
        }
    }

    pub fn after_commit(&mut self, ctx: &mut Ctx<'_>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let ids: Vec<NodeId> = self
            .nodes
            .iter()
            .filter(|(_, state)| {
                state.preset
                    || state.keyframes
                    || state.program
                    || state.worklet
                    || state.shimmers
                    || state.restart
            })
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            let Some(state) = self.nodes.get_mut(&id) else {
                continue;
            };
            let (preset, keyframes, program, worklet, shimmers, restart) = (
                std::mem::take(&mut state.preset),
                std::mem::take(&mut state.keyframes),
                std::mem::take(&mut state.program),
                std::mem::take(&mut state.worklet),
                std::mem::take(&mut state.shimmers),
                std::mem::take(&mut state.restart),
            );
            // The first restart key seen never triggers; later changes replay.
            let mut replay = false;
            if restart {
                let key = ctx
                    .scene
                    .node(id)
                    .and_then(|node| node.props().integer(PropKey::AnimationRestartKey));
                replay = state.restart_key.is_some() && key != state.restart_key;
                state.restart_key = key;
            }
            if preset {
                self.start_preset(ctx, id);
            }
            if keyframes || replay {
                self.start_keyframes(ctx, id);
            }
            if program || replay {
                self.start_program(ctx, id, replay);
            }
            if worklet {
                self.start_worklet(ctx, id);
            }
            if shimmers {
                self.sync_shimmer(ctx, id);
            }
        }
        self.fresh.clear();
    }

    fn stop(&mut self, ctx: &mut Ctx<'_>, id: NodeId, slot: u32, keep_values: bool) {
        ctx.animator.cancel(Owner::Motion(id, slot), ctx.now_ms);
        let Some(runner) = self
            .nodes
            .get_mut(&id)
            .and_then(|state| state.runners[slot as usize].take())
        else {
            return;
        };
        if !keep_values {
            Self::clear_written(ctx, id, &runner);
        }
    }

    fn clear_written(ctx: &mut Ctx<'_>, id: NodeId, runner: &Runner) {
        let mut values = ctx.scene.motion(id);
        for (position, property) in PROPERTIES.iter().enumerate() {
            if runner.wrote[position] {
                values.set(*property, None);
            }
        }
        ctx.scene.set_motion(id, values);
        if runner.wrote_radius {
            ctx.scene.set_style_overlay(id, StyleLayer::Presenter, None);
        }
    }

    fn apply(ctx: &mut Ctx<'_>, id: NodeId, runner: &mut Runner, frame: Frame) {
        let mut values = ctx.scene.motion(id);
        for (position, property) in PROPERTIES.iter().enumerate() {
            if let Some(value) = frame.values[position] {
                let value = if *property == MotionProperty::Opacity {
                    value.clamp(0.0, 1.0)
                } else {
                    value
                };
                values.set(*property, Some(value));
                runner.wrote[position] = true;
            }
        }
        ctx.scene.set_motion(id, values);
        if let Some(radius) = frame.border_radius {
            // An animated corner radius also clips the node.
            let mut overlay = PropMap::new();
            overlay.insert(PropKey::BorderRadius, PropValue::Float(f64::from(radius)));
            overlay.insert(PropKey::Overflow, PropValue::Integer(2));
            ctx.scene
                .set_style_overlay(id, StyleLayer::Presenter, Some(overlay));
            runner.wrote_radius = true;
        }
    }

    /// Starts a runner: the first frame applies at once, the clock follows.
    fn start(&mut self, ctx: &mut Ctx<'_>, id: NodeId, slot: u32, mut runner: Runner, delay: u32) {
        self.stop(ctx, id, slot, true);
        let instant = ctx.reduced_motion || runner.duration <= 0.0 || runner.iterations == Some(0);
        if instant {
            let last = runner.sample(runner.duration);
            Self::apply(ctx, id, &mut runner, last);
            self.finish(ctx, id, runner);
            return;
        }
        let first = runner.sample(0.0);
        if delay == 0 {
            Self::apply(ctx, id, &mut runner, first);
        }
        let total = match runner.iterations {
            Some(iterations) => runner.duration * iterations as f32,
            None => runner.duration,
        };
        ctx.animator.start(
            Owner::Motion(id, slot),
            0.0,
            total,
            Curve::timing(total.round().max(1.0) as u32, Easing::Linear),
            delay,
            ctx.now_ms,
        );
        self.nodes.entry(id).or_default().runners[slot as usize] = Some(runner);
    }

    fn finish(&mut self, ctx: &mut Ctx<'_>, id: NodeId, runner: Runner) {
        if !runner.holds {
            Self::clear_written(ctx, id, &runner);
        }
        if let Some(payload) = runner.completion {
            ctx.emit_if(
                id,
                PropKey::OnAnimationComplete,
                event_kind::ANIMATION_COMPLETE,
                payload,
            );
        }
    }

    /// Frame-clock sample of a runner (`elapsed` in milliseconds) or a shimmer.
    pub fn on_sample(&mut self, ctx: &mut Ctx<'_>, owner: Owner, value: f32, finished: bool) {
        match owner {
            Owner::Transition(id, property) => {
                let mut values = ctx.scene.motion(id);
                values.set(property, if finished { None } else { Some(value) });
                ctx.scene.set_motion(id, values);
            }
            Owner::Motion(id, SLOT_SHIMMER) => self.sample_shimmer(ctx, id, value, finished),
            Owner::Motion(id, slot) => {
                let Some(mut runner) = self
                    .nodes
                    .get_mut(&id)
                    .and_then(|state| state.runners.get_mut(slot as usize)?.take())
                else {
                    return;
                };
                let iteration = if runner.duration > 0.0 {
                    (value / runner.duration).floor() as u32
                } else {
                    0
                };
                let done = finished && runner.iterations.is_some();
                let local = if done {
                    runner.duration
                } else {
                    value - iteration as f32 * runner.duration
                };
                let played = runner.completed
                    + if done {
                        runner.iterations.unwrap_or(1).saturating_sub(1)
                    } else {
                        iteration
                    };
                let backwards = runner.alternate && played % 2 == 1;
                let frame = runner.sample(if backwards {
                    runner.duration - local
                } else {
                    local
                });
                Self::apply(ctx, id, &mut runner, frame);
                if done {
                    self.finish(ctx, id, runner);
                } else if finished {
                    // An endless runner: the next iteration restarts the clock.
                    runner.completed += 1;
                    let duration = runner.duration.round().max(1.0) as u32;
                    ctx.animator.start(
                        owner,
                        0.0,
                        runner.duration,
                        Curve::timing(duration, Easing::Linear),
                        0,
                        ctx.now_ms,
                    );
                    self.nodes.entry(id).or_default().runners[slot as usize] = Some(runner);
                } else {
                    self.nodes.entry(id).or_default().runners[slot as usize] = Some(runner);
                }
            }
            _ => {}
        }
    }

    // ---- presets ---------------------------------------------------------

    fn start_preset(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        self.stop(ctx, id, SLOT_PRESET, false);
        let Some(node) = ctx.scene.node(id) else {
            return;
        };
        let props = node.props();
        let kind = props.integer_or(PropKey::AnimationKind, 1);
        let opacity = props.number_or(PropKey::Opacity, 1.0);
        let scale_x = props.number_or(PropKey::ScaleX, 1.0);
        let scale_y = props.number_or(PropKey::ScaleY, 1.0);
        let translate_x = props.number_or(PropKey::TranslationX, 0.0);
        let translate_y = props.number_or(PropKey::TranslationY, 0.0);
        let pulse = kind == 2;
        let duration = props
            .integer_or(PropKey::AnimationDurationMs, 240)
            .clamp(100, if pulse { 60_000 } else { 2000 }) as f32;
        let easing = Easing::from_legacy(props.integer_or(PropKey::AnimationEasing, 3));
        let fade = (MotionProperty::Opacity, 0.0, opacity);
        let tracks = match kind {
            2 => vec![(MotionProperty::Opacity, opacity, 0.55 * opacity)],
            3 => vec![fade],
            4 => vec![
                fade,
                (MotionProperty::ScaleX, scale_x * 0.94, scale_x),
                (MotionProperty::ScaleY, scale_y * 0.94, scale_y),
            ],
            5 => vec![
                fade,
                (MotionProperty::TranslateY, translate_y + 18.0, translate_y),
            ],
            6 => vec![
                fade,
                (MotionProperty::TranslateY, translate_y - 18.0, translate_y),
            ],
            7 => vec![
                (MotionProperty::ScaleX, scale_x * 0.9, scale_x),
                (MotionProperty::ScaleY, scale_y * 0.9, scale_y),
            ],
            8 => vec![(MotionProperty::TranslateX, translate_x - 8.0, translate_x)],
            _ => return,
        };
        let runner = Runner {
            clip: Clip::Preset { tracks, easing },
            duration,
            iterations: if pulse { None } else { Some(1) },
            alternate: pulse,
            // Presets end on the node's own values.
            holds: false,
            completed: 0,
            completion: None,
            wrote: [false; 6],
            wrote_radius: false,
        };
        // Reduced motion never pulses.
        if pulse && ctx.reduced_motion {
            return;
        }
        self.start(ctx, id, SLOT_PRESET, runner, 0);
    }

    // ---- keyframes -------------------------------------------------------

    fn start_keyframes(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(node) = ctx.scene.node(id) else {
            return;
        };
        let props = node.props();
        let state = props.integer_or(PropKey::AnimationPlayState, 1);
        let parent_width = ctx
            .scene
            .node(node.parent)
            .map_or(ctx.viewport.width, |parent| parent.frame.width);
        let frames = props
            .bytes(PropKey::AnimationKeyframes)
            .and_then(|json| parse_keyframes(json, parent_width));
        let Some(frames) = frames else {
            return self.stop(ctx, id, SLOT_KEYFRAMES, false);
        };
        match state {
            // Paused freezes on the current frame; Stopped returns to the node's own values.
            2 => {
                ctx.animator
                    .cancel(Owner::Motion(id, SLOT_KEYFRAMES), ctx.now_ms);
                return;
            }
            3 => return self.stop(ctx, id, SLOT_KEYFRAMES, false),
            _ => {}
        }
        let iterations = props
            .integer_or(PropKey::AnimationIterations, 1)
            .clamp(0, 10_000) as u32;
        let fill = props.integer_or(PropKey::AnimationFillMode, 2);
        let runner = Runner {
            clip: Clip::Keyframes {
                frames,
                timeline: Easing::from_legacy(props.integer_or(PropKey::AnimationEasing, 4)),
            },
            duration: props
                .integer_or(PropKey::AnimationDurationMs, 300)
                .clamp(1, 60_000) as f32,
            iterations: (iterations > 0).then_some(iterations),
            alternate: props.flag_or(PropKey::AnimationAutoReverse, false),
            holds: matches!(fill, 2 | 4),
            completed: 0,
            completion: Some(Vec::new()),
            wrote: [false; 6],
            wrote_radius: false,
        };
        let delay = props
            .integer_or(PropKey::AnimationDelayMs, 0)
            .clamp(0, 60_000) as u32;
        // Keyframes and worklets exclude each other.
        self.stop(ctx, id, SLOT_WORKLET, false);
        self.start(ctx, id, SLOT_KEYFRAMES, runner, delay);
    }

    // ---- programs --------------------------------------------------------

    fn start_program(&mut self, ctx: &mut Ctx<'_>, id: NodeId, replay: bool) {
        let Some(node) = ctx.scene.node(id) else {
            return;
        };
        let parsed = node
            .props()
            .text(PropKey::AnimationProgram)
            .and_then(parse_program);
        let size = node.frame.size();
        let Some((program_id, iterations, tracks)) = parsed else {
            // Removed or malformed: the runner stops, values stay where they are.
            self.stop(ctx, id, SLOT_PROGRAM, true);
            if let Some(state) = self.nodes.get_mut(&id) {
                state.program_id = None;
            }
            return;
        };
        let state = self.nodes.entry(id).or_default();
        // The same program applied again is not a restart.
        if state.program_id == Some(program_id) && !replay {
            return;
        }
        state.program_id = Some(program_id);
        let radius = node.decoration.radii.top_left;
        let scene = &*ctx.scene;
        let program = Program::build(program_id, iterations, tracks, size, |track| match track {
            Track::Property(property) => scene.displayed(id, property),
            Track::Scale => scene.displayed(id, MotionProperty::ScaleX),
            Track::BorderRadius => radius,
        });
        let runner = Runner {
            duration: program.duration,
            iterations: u32::try_from(program.iterations).ok(),
            clip: Clip::Program(program),
            alternate: false,
            holds: true,
            completed: 0,
            completion: Some(program_id.to_string().into_bytes()),
            wrote: [false; 6],
            wrote_radius: false,
        };
        self.start(ctx, id, SLOT_PROGRAM, runner, 0);
    }

    // ---- worklets --------------------------------------------------------

    fn start_worklet(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(node) = ctx.scene.node(id) else {
            return;
        };
        let props = node.props();
        let Some(instructions) = props.bytes(PropKey::WorkletProgram).and_then(parse_worklet)
        else {
            return self.stop(ctx, id, SLOT_WORKLET, false);
        };
        let iterations = props
            .integer_or(PropKey::WorkletIterations, 1)
            .clamp(0, 10_000) as u32;
        let runner = Runner {
            clip: Clip::Worklet {
                instructions,
                target: props.integer_or(PropKey::WorkletTarget, 1),
            },
            duration: props
                .integer_or(PropKey::WorkletDurationMs, 300)
                .clamp(1, 60_000) as f32,
            iterations: (iterations > 0).then_some(iterations),
            alternate: false,
            holds: true,
            completed: 0,
            completion: Some(Vec::new()),
            wrote: [false; 6],
            wrote_radius: false,
        };
        self.stop(ctx, id, SLOT_KEYFRAMES, false);
        self.start(ctx, id, SLOT_WORKLET, runner, 0);
    }

    // ---- shimmer ---------------------------------------------------------

    /// The sweep is two host nodes below the node's children: a holder that
    /// clips to the node's corners and a gradient strip that crosses it.
    fn sync_shimmer(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(node) = ctx.scene.node(id) else {
            return;
        };
        let props = node.props();
        let enabled = props.flag_or(PropKey::ShimmerEnabled, false)
            && !ctx.reduced_motion
            && !node.frame.is_empty();
        let color = props.integer_or(PropKey::ShimmerGradientColor, 0x59FF_FFFF);
        let duration = props.integer_or(PropKey::ShimmerDurationMs, 1200).max(1) as u32;
        let frame = node.frame;
        let radii = node.decoration.radii;
        let state = self.nodes.entry(id).or_default();
        if !enabled {
            if let Some(shimmer) = state.shimmer.take() {
                ctx.animator
                    .cancel(Owner::Motion(id, SLOT_SHIMMER), ctx.now_ms);
                ctx.scene.remove_synthetic(shimmer.holder);
            }
            return;
        }
        let strip_width = (1.25 * frame.width).max(1.0);
        let alpha = (color >> 24) & 0xFF;
        let tint =
            |factor: f64| (((alpha as f64 * factor).round() as i64) << 24) | (color & 0x00FF_FFFF);
        let gradient = format!(
            "[{{\"t\":1,\"m\":1,\"a\":90,\"s\":[[{},0,1],[{},0.35,1],[{},0.5,1],[{},0.65,1],[{},1,1]]}}]",
            tint(0.0),
            tint(0.24),
            tint(1.0),
            tint(0.24),
            tint(0.0),
        );
        let strip_frame = Rect::new(frame.x, frame.y, strip_width, frame.height);
        match &mut state.shimmer {
            Some(shimmer) => {
                shimmer.duration = duration;
                ctx.scene.set_synthetic_frame(shimmer.holder, frame);
                ctx.scene.set_synthetic_frame(shimmer.strip, strip_frame);
                ctx.scene.set_prop(
                    shimmer.strip,
                    PropKey::BackgroundGradient,
                    Some(PropValue::String(gradient)),
                );
            }
            None => {
                let mut holder = PropMap::new();
                holder.insert(PropKey::ZIndex, PropValue::Integer(-1));
                holder.insert(PropKey::PointerEvents, PropValue::Integer(2));
                holder.insert(PropKey::Overflow, PropValue::Integer(2));
                let holder = ctx
                    .scene
                    .add_synthetic(id, NodeKind::View, holder, frame, 0, None);
                let mut strip = PropMap::new();
                strip.insert(PropKey::BackgroundGradient, PropValue::String(gradient));
                let strip =
                    ctx.scene
                        .add_synthetic(holder, NodeKind::View, strip, strip_frame, 0, None);
                state.shimmer = Some(Shimmer {
                    holder,
                    strip,
                    duration,
                });
                ctx.animator.start(
                    Owner::Motion(id, SLOT_SHIMMER),
                    0.0,
                    1.0,
                    Curve::timing(duration, Easing::Linear),
                    0,
                    ctx.now_ms,
                );
            }
        }
        if let Some(shimmer) = self.nodes.get(&id).and_then(|state| state.shimmer.as_ref()) {
            for (key, radius) in [
                (PropKey::BorderTopLeftRadius, radii.top_left),
                (PropKey::BorderTopRightRadius, radii.top_right),
                (PropKey::BorderBottomRightRadius, radii.bottom_right),
                (PropKey::BorderBottomLeftRadius, radii.bottom_left),
            ] {
                ctx.scene.set_prop(
                    shimmer.holder,
                    key,
                    (radius > 0.0).then_some(PropValue::Float(f64::from(radius))),
                );
            }
        }
    }

    fn sample_shimmer(&mut self, ctx: &mut Ctx<'_>, id: NodeId, progress: f32, finished: bool) {
        let Some(shimmer) = self.nodes.get(&id).and_then(|state| state.shimmer.as_ref()) else {
            return;
        };
        let Some(width) = ctx.scene.node(id).map(|node| node.frame.width) else {
            return;
        };
        let strip_width = (1.25 * width).max(1.0);
        // The strip's left edge travels from fully off the left to fully off the right.
        let offset = -strip_width + (width + strip_width) * progress;
        ctx.scene.set_layer_override(
            shimmer.strip,
            LayerSlot::Presenter,
            LayerOverride {
                transform: crate::geometry::Affine::translation(offset, 0.0),
                ..LayerOverride::NONE
            },
        );
        if finished {
            ctx.animator.start(
                Owner::Motion(id, SLOT_SHIMMER),
                0.0,
                1.0,
                Curve::timing(shimmer.duration, Easing::Linear),
                0,
                ctx.now_ms,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_spec_cycles_lists_and_resolves_rules() {
        let rules = parse_transition_spec(
            "@property opacity,transform\n@duration 120,280\n@timing ease-out,spring:260:18:1\n@delay 40",
        );
        assert_eq!(rules.len(), 2);
        let opacity = rule_for(&rules, MotionProperty::Opacity).expect("opacity");
        assert_eq!(
            (
                opacity.duration_ms,
                opacity.delay_ms,
                opacity.timing.as_str()
            ),
            (120, 40, "ease-out")
        );
        let scale = rule_for(&rules, MotionProperty::ScaleX).expect("transform covers scale");
        assert_eq!(
            (scale.duration_ms, scale.timing.as_str()),
            (280, "spring:260:18:1")
        );
        assert!(
            rule_for(
                &parse_transition_spec("@property opacity"),
                MotionProperty::Rotate
            )
            .is_none()
        );
        // Defaults: every property, instantly.
        assert_eq!(
            rule_for(&parse_transition_spec(""), MotionProperty::Rotate)
                .map(|rule| rule.duration_ms),
            Some(0)
        );
        assert!(parse_transition_spec("@property none").is_empty());
    }

    #[test]
    fn keyframes_carry_values_forward_and_ease_per_segment() {
        let json = br#"[{"offset":0.0,"opacity":0.0,"translationY":20.0},{"offset":0.5,"opacity":1.0},{"offset":1.0,"translationY":0.0}]"#;
        let frames = parse_keyframes(json, 400.0).expect("valid");
        assert_eq!(
            frames[1].values[index(MotionProperty::TranslateY)],
            Some(20.0)
        );
        assert_eq!(frames[2].values[index(MotionProperty::Opacity)], Some(1.0));
        let middle = sample_keyframes(&frames, Easing::Linear, 0.25);
        assert_eq!(middle.values[index(MotionProperty::Opacity)], Some(0.5));
        assert_eq!(middle.values[index(MotionProperty::TranslateY)], Some(20.0));
        let late = sample_keyframes(&frames, Easing::Linear, 0.75);
        assert_eq!(late.values[index(MotionProperty::TranslateY)], Some(10.0));
        // A property no frame mentions is never animated.
        assert_eq!(late.values[index(MotionProperty::Rotate)], None);
        assert!(parse_keyframes(br#"[{"offset":0.5},{"offset":0.5}]"#, 0.0).is_none());
        assert!(parse_keyframes(br#"[{"offset":0.0}]"#, 0.0).is_none());
        let percent = parse_keyframes(
            br#"[{"offset":0,"translationXPercent":50},{"offset":1,"translationXPercent":0}]"#,
            400.0,
        )
        .unwrap();
        assert_eq!(
            percent[0].values[index(MotionProperty::TranslateX)],
            Some(200.0)
        );
    }

    #[test]
    fn programs_run_phases_in_sequence_and_tracks_in_parallel() {
        let text = "pam-motion 1 id=42 iterations=-1\n0 opacity set(0) timing(1,200,linear,0)\n0 translateY timing(50%,100,linear,0)\n1 scale timing(2,100,linear,50)";
        let (id, iterations, tracks) = parse_program(text).expect("valid");
        assert_eq!((id, iterations), (42, -1));
        let program =
            Program::build(
                id,
                iterations,
                tracks,
                Size::new(100.0, 80.0),
                |track| match track {
                    Track::Scale => 1.0,
                    _ => 0.0,
                },
            );
        // Phase 0 lasts 200 ms (its longest track); phase 1 adds a 50 ms delay and 100 ms.
        assert_eq!(program.duration, 350.0);
        let early = program.sample(100.0);
        assert_eq!(early.values[index(MotionProperty::Opacity)], Some(0.5));
        assert_eq!(early.values[index(MotionProperty::TranslateY)], Some(40.0));
        assert_eq!(early.values[index(MotionProperty::ScaleX)], Some(1.0));
        let late = program.sample(300.0);
        assert_eq!(late.values[index(MotionProperty::ScaleX)], Some(1.5));
        assert_eq!(late.values[index(MotionProperty::ScaleY)], Some(1.5));
        assert_eq!(
            program.sample(350.0).values[index(MotionProperty::Opacity)],
            Some(1.0)
        );
        assert!(parse_program("pam-motion 2 id=1").is_none());
        assert!(parse_program("pam-motion 1 id=1\n0 opacity jump(1)").is_none());
        assert!(parse_program("pam-motion 1 id=1\n0 unknown set(1)").is_none());
    }

    #[test]
    fn worklets_evaluate_bytecode_and_reject_bad_programs() {
        let mut bytes = b"PNW1".to_vec();
        bytes.extend_from_slice(&3_u16.to_le_bytes());
        bytes.extend_from_slice(&[1, 0]);
        bytes.extend_from_slice(&[8, 4]);
        for operand in [0.0_f64, 300.0, 0.0, 1.0] {
            bytes.extend_from_slice(&operand.to_le_bytes());
        }
        bytes.extend_from_slice(&[7, 2]);
        for operand in [0.0_f64, 1.0] {
            bytes.extend_from_slice(&operand.to_le_bytes());
        }
        let instructions = parse_worklet(&bytes).expect("valid");
        assert_eq!(run_worklet(&instructions, 150.0), Some(0.5));
        assert_eq!(run_worklet(&instructions, 900.0), Some(1.0));
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(parse_worklet(&trailing).is_none());
        assert!(parse_worklet(b"PNW1\x01\x00\x06\x01\0\0\0\0\0\0\0\0").is_none());
        assert!(parse_worklet(b"nope").is_none());
    }
}
