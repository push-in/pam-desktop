//! Time-based animation shared by every feature: navigation transitions,
//! modal presentation, press feedback, implicit transitions and programs all
//! run on one frame-clock sampler.
//!
//! The animator only produces numbers. Each running tween belongs to an
//! [`Owner`]; the runtime routes the sampled value to whoever owns it.

use crate::backend::NodeId;

/// Timing curve of a tween, sampled with progress `0..=1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Easing {
    Linear,
    /// CSS cubic-bezier control points.
    Bezier(f32, f32, f32, f32),
    InQuad,
    OutQuad,
    InOutQuad,
    InCubic,
    OutCubic,
    InOutCubic,
    /// `1 - (1 - t)^(2 * factor)`: the platform "decelerate" curve.
    Decelerate(f32),
}

impl Easing {
    pub const EASE: Self = Self::Bezier(0.25, 0.1, 0.25, 1.0);
    pub const EASE_IN: Self = Self::Bezier(0.42, 0.0, 1.0, 1.0);
    pub const EASE_OUT: Self = Self::Bezier(0.0, 0.0, 0.58, 1.0);
    pub const EASE_IN_OUT: Self = Self::Bezier(0.42, 0.0, 0.58, 1.0);
    pub const EASE_OUT_BACK: Self = Self::Bezier(0.34, 1.56, 0.64, 1.0);

    /// Easing tokens of transition specs and motion programs. Unknown tokens
    /// fall back to `ease-in-out`.
    #[must_use]
    pub fn from_token(token: &str) -> Self {
        match token.trim() {
            "linear" => Self::Linear,
            "ease" => Self::EASE,
            "ease-in" => Self::EASE_IN,
            "ease-out" => Self::EASE_OUT,
            "ease-in-out" => Self::EASE_IN_OUT,
            "ease-out-back" => Self::EASE_OUT_BACK,
            "ease-in-quad" => Self::InQuad,
            "ease-out-quad" => Self::OutQuad,
            "ease-in-out-quad" => Self::InOutQuad,
            "ease-in-cubic" => Self::InCubic,
            "ease-out-cubic" => Self::OutCubic,
            "ease-in-out-cubic" => Self::InOutCubic,
            other => other
                .strip_prefix("bezier:")
                .and_then(|values| {
                    let numbers: Vec<f32> = values
                        .split(':')
                        .filter_map(|value| value.parse::<f32>().ok())
                        .collect();
                    (numbers.len() == 4 && numbers.iter().all(|number| number.is_finite())).then(
                        || {
                            Self::Bezier(
                                numbers[0].clamp(0.0, 1.0),
                                numbers[1],
                                numbers[2].clamp(0.0, 1.0),
                                numbers[3],
                            )
                        },
                    )
                })
                .unwrap_or(Self::EASE_IN_OUT),
        }
    }

    /// The `AnimationEasing` property: 1 linear, 2 ease-in, 3 ease-out,
    /// 4 ease-in-out, 5 "spring" (an overshooting ease-out, not a physical spring).
    #[must_use]
    pub fn from_legacy(value: i64) -> Self {
        match value {
            1 => Self::Linear,
            2 => Self::EASE_IN,
            3 => Self::EASE_OUT,
            5 => Self::EASE_OUT_BACK,
            _ => Self::EASE_IN_OUT,
        }
    }

    #[must_use]
    pub fn apply(self, progress: f32) -> f32 {
        let p = progress.clamp(0.0, 1.0);
        match self {
            Self::Linear => p,
            Self::Bezier(x1, y1, x2, y2) => bezier(x1, y1, x2, y2, p),
            Self::InQuad => p * p,
            Self::OutQuad => p * (2.0 - p),
            Self::InOutQuad => {
                if p < 0.5 {
                    2.0 * p * p
                } else {
                    -1.0 + (4.0 - 2.0 * p) * p
                }
            }
            Self::InCubic => p * p * p,
            Self::OutCubic => (p - 1.0).powi(3) + 1.0,
            Self::InOutCubic => {
                if p < 0.5 {
                    4.0 * p * p * p
                } else {
                    (p - 1.0) * (2.0 * p - 2.0).powi(2) + 1.0
                }
            }
            Self::Decelerate(factor) => 1.0 - (1.0 - p).powf(2.0 * factor),
        }
    }
}

