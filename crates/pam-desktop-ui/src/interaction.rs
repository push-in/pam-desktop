//! Pointer and keyboard interaction, implemented once for every backend.
//!
//! Backends forward raw pointer and key events of a window; this module hit
//! tests the scene and runs the press state machine of the protocol (press,
//! long press, double tap, retention, hover and state styles) so behaviour is
//! identical on GTK, AppKit and Win32.

use pam_desktop_wire::{NodeKind, PropKey, PropValue};

use crate::animator::{Curve, Easing, Owner, Timer};
use crate::backend::{KeyEvent, NodeId, PointerButton, PointerEvent, PointerPhase, ViewClass};
use crate::color::Color;
use crate::context::{event_kind, Ctx, HostCommand};
use crate::geometry::{Affine, Insets, Point, Rect};
use crate::props::PropMap;
use crate::scene::{Action, Change, IdMap, LayerOverride, LayerSlot, StyleLayer};
use crate::wiremap::WireMap;

const PRESS_IN_MS: u32 = 70;
const PRESS_OUT_MS: u32 = 110;
const DEFAULT_PRESS_OPACITY: f32 = 0.72;
const DEFAULT_LONG_PRESS_MS: i64 = 500;
const DEFAULT_DOUBLE_TAP_MS: i64 = 250;
/// Movement that cancels a pending long press.
const LONG_PRESS_SLOP: f32 = 10.0;
/// Two taps farther apart than this are not a double tap.
const DOUBLE_TAP_SLOP: f32 = 100.0;

const STATE_PRESSED: u8 = 1;
const STATE_FOCUSED: u8 = 2;
const STATE_HOVERED: u8 = 3;

#[derive(Clone, Copy, Debug)]
struct Press {
    node: NodeId,
    started: Point,
    /// The pointer is within the retention bounds.
    inside: bool,
    in_sent: bool,
    long_pressed: bool,
    long_press_cancelled: bool,
    /// Whoever handles the long press (the node or an enclosing pressable).
    long_press_target: Option<NodeId>,
}

#[derive(Clone, Copy, Debug)]
struct Tap {
    node: NodeId,
    at_ms: u64,
    position: Point,
}

/// State styles compiled from `:pressed`, `:focus` and `:hover` rules.
#[derive(Clone, Debug, Default, PartialEq)]
struct StateStyles {
    states: Vec<(u8, PropMap)>,
}

impl StateStyles {
    /// `{"<state>": {"<propKey>": number, …}, …}`
    fn parse(json: &str) -> Self {
        let Ok(serde_json::Value::Object(states)) = serde_json::from_str::<serde_json::Value>(json) else {
            return Self::default();
        };
        let mut parsed = Vec::new();
        for (state, rules) in states {
            let (Ok(state), Some(rules)) = (state.parse::<u8>(), rules.as_object()) else { continue };
            if !(STATE_PRESSED..=STATE_HOVERED).contains(&state) {
                continue;
            }
            let mut props = PropMap::new();
            for (key, value) in rules {
                let key = match key.parse::<u16>() {
                    Ok(38) => PropKey::Opacity,
                    Ok(74) => PropKey::ScaleX,
                    Ok(75) => PropKey::ScaleY,
                    Ok(72) => PropKey::TranslationX,
                    Ok(73) => PropKey::TranslationY,
                    Ok(10) => PropKey::BackgroundColor,
                    Ok(11) => PropKey::TextColor,
                    Ok(37) => PropKey::BorderColor,
                    Ok(56) => PropKey::Elevation,
                    _ => continue,
                };
                let colour = matches!(key, PropKey::BackgroundColor | PropKey::TextColor | PropKey::BorderColor);
                let value = match (colour, value.as_i64(), value.as_f64()) {
                    (true, Some(integer), _) => PropValue::Integer(integer),
                    (true, None, Some(number)) => PropValue::Integer(number as i64),
                    (false, _, Some(number)) if number.is_finite() => PropValue::Float(number),
                    _ => continue,
                };
                props.insert(key, value);
            }
            if !props.is_empty() {
                parsed.push((state, props));
            }
        }
        Self { states: parsed }
    }

