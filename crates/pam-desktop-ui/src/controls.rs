//! Controls the host draws from scene nodes: the switch and the activity
//! indicator. They animate on the frame clock and never repaint in PHP.

use pam_desktop_wire::{NodeKind, PropKey, PropValue};

use crate::animator::{Curve, Easing, Owner};
use crate::backend::NodeId;
use crate::color::Color;
use crate::context::{Ctx, event_kind};
use crate::geometry::{Affine, Rect};
use crate::props::PropMap;
use crate::scene::{Change, IdMap, LayerOverride, LayerSlot, StyleLayer};

const SWITCH_MS: u32 = 180;
const SWITCH_TRACK_ON: u32 = 0xFF34_C759;
const SWITCH_TRACK_OFF: u32 = 0x2978_7880;
const SWITCH_TRACK_OFF_DARK: u32 = 0x5278_7880;
const SWITCH_THUMB: u32 = 0xFFFF_FFFF;
const SWITCH_THUMB_INSET: f32 = 2.0;
const DISABLED_OPACITY: f32 = 0.5;

const SPINNER_MS: u32 = 900;
const SPINNER_COLOR: u32 = 0xFF8E_8E93;

#[derive(Debug)]
struct Switch {
    thumb: NodeId,
    /// What the control shows (the user may have flipped it ahead of PHP).
    on: bool,
    /// 0 off … 1 on, the animated position.
    position: f32,
    dirty: bool,
    checked_changed: bool,
}

#[derive(Debug, Default)]
struct Spinner {
    spinning: bool,
    dirty: bool,
    dots: Vec<NodeId>,
}

#[derive(Debug, Default)]
struct ButtonSpinner {
    dots: Vec<NodeId>,
    spinning: bool,
    dirty: bool,
}

#[derive(Default)]
pub struct Controls {
    switches: IdMap<Switch>,
    spinners: IdMap<Spinner>,
    button_spinners: IdMap<ButtonSpinner>,
    pub dark: bool,
    dirty: bool,
    visibility_dirty: bool,
}

fn checked(ctx: &Ctx<'_>, id: NodeId) -> bool {
    ctx.scene.node(id).is_some_and(|node| {
        let props = node.props();
        props
            .flag(PropKey::Checked)
            .or_else(|| props.flag(PropKey::Value))
            .unwrap_or(false)
    })
}

impl Controls {
    #[must_use]
    pub fn new(dark: bool) -> Self {
        Self {
            dark,
            ..Self::default()
        }
    }

    pub fn on_change(&mut self, ctx: &mut Ctx<'_>, change: Change) {
        match change {
            Change::Created(id, NodeKind::Switch) => {
                let mut thumb = PropMap::new();
                thumb.insert(PropKey::PointerEvents, PropValue::Integer(2));
                thumb.insert(
                    PropKey::BoxShadows,
                    PropValue::String("[[0,1,3,0,1073741824,0]]".into()),
                );
                let thumb = ctx
                    .scene
                    .add_synthetic(id, NodeKind::View, thumb, Rect::ZERO, 0, None);
                let on = checked(ctx, id);
                self.switches.insert(
                    id,
                    Switch {
                        thumb,
                        on,
                        position: if on { 1.0 } else { 0.0 },
                        dirty: true,
                        checked_changed: false,
                    },
                );
                self.dirty = true;
            }
            Change::Created(id, NodeKind::ActivityIndicator) => {
                self.spinners.insert(
                    id,
                    Spinner {
                        spinning: false,
                        dirty: true,
                        dots: Vec::new(),
                    },
                );
                self.dirty = true;
            }
            Change::Created(id, NodeKind::Button) => {
                self.button_spinners.insert(
                    id,
                    ButtonSpinner {
                        dots: Vec::new(),
                        spinning: false,
                        dirty: true,
                    },
                );
                self.dirty = true;
            }
            Change::Removed(id, NodeKind::Switch) => {
                self.switches.remove(&id);
            }
            Change::Removed(id, NodeKind::ActivityIndicator) => {
                ctx.animator.cancel(Owner::Control(id), ctx.now_ms);
                if let Some(spinner) = self.spinners.remove(&id) {
                    for dot in spinner.dots {
                        ctx.scene.remove_synthetic(dot);
                    }
                }
            }
            Change::Removed(id, NodeKind::Button) => {
                if let Some(button) = self.button_spinners.remove(&id) {
                    ctx.animator.cancel(Owner::Control(id), ctx.now_ms);
                    for dot in button.dots {
                        ctx.scene.remove_synthetic(dot);
                    }
                }
            }
            Change::Updated(id, key) => {
                if key == PropKey::Visible {
                    self.visibility_dirty = true;
                    self.dirty = true;
                }
                if let Some(switch) = self.switches.get_mut(&id) {
                    switch.dirty = true;
                    switch.checked_changed |= matches!(key, PropKey::Checked | PropKey::Value);
                    self.dirty = true;
                } else if let Some(spinner) = self.spinners.get_mut(&id) {
                    spinner.dirty = true;
                    self.dirty = true;
                } else if matches!(
                    key,
                    PropKey::Loading | PropKey::Text | PropKey::TextColor | PropKey::ProgressColor
                ) && let Some(button) = self.button_spinners.get_mut(&id)
                {
                    button.dirty = true;
                    self.dirty = true;
                }
            }
            Change::Frame(id) => {
                if let Some(switch) = self.switches.get_mut(&id) {
                    switch.dirty = true;
                    self.dirty = true;
                } else if let Some(spinner) = self.spinners.get_mut(&id) {
                    spinner.dirty = true;
                    self.dirty = true;
                } else if let Some(button) = self.button_spinners.get_mut(&id) {
                    button.dirty = true;
                    self.dirty = true;
                }
            }
            _ => {}
        }
    }