/// CSS cubic-bezier: Newton iterations on x(t), then bisection.
fn bezier(x1: f32, y1: f32, x2: f32, y2: f32, p: f32) -> f32 {
    if p <= 0.0 {
        return 0.0;
    }
    if p >= 1.0 {
        return 1.0;
    }
    let curve = |a: f32, b: f32, t: f32| {
        let inverse = 1.0 - t;
        3.0 * inverse * inverse * t * a + 3.0 * inverse * t * t * b + t * t * t
    };
    let slope = |a: f32, b: f32, t: f32| {
        let inverse = 1.0 - t;
        3.0 * inverse * inverse * a + 6.0 * inverse * t * (b - a) + 3.0 * t * t * (1.0 - b)
    };
    let mut t = p;
    let mut solved = false;
    for _ in 0..8 {
        let error = curve(x1, x2, t) - p;
        if error.abs() < 1e-5 {
            solved = true;
            break;
        }
        let derivative = slope(x1, x2, t);
        if derivative.abs() < 1e-6 {
            break;
        }
        t -= error / derivative;
    }
    if !solved {
        let (mut low, mut high) = (0.0_f32, 1.0_f32);
        t = p;
        for _ in 0..24 {
            let x = curve(x1, x2, t);
            if (x - p).abs() < 1e-5 {
                break;
            }
            if x < p {
                low = t;
            } else {
                high = t;
            }
            t = (low + high) / 2.0;
        }
    }
    curve(y1, y2, t.clamp(0.0, 1.0))
}

/// A damped oscillator in closed form with a precomputed settling time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub stiffness: f32,
    pub damping: f32,
    pub mass: f32,
    /// Initial velocity in units per second.
    pub velocity: f32,
}

impl Default for Spring {
    fn default() -> Self {
        Self {
            stiffness: 100.0,
            damping: 10.0,
            mass: 1.0,
            velocity: 0.0,
        }
    }
}

impl Spring {
    /// `spring:<stiffness>:<damping>:<mass>` (missing fields keep defaults).
    #[must_use]
    pub fn from_token(token: &str) -> Option<Self> {
        let values = token.trim().strip_prefix("spring")?;
        let mut spring = Self::default();
        let mut fields = values
            .trim_start_matches(':')
            .split(':')
            .map(|value| value.parse::<f32>().ok());
        if let Some(Some(stiffness)) = fields.next() {
            spring.stiffness = stiffness;
        }
        if let Some(Some(damping)) = fields.next() {
            spring.damping = damping;
        }
        if let Some(Some(mass)) = fields.next() {
            spring.mass = mass;
        }
        (spring.stiffness > 0.0 && spring.damping >= 0.0 && spring.mass > 0.0).then_some(spring)
    }

    /// Displacement from the target `seconds` after the start.
    fn displacement(&self, start: f32, seconds: f32) -> f32 {
        let omega = (self.stiffness / self.mass).sqrt();
        let zeta = self.damping / (2.0 * (self.stiffness * self.mass).sqrt());
        let velocity = self.velocity;
        if zeta < 1.0 {
            let damped = omega * (1.0 - zeta * zeta).sqrt();
            (-zeta * omega * seconds).exp()
                * (start * (damped * seconds).cos()
                    + (velocity + zeta * omega * start) / damped * (damped * seconds).sin())
        } else if (zeta - 1.0).abs() < f32::EPSILON {
            (start + (velocity + omega * start) * seconds) * (-omega * seconds).exp()
        } else {
            let root = (zeta * zeta - 1.0).sqrt();
            let r1 = -omega * (zeta - root);
            let r2 = -omega * (zeta + root);
            let a = (velocity - r2 * start) / (r1 - r2);
            a * (r1 * seconds).exp() + (start - a) * (r2 * seconds).exp()
        }
    }

    /// Milliseconds until the spring rests at `to` (capped at ten seconds).
    #[must_use]
    pub fn duration_ms(&self, from: f32, to: f32) -> u32 {
        let start = from - to;
        let scale = (to - from).abs().max(self.velocity.abs() * 0.05).max(1e-3);
        let (rest_displacement, rest_velocity) = (0.002 * scale, 0.02 * scale);
        if start.abs() < rest_displacement && self.velocity.abs() < rest_velocity {
            return 0;
        }
        const STEP: f32 = 5e-4;
        for millisecond in 1..10_000_u32 {
            let seconds = millisecond as f32 / 1000.0;
            let position = self.displacement(start, seconds);
            let speed = (self.displacement(start, seconds + STEP)
                - self.displacement(start, seconds - STEP))
                / (2.0 * STEP);
            if position.abs() < rest_displacement && speed.abs() < rest_velocity {
                return millisecond;
            }
        }
        10_000
    }