    fn get(&self, state: u8) -> Option<&PropMap> {
        self.states.iter().find(|(id, _)| *id == state).map(|(_, props)| props)
    }
}

#[derive(Default)]
pub struct Interaction {
    press: Option<Press>,
    last_tap: Option<Tap>,
    /// Nodes under the pointer, innermost first.
    hovered: Vec<NodeId>,
    focused: Option<NodeId>,
    pointer: Option<Point>,
    /// A pointer move waiting to be reported (one per drain).
    pending_move: Option<(NodeId, Vec<u8>)>,
    styles: IdMap<StateStyles>,
    /// Nodes that want `ClickOutside`.
    outside_observers: Vec<NodeId>,
    /// Nodes currently carrying a style overlay from this module.
    styled: Vec<NodeId>,
    /// Animated `PressBackgroundColor` fills.
    press_fills: IdMap<Color>,
    pub dark: bool,
}

fn pointer_payload(ctx: &Ctx<'_>, node: NodeId, position: Point) -> Vec<u8> {
    let origin = ctx.scene.window_origin(node);
    WireMap::new()
        .with("x", position.x - origin.x)
        .with("y", position.y - origin.y)
        .with("pageX", position.x)
        .with("pageY", position.y)
        .with("timestamp", i64::try_from(ctx.now_ms).unwrap_or(0))
        .with("pointerId", 0_i64)
        .with("pointerCount", 1_i64)
        .encode()
}

/// Whether presses on this node show the default dim/scale feedback.
fn shows_feedback(ctx: &Ctx<'_>, id: NodeId) -> bool {
    ctx.scene.node(id).is_some_and(|node| {
        let pressable_kind = matches!(node.kind, NodeKind::Button | NodeKind::Pressable) && node.action.is_none();
        let authored = node.handles(PropKey::PressOpacity) || node.handles(PropKey::PressScale) || node.handles(PropKey::PressBackgroundColor);
        // A surface that also takes a double tap would flash on each tap.
        (pressable_kind || authored) && !node.handles(PropKey::OnDoubleTap)
    })
}

impl Interaction {
    #[must_use]
    pub fn new(dark: bool) -> Self {
        Self { dark, ..Self::default() }
    }

    pub fn on_change(&mut self, ctx: &mut Ctx<'_>, change: Change) {
        match change {
            Change::Created(id, kind) => {
                self.read_styles(ctx, id);
                self.watch_outside(ctx, id);
                if kind == NodeKind::Button {
                    self.restyle(ctx, id);
                }
            }
            Change::Updated(id, PropKey::NativeStateStyles) => {
                self.read_styles(ctx, id);
                self.restyle(ctx, id);
            }
            Change::Updated(id, PropKey::OnClickOutside) => self.watch_outside(ctx, id),
            Change::Updated(id, PropKey::BackgroundColor | PropKey::BackgroundGradient | PropKey::TextColor | PropKey::BorderWidth) => {
                if ctx.scene.node(id).is_some_and(|node| node.kind == NodeKind::Button) {
                    self.restyle(ctx, id);
                }
            }
            Change::Removed(id, _) => {
                self.styles.remove(&id);
                self.outside_observers.retain(|observer| *observer != id);
                self.hovered.retain(|hovered| *hovered != id);
                self.styled.retain(|styled| *styled != id);
                self.press_fills.remove(&id);
                if self.press.is_some_and(|press| press.node == id) {
                    self.press = None;
                    ctx.timers.cancel(Timer::LongPress(id));
                    ctx.timers.cancel(Timer::PressIn(id));
                }
                ctx.timers.cancel(Timer::DeferredPress(id));
                if self.focused == Some(id) {
                    self.focused = None;
                }
                if self.last_tap.is_some_and(|tap| tap.node == id) {
                    self.last_tap = None;
                }
            }
            _ => {}
        }
    }

    fn read_styles(&mut self, ctx: &Ctx<'_>, id: NodeId) {
        match ctx.scene.node(id).and_then(|node| node.props().text(PropKey::NativeStateStyles)) {
            Some(json) if !json.is_empty() => {
                self.styles.insert(id, StateStyles::parse(json));
            }
            _ => {
                self.styles.remove(&id);
            }
        }
    }