    pub fn after_commit(&mut self, ctx: &mut Ctx<'_>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let switches: Vec<NodeId> = self
            .switches
            .iter()
            .filter(|(_, switch)| switch.dirty)
            .map(|(id, _)| *id)
            .collect();
        for id in switches {
            let authored = checked(ctx, id);
            let Some(switch) = self.switches.get_mut(&id) else {
                continue;
            };
            switch.dirty = false;
            // A value from PHP overrides what the user flipped.
            if std::mem::take(&mut switch.checked_changed) && switch.on != authored {
                switch.on = authored;
                Self::animate(ctx, id, switch);
            }
            self.paint_switch(ctx, id);
        }
        let visibility_dirty = std::mem::take(&mut self.visibility_dirty);
        let spinners: Vec<NodeId> = self
            .spinners
            .iter()
            .filter(|(_, spinner)| spinner.dirty || visibility_dirty)
            .map(|(id, _)| *id)
            .collect();
        for id in spinners {
            self.sync_spinner(ctx, id);
        }
        let buttons: Vec<NodeId> = self
            .button_spinners
            .iter()
            .filter(|(_, button)| button.dirty || visibility_dirty)
            .map(|(id, _)| *id)
            .collect();
        for id in buttons {
            self.sync_button_spinner(ctx, id);
        }
    }

    /// Appearance changed: default colours follow.
    pub fn refresh(&mut self, ctx: &mut Ctx<'_>) {
        let ids: Vec<NodeId> = self.switches.keys().copied().collect();
        for id in ids {
            self.paint_switch(ctx, id);
        }
    }

    fn animate(ctx: &mut Ctx<'_>, id: NodeId, switch: &mut Switch) {
        let target = if switch.on { 1.0 } else { 0.0 };
        if ctx.reduced_motion {
            ctx.animator.cancel(Owner::Control(id), ctx.now_ms);
            switch.position = target;
        } else {
            ctx.animator.start(
                Owner::Control(id),
                switch.position,
                target,
                Curve::timing(SWITCH_MS, Easing::EASE_IN_OUT),
                0,
                ctx.now_ms,
            );
        }
    }