    #[must_use]
    pub fn position(&self, from: f32, to: f32, elapsed_ms: f32) -> f32 {
        to + self.displacement(from - to, elapsed_ms / 1000.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    Timing {
        duration_ms: u32,
        easing: Easing,
    },
    /// `duration_ms` is the precomputed settling time.
    Spring {
        spring: Spring,
        duration_ms: u32,
    },
}

impl Curve {
    #[must_use]
    pub fn timing(duration_ms: u32, easing: Easing) -> Self {
        Self::Timing {
            duration_ms,
            easing,
        }
    }

    #[must_use]
    pub fn spring(spring: Spring, from: f32, to: f32) -> Self {
        Self::Spring {
            spring,
            duration_ms: spring.duration_ms(from, to),
        }
    }

    #[must_use]
    pub fn duration_ms(&self) -> u32 {
        match self {
            Self::Timing { duration_ms, .. } | Self::Spring { duration_ms, .. } => *duration_ms,
        }
    }

    fn sample(&self, from: f32, to: f32, elapsed_ms: f32) -> f32 {
        let duration = self.duration_ms() as f32;
        if elapsed_ms >= duration {
            return to;
        }
        match self {
            Self::Timing { easing, .. } => from + (to - from) * easing.apply(elapsed_ms / duration),
            Self::Spring { spring, .. } => spring.position(from, to, elapsed_ms),
        }
    }
}

/// The animated properties of a node's presentation state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MotionProperty {
    Opacity,
    TranslateX,
    TranslateY,
    ScaleX,
    ScaleY,
    Rotate,
}

/// Who a running tween belongs to. Starting a tween replaces the previous one
/// of the same owner.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Owner {
    /// Route transition progress of a navigation host.
    Navigation(NodeId),
    ModalBackdrop(NodeId),
    ModalContent(NodeId),
    /// Press feedback progress (0 released, 1 pressed).
    Press(NodeId),
    /// Implicit transition of one presentation property.
    Transition(NodeId, MotionProperty),
    /// A preset, keyframe or program track of a node.
    Motion(NodeId, u32),
    /// A host-drawn control (switch position, spinner angle).
    Control(NodeId),
    /// A gliding scroll offset.
    Scroll(NodeId),
    /// Fade-in of an image that arrived from the network.
    Image(NodeId),
    /// The toast sliding in or out.
    Toast,
    /// Shared elements flying between two routes.
    SharedFlight,
    /// A released bottom sheet returning to its snap point.
    SheetDrag(NodeId),
    /// A bar bound to a media player gliding to the new position.
    MediaBar(NodeId),
    /// A drag target settling on a snap point.
    Gesture(NodeId),
    /// The content of a native-transform detector returning to rest.
    GesturePose(NodeId),
}