    fn watch_outside(&mut self, ctx: &Ctx<'_>, id: NodeId) {
        let watches = ctx.scene.node(id).is_some_and(|node| node.handles(PropKey::OnClickOutside));
        let known = self.outside_observers.contains(&id);
        if watches && !known {
            self.outside_observers.push(id);
        } else if !watches && known {
            self.outside_observers.retain(|observer| *observer != id);
        }
    }

    /// The appearance of a button the application did not style: a neutral
    /// control surface, like a native push button.
    fn default_button(&self, ctx: &Ctx<'_>, id: NodeId, hovered: bool, pressed: bool) -> Option<PropMap> {
        let node = ctx.scene.node(id)?;
        if node.kind != NodeKind::Button || node.is_synthetic() {
            return None;
        }
        let styled = node.handles(PropKey::BackgroundColor)
            || node.handles(PropKey::BackgroundGradient)
            || node.handles(PropKey::BorderWidth)
            || node.handles(PropKey::TextColor);
        if styled {
            return None;
        }
        let (rest, hover, down, text) = if self.dark {
            (0xFF3A_3A3C_u32, 0xFF48_484A_u32, 0xFF54_5456_u32, 0xFFFF_FFFF_u32)
        } else {
            (0xFFE9_E9EB_u32, 0xFFDE_DEE2_u32, 0xFFD1_D1D6_u32, 0xFF1C_1C1E_u32)
        };
        let background = if pressed {
            down
        } else if hovered {
            hover
        } else {
            rest
        };
        let mut props = PropMap::new();
        props.insert(PropKey::BackgroundColor, PropValue::Integer(i64::from(background)));
        props.insert(PropKey::TextColor, PropValue::Integer(i64::from(text)));
        if !node.handles(PropKey::BorderRadius) {
            props.insert(PropKey::BorderRadius, PropValue::Integer(6));
        }
        Some(props)
    }

    /// Recomputes the style overlay of a node from its interaction state.
    fn restyle(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let pressed = self.press.is_some_and(|press| press.node == id && press.inside);
        let hovered = self.hovered.contains(&id);
        let focused = self.focused == Some(id);
        let mut overlay = self.default_button(ctx, id, hovered, pressed).unwrap_or_default();
        if let Some(styles) = self.styles.get(&id) {
            // Later states win: hover, then focus, then pressed.
            for (active, state) in [(hovered, STATE_HOVERED), (focused, STATE_FOCUSED), (pressed, STATE_PRESSED)] {
                if !active {
                    continue;
                }
                if let Some(props) = styles.get(state) {
                    for (key, value) in props {
                        // The pressed dim and scale run as an animated layer
                        // (PHP mirrors them in PressOpacity / PressScale).
                        if state == STATE_PRESSED && matches!(key, PropKey::Opacity | PropKey::ScaleX | PropKey::ScaleY) {
                            continue;
                        }
                        overlay.insert(*key, value.clone());
                    }
                }
            }
        }
        if let Some(fill) = self.press_fills.get(&id) {
            overlay.insert(PropKey::BackgroundColor, PropValue::Integer(i64::from(fill.to_argb())));
        }
        let has_overlay = !overlay.is_empty();
        ctx.scene.set_style_overlay(id, StyleLayer::Interaction, has_overlay.then_some(overlay));
        let tracked = self.styled.contains(&id);
        if has_overlay && !tracked {
            self.styled.push(id);
        } else if !has_overlay && tracked {
            self.styled.retain(|styled| *styled != id);
        }
    }

    /// Appearance changed (dark mode): default buttons follow.
    pub fn refresh_defaults(&mut self, ctx: &mut Ctx<'_>) {
        let buttons: Vec<NodeId> = ctx.scene.nodes().filter(|node| node.kind == NodeKind::Button).map(|node| node.id).collect();
        for id in buttons {
            self.restyle(ctx, id);
        }
    }

    // ---- pointer -----------------------------------------------------------