    /// Track colour and thumb position for the current animated position.
    fn paint_switch(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(switch) = self.switches.get(&id) else {
            return;
        };
        let Some(node) = ctx.scene.node(id) else {
            return;
        };
        let props = node.props();
        let frame = node.frame;
        let enabled = props.flag_or(PropKey::Enabled, true);
        let off = props
            .color(PropKey::SwitchTrackColorFalse)
            .unwrap_or(Color::from_argb(if self.dark {
                SWITCH_TRACK_OFF_DARK
            } else {
                SWITCH_TRACK_OFF
            }));
        let on = props
            .color(PropKey::SwitchTrackColorTrue)
            .unwrap_or(Color::from_argb(SWITCH_TRACK_ON));
        let thumb_color = props
            .color(PropKey::SwitchThumbColor)
            .unwrap_or(Color::from_argb(SWITCH_THUMB));
        let (thumb, position) = (switch.thumb, switch.position);

        let mut track = PropMap::new();
        track.insert(
            PropKey::BackgroundColor,
            PropValue::Integer(i64::from(off.lerp(on, position).to_argb())),
        );
        track.insert(
            PropKey::BorderRadius,
            PropValue::Float(f64::from(frame.height / 2.0)),
        );
        ctx.scene
            .set_style_overlay(id, StyleLayer::Presenter, Some(track));
        ctx.scene.set_layer_override(
            id,
            LayerSlot::Presenter,
            LayerOverride {
                opacity: if enabled { 1.0 } else { DISABLED_OPACITY },
                ..LayerOverride::NONE
            },
        );

        let diameter = (frame.height - SWITCH_THUMB_INSET * 2.0).max(0.0);
        let travel = (frame.width - diameter - SWITCH_THUMB_INSET * 2.0).max(0.0);
        ctx.scene.set_synthetic_frame(
            thumb,
            Rect::new(
                frame.x + SWITCH_THUMB_INSET,
                frame.y + SWITCH_THUMB_INSET,
                diameter,
                diameter,
            ),
        );
        ctx.scene.set_prop(
            thumb,
            PropKey::BackgroundColor,
            Some(PropValue::Integer(i64::from(thumb_color.to_argb()))),
        );
        ctx.scene.set_prop(
            thumb,
            PropKey::BorderRadius,
            Some(PropValue::Float(f64::from(diameter / 2.0))),
        );
        ctx.scene.set_layer_override(
            thumb,
            LayerSlot::Presenter,
            LayerOverride {
                transform: Affine::translation(travel * position, 0.0),
                ..LayerOverride::NONE
            },
        );
    }

    /// The user flipped the switch: it moves at once and PHP hears about it.
    pub fn toggle(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let enabled = ctx
            .scene
            .node(id)
            .is_some_and(|node| node.props().flag_or(PropKey::Enabled, true));
        let Some(switch) = self.switches.get_mut(&id) else {
            return;
        };
        if !enabled {
            return;
        }
        switch.on = !switch.on;
        let on = switch.on;
        Self::animate(ctx, id, switch);
        self.paint_switch(ctx, id);
        ctx.emit_if(
            id,
            PropKey::OnToggle,
            event_kind::TOGGLE,
            if on { b"1".to_vec() } else { b"0".to_vec() },
        );
    }

    fn sync_spinner(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(node) = ctx.scene.node(id) else {
            return;
        };
        let props = node.props();
        let frame = node.frame;
        let parent = node.parent;
        let size = frame.width.min(frame.height);
        let animating = props.flag_or(PropKey::ActivityAnimating, true);
        let parent_visible = parent == 0 || ctx.scene.is_effectively_visible(parent);
        let visible = props.flag_or(PropKey::Visible, true) && parent_visible;
        let hidden =
            !(visible && (animating || !props.flag_or(PropKey::ActivityHidesWhenStopped, true)));
        let hides_when_stopped = props.flag_or(PropKey::ActivityHidesWhenStopped, true);
        let color = props.integer_or(PropKey::ProgressColor, i64::from(SPINNER_COLOR));
        let Some(spinner) = self.spinners.get_mut(&id) else {
            return;
        };
        spinner.dirty = false;

        let spin = animating && !hidden && !ctx.reduced_motion && size > 0.0;
        let show_dots = visible && size > 0.0 && (animating || !hides_when_stopped);
        let dot_size = (size * 0.12).clamp(2.0, 3.0);
        if show_dots && spinner.dots.is_empty() {
            for index in 0..8 {
                let mut props = PropMap::new();
                props.insert(PropKey::PointerEvents, PropValue::Integer(2));
                props.insert(PropKey::Accessible, PropValue::Boolean(false));
                props.insert(
                    PropKey::BackgroundColor,
                    PropValue::Integer(Self::spinner_dot_color(color, index)),
                );
                props.insert(
                    PropKey::BorderRadius,
                    PropValue::Float(f64::from(dot_size / 2.0)),
                );
                let dot = ctx.scene.add_synthetic(
                    id,
                    NodeKind::View,
                    props,
                    Self::spinner_dot_frame(frame, size, dot_size, index),
                    index + 1,
                    None,
                );
                spinner.dots.push(dot);
            }
        } else if !show_dots && !spinner.dots.is_empty() {
            for dot in spinner.dots.drain(..) {
                ctx.scene.remove_synthetic(dot);
            }
        } else if show_dots {
            for (index, dot) in spinner.dots.iter().copied().enumerate() {
                ctx.scene.set_synthetic_frame(
                    dot,
                    Self::spinner_dot_frame(frame, size, dot_size, index as u32),
                );
            }
        }
        if spin && !spinner.spinning {
            ctx.animator.start(
                Owner::Control(id),
                0.0,
                360.0,
                Curve::timing(SPINNER_MS, Easing::Linear),
                0,
                ctx.now_ms,
            );
        } else if !spin && spinner.spinning {
            ctx.animator.cancel(Owner::Control(id), ctx.now_ms);
        }
        spinner.spinning = spin;
        let mut clear_surface = PropMap::new();
        clear_surface.insert(PropKey::BackgroundColor, PropValue::Integer(0));
        clear_surface.insert(PropKey::BorderWidth, PropValue::Float(0.0));
        clear_surface.insert(PropKey::BorderColor, PropValue::Integer(0));
        ctx.scene
            .set_style_overlay(id, StyleLayer::Presenter, Some(clear_surface));
    }