impl Owner {
    #[must_use]
    pub fn node(&self) -> NodeId {
        match self {
            Self::Navigation(id)
            | Self::ModalBackdrop(id)
            | Self::ModalContent(id)
            | Self::Press(id)
            | Self::Transition(id, _)
            | Self::Motion(id, _)
            | Self::Control(id)
            | Self::Scroll(id)
            | Self::Gesture(id)
            | Self::MediaBar(id)
            | Self::SheetDrag(id)
            | Self::GesturePose(id)
            | Self::Image(id) => *id,
            // Host chrome belongs to no node of the application.
            Self::Toast | Self::SharedFlight => 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Tween {
    owner: Owner,
    from: f32,
    to: f32,
    start_ms: u64,
    curve: Curve,
}

impl Tween {
    fn sample(&self, now_ms: u64) -> (f32, bool) {
        if now_ms <= self.start_ms {
            return (self.from, false);
        }
        let elapsed = (now_ms - self.start_ms) as f32;
        let finished = elapsed >= self.curve.duration_ms() as f32;
        (self.curve.sample(self.from, self.to, elapsed), finished)
    }
}

/// One sampled value of a running tween.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub owner: Owner,
    pub value: f32,
    pub finished: bool,
}

#[derive(Default)]
pub struct Animator {
    tweens: Vec<Tween>,
}

impl Animator {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts (or retargets) the tween of `owner`. The first sample is `from`.
    pub fn start(
        &mut self,
        owner: Owner,
        from: f32,
        to: f32,
        curve: Curve,
        delay_ms: u32,
        now_ms: u64,
    ) {
        self.tweens.retain(|tween| tween.owner != owner);
        self.tweens.push(Tween {
            owner,
            from,
            to,
            start_ms: now_ms + u64::from(delay_ms),
            curve,
        });
    }

    /// Stops the tween of `owner` and returns its current value.
    pub fn cancel(&mut self, owner: Owner, now_ms: u64) -> Option<f32> {
        let position = self.tweens.iter().position(|tween| tween.owner == owner)?;
        Some(self.tweens.swap_remove(position).sample(now_ms).0)
    }

    /// Forgets every tween of a node that left the scene.
    pub fn forget(&mut self, node: NodeId) {
        self.tweens.retain(|tween| tween.owner.node() != node);
    }

    #[must_use]
    pub fn is_running(&self, owner: Owner) -> bool {
        self.tweens.iter().any(|tween| tween.owner == owner)
    }

    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.tweens.is_empty()
    }

    /// Samples every tween; finished ones are reported once and removed.
    pub fn tick(&mut self, now_ms: u64) -> Vec<Sample> {
        let mut samples = Vec::with_capacity(self.tweens.len());
        self.tweens.retain(|tween| {
            let (value, finished) = tween.sample(now_ms);
            samples.push(Sample {
                owner: tween.owner,
                value,
                finished,
            });
            !finished
        });
        samples
    }
}

/// Deadlines the core waits for (long press, delayed press, debounces).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Timer {
    LongPress(NodeId),
    /// A single press held back while a double tap is still possible.
    DeferredPress(NodeId),
    PressIn(NodeId),
    InputDebounce(NodeId),
    /// A windowed list re-checks whether rows near the viewport are unbuilt.
    ListWindow(NodeId),
    /// No scroll offset arrived for a while: the gesture is over.
    ScrollRest(NodeId),
    /// The refresh indicator falls back to its controlled state.
    RefreshFallback(NodeId),
    /// A playing media node reports its position.
    MediaProgress(NodeId),
    /// A hidden or ungranted media node gives its decoder back.
    MediaRelease(NodeId),
    /// A web page took too long to load.
    WebViewTimeout(NodeId),
    /// A production application reloads after a fatal error.
    ErrorRecovery,
    /// The toast on screen has been shown long enough.
    ToastDismiss,
    /// The pointer was held long enough for a long-press detector.
    GestureHold,
    /// The wheel driving a pinch stopped.
    GestureIdle,
}

#[derive(Default)]
pub struct Timers {
    pending: Vec<(u64, Timer)>,
}

impl Timers {
    /// Arms `timer` (replacing an earlier deadline of the same timer).
    pub fn schedule(&mut self, timer: Timer, at_ms: u64) {
        self.cancel(timer);
        self.pending.push((at_ms, timer));
    }

    pub fn cancel(&mut self, timer: Timer) -> bool {
        let before = self.pending.len();
        self.pending.retain(|(_, pending)| *pending != timer);
        self.pending.len() != before
    }

    #[must_use]
    pub fn is_pending(&self, timer: Timer) -> bool {
        self.pending.iter().any(|(_, pending)| *pending == timer)
    }

    /// Timers due at `now_ms`, earliest first.
    pub fn take_due(&mut self, now_ms: u64) -> Vec<Timer> {
        let mut due: Vec<(u64, Timer)> = Vec::new();
        self.pending.retain(|entry| {
            if entry.0 <= now_ms {
                due.push(*entry);
                false
            } else {
                true
            }
        });
        due.sort_by_key(|(at, _)| *at);
        due.into_iter().map(|(_, timer)| timer).collect()
    }