    /// Nearest enabled pressable at or above `id`. Native text fields keep
    /// their own pointer handling.
    fn press_target(ctx: &Ctx<'_>, id: NodeId) -> Option<NodeId> {
        let mut current = id;
        loop {
            let node = ctx.scene.node(current)?;
            if matches!(node.class, ViewClass::Input | ViewClass::TextArea) {
                return None;
            }
            if node.is_pressable() {
                return node.props().flag_or(PropKey::Enabled, true).then_some(current);
            }
            if node.parent == 0 || node.kind == NodeKind::Modal {
                return None;
            }
            current = node.parent;
        }
    }

    /// Nearest node at or above `id` carrying `marker`.
    fn nearest_with(ctx: &Ctx<'_>, id: NodeId, marker: PropKey) -> Option<NodeId> {
        let mut current = id;
        loop {
            let node = ctx.scene.node(current)?;
            if node.handles(marker) {
                return Some(current);
            }
            if node.parent == 0 {
                return None;
            }
            current = node.parent;
        }
    }

    /// The node's box on screen grown by its hit slop and press retention.
    fn retention_bounds(ctx: &Ctx<'_>, id: NodeId) -> Rect {
        let Some(node) = ctx.scene.node(id) else { return Rect::ZERO };
        let origin = ctx.scene.window_origin(id);
        let props = node.props();
        let slop = props.number_or(PropKey::HitSlop, 0.0).max(0.0);
        let edge = |slop_key, retention_key, default: f32| {
            props.number(slop_key).map_or(slop, |value| value.max(0.0)) + props.number_or(retention_key, default).max(0.0)
        };
        Rect::new(origin.x, origin.y, node.frame.width, node.frame.height).outset_by(Insets::new(
            edge(PropKey::HitSlopLeft, PropKey::PressRetentionLeft, 20.0),
            edge(PropKey::HitSlopTop, PropKey::PressRetentionTop, 20.0),
            edge(PropKey::HitSlopRight, PropKey::PressRetentionRight, 20.0),
            edge(PropKey::HitSlopBottom, PropKey::PressRetentionBottom, 30.0),
        ))
    }

    fn set_pressed_visual(&mut self, ctx: &mut Ctx<'_>, id: NodeId, pressed: bool) {
        self.restyle(ctx, id);
        if !shows_feedback(ctx, id) {
            return;
        }
        let owner = Owner::Press(id);
        let from = ctx.animator.cancel(owner, ctx.now_ms).unwrap_or(if pressed { 0.0 } else { 1.0 });
        let to = if pressed { 1.0 } else { 0.0 };
        if ctx.reduced_motion {
            self.on_press_sample(ctx, id, to);
        } else {
            let duration = if pressed { PRESS_IN_MS } else { PRESS_OUT_MS };
            ctx.animator.start(owner, from, to, Curve::timing(duration, Easing::EASE_OUT), 0, ctx.now_ms);
        }
    }

    /// Frame-clock sample of the press feedback (0 released, 1 pressed).
    pub fn on_press_sample(&mut self, ctx: &mut Ctx<'_>, id: NodeId, amount: f32) {
        let Some(node) = ctx.scene.node(id) else { return };
        let props = node.props();
        let unstyled_button = self.default_button(ctx, id, false, false).is_some();
        // A default button already changes its surface colour when pressed.
        let target_opacity = if unstyled_button && !node.handles(PropKey::PressOpacity) {
            1.0
        } else {
            props.number_or(PropKey::PressOpacity, DEFAULT_PRESS_OPACITY).clamp(0.0, 1.0)
        };
        let target_scale = props.number_or(PropKey::PressScale, 1.0).clamp(0.01, 4.0);
        let size = node.frame.size();
        let mut layer = LayerOverride::NONE;
        layer.opacity = 1.0 + (target_opacity - 1.0) * amount;
        if target_scale != 1.0 {
            let scale = 1.0 + (target_scale - 1.0) * amount;
            layer.transform = Affine::scaling(scale, scale).around(Point::new(size.width / 2.0, size.height / 2.0));
        }
        // PressBackgroundColor fades the fill towards the pressed colour.
        let press_fill = props.color(PropKey::PressBackgroundColor).map(|pressed_color| {
            let base = props.color(PropKey::BackgroundColor).unwrap_or(Color::TRANSPARENT);
            base.lerp(pressed_color, amount)
        });
        ctx.scene.set_layer_override(id, LayerSlot::Interaction, layer);
        if let Some(fill) = press_fill {
            if amount > 0.0 {
                self.press_fills.insert(id, fill);
            } else {
                self.press_fills.remove(&id);
            }
            self.restyle(ctx, id);
        }
    }