    fn spinner_dot_color(color: i64, index: u32) -> i64 {
        let argb = color as u32;
        let alpha = ((argb >> 24) as f32 * (1.0 - index as f32 * 0.095)).round() as u32;
        i64::from((alpha << 24) | (argb & 0x00FF_FFFF))
    }

    fn spinner_dot_frame(frame: Rect, size: f32, dot_size: f32, index: u32) -> Rect {
        let angle = index as f32 * std::f32::consts::TAU / 8.0 - std::f32::consts::FRAC_PI_2;
        let radius = size * 0.35;
        let center_x = frame.x + frame.width / 2.0;
        let center_y = frame.y + frame.height / 2.0;
        Rect::new(
            center_x + angle.cos() * radius - dot_size / 2.0,
            center_y + angle.sin() * radius - dot_size / 2.0,
            dot_size,
            dot_size,
        )
    }

    /// A loading Button keeps its accessible label and busy state while a
    /// native dot indicator occupies the label area.
    fn sync_button_spinner(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(node) = ctx.scene.node(id) else {
            return;
        };
        let props = node.props();
        let loading = props.flag_or(PropKey::Loading, false);
        let label = props.text_or_empty(PropKey::Text).to_owned();
        let color = props.integer_or(PropKey::TextColor, 0xFFFF_FFFF);
        let size = node.frame.height.clamp(12.0, 18.0);
        let node_frame = node.frame;
        let Some(button) = self.button_spinners.get_mut(&id) else {
            return;
        };
        button.dirty = false;

        if loading {
            if button.dots.is_empty() {
                for index in 0..8 {
                    let mut dot_props = PropMap::new();
                    dot_props.insert(PropKey::PointerEvents, PropValue::Integer(2));
                    dot_props.insert(PropKey::Accessible, PropValue::Boolean(false));
                    dot_props.insert(
                        PropKey::BackgroundColor,
                        PropValue::Integer(Self::spinner_dot_color(color, index)),
                    );
                    dot_props.insert(
                        PropKey::BorderRadius,
                        PropValue::Float(f64::from((size * 0.12).clamp(2.0, 3.0) / 2.0)),
                    );
                    let dot = ctx.scene.add_synthetic(
                        id,
                        NodeKind::View,
                        dot_props,
                        Self::spinner_dot_frame(
                            node_frame,
                            size,
                            (size * 0.12).clamp(2.0, 3.0),
                            index,
                        ),
                        index + 1,
                        None,
                    );
                    button.dots.push(dot);
                }
            } else {
                for (index, dot) in button.dots.iter().copied().enumerate() {
                    ctx.scene.set_synthetic_frame(
                        dot,
                        Self::spinner_dot_frame(
                            node_frame,
                            size,
                            (size * 0.12).clamp(2.0, 3.0),
                            index as u32,
                        ),
                    );
                    ctx.scene.set_prop(
                        dot,
                        PropKey::BackgroundColor,
                        Some(PropValue::Integer(Self::spinner_dot_color(
                            color,
                            index as u32,
                        ))),
                    );
                }
            }
            let spin = ctx.scene.is_effectively_visible(id) && !ctx.reduced_motion;
            if spin && !button.spinning {
                ctx.animator.start(
                    Owner::Control(id),
                    0.0,
                    360.0,
                    Curve::timing(SPINNER_MS, Easing::Linear),
                    0,
                    ctx.now_ms,
                );
            } else if !spin && button.spinning {
                ctx.animator.cancel(Owner::Control(id), ctx.now_ms);
            }
            button.spinning = spin;
            let mut overlay = PropMap::new();
            overlay.insert(PropKey::Text, PropValue::String(String::new()));
            overlay.insert(PropKey::AccessibilityLabel, PropValue::String(label));
            ctx.scene
                .set_style_overlay(id, StyleLayer::Presenter, Some(overlay));
        } else {
            ctx.animator.cancel(Owner::Control(id), ctx.now_ms);
            button.spinning = false;
            ctx.scene.set_style_overlay(id, StyleLayer::Presenter, None);
            for dot in button.dots.drain(..) {
                ctx.scene.remove_synthetic(dot);
            }
        }
    }