    /// The earliest deadline still pending.
    #[must_use]
    pub fn next_deadline(&self) -> Option<u64> {
        self.pending.iter().map(|(at, _)| *at).min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_tokens_match_their_definitions() {
        assert_eq!(Easing::from_token("linear").apply(0.3), 0.3);
        assert!((Easing::from_token("ease-out-quad").apply(0.5) - 0.75).abs() < 1e-6);
        assert!((Easing::from_token("ease-in-cubic").apply(0.5) - 0.125).abs() < 1e-6);
        assert_eq!(Easing::from_token("nonsense"), Easing::EASE_IN_OUT);
        assert_eq!(
            Easing::from_token("bezier:0.1:0.2:0.3:0.4"),
            Easing::Bezier(0.1, 0.2, 0.3, 0.4)
        );
        // CSS ease-in-out is symmetric around the midpoint.
        assert!((Easing::EASE_IN_OUT.apply(0.5) - 0.5).abs() < 1e-3);
        assert!(Easing::EASE_OUT.apply(0.25) > 0.25 && Easing::EASE_IN.apply(0.25) < 0.25);
        // The overshooting curve goes past the target before settling.
        assert!(Easing::EASE_OUT_BACK.apply(0.7) > 1.0);
        for easing in [
            Easing::EASE,
            Easing::EASE_OUT_BACK,
            Easing::InOutCubic,
            Easing::Decelerate(1.75),
        ] {
            assert_eq!(easing.apply(0.0), 0.0);
            assert!((easing.apply(1.0) - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn spring_settles_on_the_target() {
        let spring = Spring::default();
        let duration = spring.duration_ms(0.0, 100.0);
        assert!(duration > 200 && duration < 3000, "duration {duration}");
        assert!((spring.position(0.0, 100.0, duration as f32) - 100.0).abs() < 0.3);
        // Underdamped: it overshoots on the way.
        let peak = (0..duration)
            .map(|ms| spring.position(0.0, 100.0, ms as f32))
            .fold(0.0_f32, f32::max);
        assert!(peak > 100.0);
        assert_eq!(spring.duration_ms(5.0, 5.0), 0);
        let parsed = Spring::from_token("spring:260:18:1").expect("valid");
        assert_eq!(
            (parsed.stiffness, parsed.damping, parsed.mass),
            (260.0, 18.0, 1.0)
        );
        assert!(Spring::from_token("spring:0:1:1").is_none());
    }

    #[test]
    fn tweens_sample_delay_progress_and_finish_once() {
        let mut animator = Animator::new();
        let owner = Owner::Navigation(7);
        animator.start(
            owner,
            0.0,
            1.0,
            Curve::timing(100, Easing::Linear),
            50,
            1_000,
        );
        assert_eq!(
            animator.tick(1_020),
            vec![Sample {
                owner,
                value: 0.0,
                finished: false
            }]
        );
        let middle = animator.tick(1_100);
        assert!((middle[0].value - 0.5).abs() < 1e-6 && !middle[0].finished);
        assert_eq!(
            animator.tick(1_200),
            vec![Sample {
                owner,
                value: 1.0,
                finished: true
            }]
        );
        assert!(animator.is_idle());
    }

    #[test]
    fn starting_again_replaces_and_cancel_reports_the_current_value() {
        let mut animator = Animator::new();
        let owner = Owner::Press(1);
        animator.start(owner, 0.0, 1.0, Curve::timing(100, Easing::Linear), 0, 0);
        animator.start(owner, 0.5, 0.0, Curve::timing(100, Easing::Linear), 0, 50);
        assert_eq!(animator.tick(50).len(), 1);
        assert!((animator.cancel(owner, 100).unwrap() - 0.25).abs() < 1e-6);
        assert!(animator.cancel(owner, 100).is_none());
        animator.start(
            Owner::Transition(9, MotionProperty::Opacity),
            0.0,
            1.0,
            Curve::timing(10, Easing::Linear),
            0,
            0,
        );
        animator.forget(9);
        assert!(animator.is_idle());
    }

    #[test]
    fn timers_fire_in_order_and_can_be_replaced() {
        let mut timers = Timers::default();
        timers.schedule(Timer::LongPress(1), 500);
        timers.schedule(Timer::DeferredPress(2), 250);
        timers.schedule(Timer::LongPress(1), 400);
        assert_eq!(timers.next_deadline(), Some(250));
        assert!(timers.take_due(100).is_empty());
        assert_eq!(
            timers.take_due(450),
            vec![Timer::DeferredPress(2), Timer::LongPress(1)]
        );
        assert_eq!(timers.next_deadline(), None);
        timers.schedule(Timer::PressIn(3), 10);
        assert!(timers.cancel(Timer::PressIn(3)) && !timers.cancel(Timer::PressIn(3)));
    }
}