    fn update_hover(&mut self, ctx: &mut Ctx<'_>, position: Option<Point>) {
        let mut chain = Vec::new();
        if let Some(hit) = position.and_then(|position| ctx.scene.hit_test(position)) {
            let mut current = hit;
            while let Some(node) = ctx.scene.node(current) {
                chain.push(current);
                if node.parent == 0 {
                    break;
                }
                current = node.parent;
            }
        }
        if chain == self.hovered {
            return;
        }
        let previous = std::mem::replace(&mut self.hovered, chain);
        let position = position.unwrap_or(Point::ZERO);
        for id in previous.iter().filter(|id| !self.hovered.contains(id)).copied().collect::<Vec<_>>() {
            if self.styles.contains_key(&id) || ctx.scene.node(id).is_some_and(|node| node.kind == NodeKind::Button) {
                self.restyle(ctx, id);
            }
            let payload = pointer_payload(ctx, id, position);
            ctx.emit_if(id, PropKey::OnHoverOut, event_kind::HOVER_OUT, payload);
        }
        for id in self.hovered.iter().filter(|id| !previous.contains(id)).copied().collect::<Vec<_>>() {
            if self.styles.contains_key(&id) || ctx.scene.node(id).is_some_and(|node| node.kind == NodeKind::Button) {
                self.restyle(ctx, id);
            }
            let payload = pointer_payload(ctx, id, position);
            ctx.emit_if(id, PropKey::OnHoverIn, event_kind::HOVER_IN, payload);
        }
    }

    /// Returns the core action a press on chrome triggered, if any.
    pub fn on_pointer(&mut self, ctx: &mut Ctx<'_>, pointer: PointerEvent) -> Option<Action> {
        match pointer.phase {
            PointerPhase::Down => {
                self.pointer = Some(pointer.position);
                if pointer.button == PointerButton::Primary {
                    self.pointer_down(ctx, pointer.position);
                }
                None
            }
            PointerPhase::Move => {
                self.pointer = Some(pointer.position);
                self.update_hover(ctx, Some(pointer.position));
                self.pointer_move(ctx, pointer.position);
                None
            }
            PointerPhase::Up => {
                self.pointer = Some(pointer.position);
                if pointer.button == PointerButton::Primary {
                    self.pointer_up(ctx, pointer.position)
                } else {
                    None
                }
            }
            PointerPhase::Cancel => {
                self.cancel_press(ctx);
                None
            }
            PointerPhase::Leave => {
                self.pointer = None;
                self.update_hover(ctx, None);
                None
            }
        }
    }