    /// Frame-clock sample of a switch position or a spinner angle.
    pub fn on_sample(&mut self, ctx: &mut Ctx<'_>, id: NodeId, value: f32, finished: bool) {
        if self.button_spinners.contains_key(&id) {
            self.animate_button_spinner(ctx, id, value, finished);
            return;
        }
        if let Some(switch) = self.switches.get_mut(&id) {
            switch.position = value.clamp(0.0, 1.0);
            self.paint_switch(ctx, id);
            return;
        }
        let Some(spinner) = self.spinners.get(&id) else {
            return;
        };
        if !spinner.spinning {
            return;
        }
        // Shade the native dots instead of transforming their container. GTK
        // embeds native children beneath this node; rotating that surface can
        // produce an invalid GSK node instead of the progress glyph.
        let color = ctx
            .scene
            .node(id)
            .map(|node| {
                node.props()
                    .integer_or(PropKey::ProgressColor, i64::from(SPINNER_COLOR))
            })
            .unwrap_or(i64::from(SPINNER_COLOR));
        Self::shade_spinner_dots(ctx, &spinner.dots, color, value);
        if finished {
            ctx.animator.start(
                Owner::Control(id),
                0.0,
                360.0,
                Curve::timing(SPINNER_MS, Easing::Linear),
                0,
                ctx.now_ms,
            );
        }
    }

    fn animate_button_spinner(
        &mut self,
        ctx: &mut Ctx<'_>,
        id: NodeId,
        value: f32,
        finished: bool,
    ) {
        let Some(button) = self.button_spinners.get(&id) else {
            return;
        };
        if !button.spinning {
            return;
        }
        let color = ctx
            .scene
            .node(id)
            .map(|node| node.props().integer_or(PropKey::TextColor, 0xFFFF_FFFF))
            .unwrap_or(0xFFFF_FFFF);
        Self::shade_spinner_dots(ctx, &button.dots, color, value);
        if finished {
            ctx.animator.start(
                Owner::Control(id),
                0.0,
                360.0,
                Curve::timing(SPINNER_MS, Easing::Linear),
                0,
                ctx.now_ms,
            );
        }
    }
    fn shade_spinner_dots(ctx: &mut Ctx<'_>, dots: &[NodeId], color: i64, value: f32) {
        let phase = value.rem_euclid(360.0) / 45.0;
        for (index, dot) in dots.iter().copied().enumerate() {
            let distance = ((index as f32 - phase).rem_euclid(8.0))
                .min((phase - index as f32).rem_euclid(8.0));
            let alpha = (1.0 - distance / 8.0).clamp(0.25, 1.0);
            let argb = color as u32;
            let a = (((argb >> 24) as f32) * alpha).round() as u32;
            let shaded = i64::from((a << 24) | (argb & 0x00FF_FFFF));
            ctx.scene.set_prop(
                dot,
                PropKey::BackgroundColor,
                Some(PropValue::Integer(shaded)),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;
    use crate::testing::{Harness, TreeNode};
    use pam_desktop_wire::{NodeKind, PropKey, PropValue};

    fn settle(controls: &mut Controls, harness: &mut Harness) {
        for _ in 0..4 {
            let changes = harness.scene.take_changes();
            if changes.is_empty() {
                break;
            }
            for change in changes {
                let mut ctx = harness.ctx();
                controls.on_change(&mut ctx, change);
            }
            let mut ctx = harness.ctx();
            controls.after_commit(&mut ctx);
        }
    }

    #[test]
    fn loading_button_uses_native_spinner_and_restores_its_label() {
        let properties = [
            (PropKey::Text, PropValue::String("Save".into())),
            (PropKey::Loading, PropValue::Boolean(true)),
            (PropKey::TextColor, PropValue::Integer(0xFFFF_FFFF)),
        ];
        let nodes: [TreeNode<'_>; 2] = [
            (1, 0, NodeKind::View, &[], Rect::new(0.0, 0.0, 240.0, 80.0)),
            (
                2,
                1,
                NodeKind::Button,
                &properties,
                Rect::new(0.0, 0.0, 240.0, 48.0),
            ),
        ];
        let mut harness = Harness::with_tree(&nodes);
        let mut controls = Controls::new(true);
        settle(&mut controls, &mut harness);

        let dots = controls.button_spinners.get(&2).unwrap().dots.clone();
        harness
            .scene
            .flush(&mut crate::testing::RecordingBackend::default(), 1);
        assert_eq!(dots.len(), 8);
        assert!(dots.iter().all(|dot| {
            harness
                .scene
                .node(*dot)
                .is_some_and(|node| node.kind == NodeKind::View && node.parent == 2)
        }));
        assert_ne!(
            harness.scene.node(dots[0]).unwrap().frame,
            harness.scene.node(dots[4]).unwrap().frame
        );
        assert!(controls.button_spinners.get(&2).unwrap().spinning);
        assert!(harness.scene.text_block(2).is_none());

        harness
            .scene
            .set_prop(2, PropKey::Loading, Some(PropValue::Boolean(false)));
        settle(&mut controls, &mut harness);
        assert!(dots.iter().all(|dot| harness.scene.node(*dot).is_none()));
        assert_eq!(harness.scene.text_block(2).unwrap().plain_text(), "Save");
    }

    #[test]
    fn activity_indicator_animates_dots_without_transforming_native_container() {
        let properties = [(PropKey::ProgressColor, PropValue::Integer(0xFF022C22))];
        let nodes: [TreeNode<'_>; 2] = [
            (1, 0, NodeKind::View, &[], Rect::new(0.0, 0.0, 100.0, 42.0)),
            (
                2,
                1,
                NodeKind::ActivityIndicator,
                &properties,
                Rect::new(12.0, 12.0, 16.0, 16.0),
            ),
        ];
        let mut harness = Harness::with_tree(&nodes);
        let mut controls = Controls::new(true);
        settle(&mut controls, &mut harness);
        let dots = controls.spinners.get(&2).unwrap().dots.clone();
        assert_eq!(dots.len(), 8);
        controls.on_sample(&mut harness.ctx(), 2, 90.0, false);
        assert_eq!(
            harness
                .scene
                .layer_override(2, LayerSlot::Presenter)
                .transform,
            Affine::IDENTITY
        );
        let first = harness
            .scene
            .node(dots[0])
            .unwrap()
            .props()
            .integer(PropKey::BackgroundColor)
            .unwrap();
        let active = harness
            .scene
            .node(dots[2])
            .unwrap()
            .props()
            .integer(PropKey::BackgroundColor)
            .unwrap();
        assert_eq!(active, 0xFF022C22);
        assert_ne!(first, active);
        harness.scene.set_prop(
            2,
            PropKey::ActivityAnimating,
            Some(PropValue::Boolean(false)),
        );
        settle(&mut controls, &mut harness);
        assert!(!controls.spinners.get(&2).unwrap().spinning);
        assert!(dots.iter().all(|id| harness.scene.node(*id).is_none()));
    }

    #[test]
    fn stopped_activity_indicator_can_restart_after_its_own_hidden_state() {
        let properties = [(PropKey::ActivityAnimating, PropValue::Boolean(false))];
        let nodes: [TreeNode<'_>; 2] = [
            (1, 0, NodeKind::View, &[], Rect::new(0.0, 0.0, 80.0, 40.0)),
            (
                2,
                1,
                NodeKind::ActivityIndicator,
                &properties,
                Rect::new(0.0, 0.0, 20.0, 20.0),
            ),
        ];
        let mut harness = Harness::with_tree(&nodes);
        let mut controls = Controls::new(true);
        settle(&mut controls, &mut harness);
        assert!(!controls.spinners.get(&2).unwrap().spinning);

        harness.scene.set_prop(
            2,
            PropKey::ActivityAnimating,
            Some(PropValue::Boolean(true)),
        );
        settle(&mut controls, &mut harness);
        assert!(controls.spinners.get(&2).unwrap().spinning);
        assert_eq!(controls.spinners.get(&2).unwrap().dots.len(), 8);
    }
}