    fn pointer_down(&mut self, ctx: &mut Ctx<'_>, position: Point) {
        self.cancel_press(ctx);
        let Some(hit) = ctx.scene.hit_test(position) else { return };
        if let Some(target) = Self::nearest_with(ctx, hit, PropKey::OnTouchStart) {
            let payload = pointer_payload(ctx, target, position);
            ctx.emit(target, event_kind::TOUCH_START, payload);
        }
        let Some(node) = Self::press_target(ctx, hit) else {
            // Nothing pressable: a drag region moves the window.
            let drags = Self::nearest_with(ctx, hit, PropKey::WindowDrag)
                .and_then(|region| ctx.scene.node(region))
                .is_some_and(|region| region.props().flag_or(PropKey::WindowDrag, false));
            if drags && !ctx.scene.node(hit).is_some_and(|node| matches!(node.class, ViewClass::Input | ViewClass::TextArea)) {
                ctx.commands.push(HostCommand::BeginWindowDrag);
            }
            return;
        };
        let Some(scene_node) = ctx.scene.node(node) else { return };
        let props = scene_node.props();
        let delay_in = props.integer_or(PropKey::PressDelayInMs, 0).clamp(0, 60_000) as u64;
        let delay_long = props.integer_or(PropKey::PressDelayLongMs, DEFAULT_LONG_PRESS_MS).clamp(1, 60_000) as u64;
        // A pressable without a long press lends it to the nearest one that has it.
        let long_press_target = Self::nearest_with(ctx, node, PropKey::OnLongPress)
            .filter(|target| ctx.scene.node(*target).is_some_and(crate::scene::SceneNode::is_pressable));
        self.press = Some(Press {
            node,
            started: position,
            inside: true,
            in_sent: false,
            long_pressed: false,
            long_press_cancelled: false,
            long_press_target,
        });
        self.set_pressed_visual(ctx, node, true);
        if delay_in == 0 {
            self.send_press_in(ctx, position);
        } else {
            ctx.timers.schedule(Timer::PressIn(node), ctx.now_ms + delay_in);
        }
        if long_press_target.is_some() {
            ctx.timers.schedule(Timer::LongPress(node), ctx.now_ms + delay_in + delay_long);
        }
    }

    fn send_press_in(&mut self, ctx: &mut Ctx<'_>, position: Point) {
        let Some(press) = self.press.as_mut() else { return };
        if press.in_sent {
            return;
        }
        press.in_sent = true;
        let node = press.node;
        let payload = pointer_payload(ctx, node, position);
        ctx.emit_if(node, PropKey::OnPressIn, event_kind::PRESS_IN, payload);
    }

    fn pointer_move(&mut self, ctx: &mut Ctx<'_>, position: Point) {
        let Some(press) = self.press else { return };
        if let Some(target) = Self::nearest_with(ctx, press.node, PropKey::OnTouchMove) {
            let payload = pointer_payload(ctx, target, position);
            self.pending_move = Some((target, payload));
        }
        if !press.long_press_cancelled && press.started.distance(position) > LONG_PRESS_SLOP {
            ctx.timers.cancel(Timer::LongPress(press.node));
            if let Some(press) = self.press.as_mut() {
                press.long_press_cancelled = true;
            }
        }
        let inside = Self::retention_bounds(ctx, press.node).contains(position);
        if inside != press.inside {
            if let Some(press) = self.press.as_mut() {
                press.inside = inside;
            }
            self.set_pressed_visual(ctx, press.node, inside);
            let payload = pointer_payload(ctx, press.node, position);
            if inside {
                // Coming back re-arms the press.
                ctx.emit_if(press.node, PropKey::OnPressIn, event_kind::PRESS_IN, payload);
            } else {
                ctx.timers.cancel(Timer::LongPress(press.node));
                ctx.emit_if(press.node, PropKey::OnPressOut, event_kind::PRESS_OUT, payload);
            }
        } else if inside && ctx.scene.node(press.node).is_some_and(|node| node.handles(PropKey::OnPressMove)) {
            let payload = pointer_payload(ctx, press.node, position);
            ctx.emit(press.node, event_kind::PRESS_MOVE, payload);
        }
    }

    fn pointer_up(&mut self, ctx: &mut Ctx<'_>, position: Point) -> Option<Action> {
        self.click_outside(ctx, position);
        let press = self.press.take()?;
        ctx.timers.cancel(Timer::LongPress(press.node));
        ctx.timers.cancel(Timer::PressIn(press.node));
        if let Some(target) = Self::nearest_with(ctx, press.node, PropKey::OnTouchEnd) {
            let payload = pointer_payload(ctx, target, position);
            ctx.emit(target, event_kind::TOUCH_END, payload);
        }
        self.set_pressed_visual(ctx, press.node, false);
        let node = ctx.scene.node(press.node)?;
        let (handles_press, handles_double_tap, action) =
            (node.handles(PropKey::OnPress), node.handles(PropKey::OnDoubleTap), node.action);
        let is_switch = node.kind == NodeKind::Switch;
        let double_tap_delay = node.props().integer_or(PropKey::PressDoubleTapDelayMs, DEFAULT_DOUBLE_TAP_MS).clamp(80, 1000) as u64;
        if !press.inside {
            return None;
        }
        let payload = pointer_payload(ctx, press.node, position);
        if !press.in_sent {
            ctx.emit_if(press.node, PropKey::OnPressIn, event_kind::PRESS_IN, payload.clone());
        }
        ctx.emit_if(press.node, PropKey::OnPressOut, event_kind::PRESS_OUT, payload.clone());
        // A handled long press replaces the press of its release.
        if press.long_pressed {
            return None;
        }
        if action.is_some() {
            return action;
        }
        if is_switch {
            return Some(Action::Toggle { id: press.node });
        }
        if handles_double_tap {
            let previous = self.last_tap.take().filter(|tap| {
                tap.node == press.node
                    && ctx.now_ms.saturating_sub(tap.at_ms) <= double_tap_delay
                    && tap.position.distance(position) <= DOUBLE_TAP_SLOP
            });
            if previous.is_some() {
                // The second tap cancels the single press that was held back.
                ctx.timers.cancel(Timer::DeferredPress(press.node));
                ctx.emit(press.node, event_kind::DOUBLE_TAP, payload);
            } else {
                self.last_tap = Some(Tap { node: press.node, at_ms: ctx.now_ms, position });
                if handles_press {
                    ctx.timers.schedule(Timer::DeferredPress(press.node), ctx.now_ms + double_tap_delay);
                }
            }
            return None;
        }
        if handles_press {
            ctx.emit(press.node, event_kind::PRESS, payload);
        }
        None
    }

    fn cancel_press(&mut self, ctx: &mut Ctx<'_>) {
        let Some(press) = self.press.take() else { return };
        ctx.timers.cancel(Timer::LongPress(press.node));
        ctx.timers.cancel(Timer::PressIn(press.node));
        self.set_pressed_visual(ctx, press.node, false);
        if press.inside && press.in_sent {
            let position = self.pointer.unwrap_or(press.started);
            let payload = pointer_payload(ctx, press.node, position);
            ctx.emit_if(press.node, PropKey::OnPressOut, event_kind::PRESS_OUT, payload);
        }
    }

    /// `ClickOutside`: a pointer release anywhere outside an observing node.
    fn click_outside(&mut self, ctx: &mut Ctx<'_>, position: Point) {
        if self.outside_observers.is_empty() {
            return;
        }
        let payload = WireMap::new().with("pageX", position.x).with("pageY", position.y).encode();
        for observer in self.outside_observers.clone() {
            if !ctx.scene.is_effectively_visible(observer) {
                continue;
            }
            let Some(node) = ctx.scene.node(observer) else { continue };
            let origin = ctx.scene.window_origin(observer);
            let bounds = Rect::new(origin.x, origin.y, node.frame.width, node.frame.height);
            if !bounds.contains(position) {
                ctx.emit(observer, event_kind::CLICK_OUTSIDE, payload.clone());
            }
        }
    }

    pub fn on_timer(&mut self, ctx: &mut Ctx<'_>, timer: Timer) {
        match timer {
            Timer::PressIn(node) => {
                if self.press.is_some_and(|press| press.node == node) {
                    let position = self.pointer.unwrap_or(Point::ZERO);
                    self.send_press_in(ctx, position);
                }
            }
            Timer::LongPress(node) => {
                let Some(press) = self.press.as_mut().filter(|press| press.node == node && press.inside) else { return };
                let Some(target) = press.long_press_target else { return };
                press.long_pressed = true;
                let position = self.pointer.unwrap_or(press.started);
                let payload = pointer_payload(ctx, target, position);
                ctx.emit(target, event_kind::LONG_PRESS, payload);
            }
            Timer::DeferredPress(node) => {
                // No second tap arrived: the single press goes through.
                let position = self.last_tap.filter(|tap| tap.node == node).map_or(Point::ZERO, |tap| tap.position);
                self.last_tap = None;
                let payload = pointer_payload(ctx, node, position);
                ctx.emit_if(node, PropKey::OnPress, event_kind::PRESS, payload);
            }
            _ => {}
        }
    }

    /// Sends what was coalesced during this drain (pointer moves).
    pub fn flush(&mut self, ctx: &mut Ctx<'_>) {
        if let Some((node, payload)) = self.pending_move.take() {
            ctx.emit(node, event_kind::TOUCH_MOVE, payload);
        }
    }

    // ---- keyboard ----------------------------------------------------------

    pub fn on_focus(&mut self, ctx: &mut Ctx<'_>, id: NodeId, focused: bool) {
        let previous = self.focused;
        if focused {
            self.focused = Some(id);
        } else if self.focused == Some(id) {
            self.focused = None;
        }
        for node in [previous, Some(id)].into_iter().flatten() {
            if self.styles.contains_key(&node) {
                self.restyle(ctx, node);
            }
        }
    }

    #[must_use]
    pub fn focused(&self) -> Option<NodeId> {
        self.focused
    }

    /// Enter and Space activate the focused pressable, reporting its center.
    pub fn on_key(&mut self, ctx: &mut Ctx<'_>, key: &KeyEvent) -> Option<Action> {
        if !key.pressed || key.repeat || !matches!(key.key.as_str(), "Enter" | " ") {
            return None;
        }
        let focused = self.focused?;
        let node = ctx.scene.node(focused)?;
        if !node.is_pressable() || !node.props().flag_or(PropKey::Enabled, true) {
            return None;
        }
        if node.action.is_some() {
            return node.action;
        }
        if node.kind == NodeKind::Switch {
            return Some(Action::Toggle { id: focused });
        }
        let origin = ctx.scene.window_origin(focused);
        let center = Point::new(origin.x + node.frame.width / 2.0, origin.y + node.frame.height / 2.0);
        let payload = pointer_payload(ctx, focused, center);
        ctx.emit_if(focused, PropKey::OnPress, event_kind::PRESS, payload);
        None
    }

    /// `KeyDown` / `KeyUp` for the focused element's nearest listener, or for
    /// every visible listener when nothing has focus (screen level handlers).
    pub fn report_key(&mut self, ctx: &mut Ctx<'_>, key: &KeyEvent) {
        let (marker, kind) = if key.pressed { (PropKey::OnKeyDown, event_kind::KEY_DOWN) } else { (PropKey::OnKeyUp, event_kind::KEY_UP) };
        let payload = WireMap::new()
            .with("key", key.key.as_str())
            .with("shift", key.modifiers.shift)
            .with("control", key.modifiers.control)
            .with("alt", key.modifiers.alt)
            .with("meta", key.modifiers.meta)
            .with("repeat", key.repeat)
            .encode();
        match self.focused.and_then(|focused| Self::nearest_with(ctx, focused, marker)) {
            Some(target) => ctx.emit(target, kind, payload),
            None => {
                let listeners: Vec<NodeId> = ctx
                    .scene
                    .nodes()
                    .filter(|node| node.handles(marker))
                    .map(|node| node.id)
                    .filter(|id| ctx.scene.is_effectively_visible(*id))
                    .collect();
                for listener in listeners {
                    ctx.emit(listener, kind, payload.clone());
                }
            }
        }
    }

    /// Drops keyboard focus (a route change, a modal opening).
    pub fn blur(&mut self, ctx: &mut Ctx<'_>) {
        if self.focused.is_some() {
            ctx.commands.push(HostCommand::Focus(None));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_styles_keep_only_host_states_and_known_properties() {
        let styles = StateStyles::parse(r#"{"1":{"38":0.5,"10":4278190335},"3":{"11":-1,"999":1},"5":{"38":0.1}}"#);
        assert_eq!(styles.states.len(), 2);
        let pressed = styles.get(STATE_PRESSED).expect("pressed");
        assert_eq!(pressed.get(&PropKey::Opacity), Some(&PropValue::Float(0.5)));
        assert_eq!(pressed.get(&PropKey::BackgroundColor), Some(&PropValue::Integer(4_278_190_335)));
        assert_eq!(styles.get(STATE_HOVERED).expect("hover").len(), 1);
        assert!(styles.get(5).is_none());
        assert!(StateStyles::parse("not json").states.is_empty());
    }
}
