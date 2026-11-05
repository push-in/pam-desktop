//! Controls the host draws from scene nodes: the switch and the activity
//! indicator. They animate on the frame clock and never repaint in PHP.

use pam_desktop_wire::{NodeKind, PropKey, PropValue};

use crate::animator::{Curve, Easing, Owner};
use crate::backend::NodeId;
use crate::color::Color;
use crate::context::{Ctx, event_kind};
use crate::geometry::{Affine, Point, Rect};
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
}

#[derive(Default)]
pub struct Controls {
    switches: IdMap<Switch>,
    spinners: IdMap<Spinner>,
    pub dark: bool,
    dirty: bool,
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
                    },
                );
                self.dirty = true;
            }
            Change::Removed(id, NodeKind::Switch) => {
                self.switches.remove(&id);
            }
            Change::Removed(id, NodeKind::ActivityIndicator) => {
                self.spinners.remove(&id);
            }
            Change::Updated(id, key) => {
                if let Some(switch) = self.switches.get_mut(&id) {
                    switch.dirty = true;
                    switch.checked_changed |= matches!(key, PropKey::Checked | PropKey::Value);
                    self.dirty = true;
                } else if let Some(spinner) = self.spinners.get_mut(&id) {
                    spinner.dirty = true;
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
        let spinners: Vec<NodeId> = self
            .spinners
            .iter()
            .filter(|(_, spinner)| spinner.dirty)
            .map(|(id, _)| *id)
            .collect();
        for id in spinners {
            self.sync_spinner(ctx, id);
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
        let size = node.frame.width.min(node.frame.height);
        let animating = props.flag_or(PropKey::ActivityAnimating, true);
        let visible = props.flag_or(PropKey::Visible, true);
        let hidden =
            !(visible && (animating || !props.flag_or(PropKey::ActivityHidesWhenStopped, true)));
        let color = props.integer_or(PropKey::ProgressColor, i64::from(SPINNER_COLOR));
        let Some(spinner) = self.spinners.get_mut(&id) else {
            return;
        };
        spinner.dirty = false;

        // A ring with one transparent quarter, turned by the frame clock.
        let stroke = (size * 0.12).clamp(2.0, 4.0);
        let mut ring = PropMap::new();
        ring.insert(PropKey::BorderWidth, PropValue::Float(f64::from(stroke)));
        ring.insert(
            PropKey::BorderRadius,
            PropValue::Float(f64::from(size / 2.0)),
        );
        ring.insert(PropKey::BorderColor, PropValue::Integer(color));
        ring.insert(PropKey::BorderLeftColor, PropValue::Integer(0));
        ctx.scene
            .set_style_overlay(id, StyleLayer::Presenter, Some(ring));

        let spin = animating && !hidden && !ctx.reduced_motion && size > 0.0;
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
        if hidden {
            ctx.scene
                .set_layer_override(id, LayerSlot::Presenter, LayerOverride::HIDDEN);
        } else if !spin {
            ctx.scene
                .set_layer_override(id, LayerSlot::Presenter, LayerOverride::NONE);
        }
    }

    /// Frame-clock sample of a switch position or a spinner angle.
    pub fn on_sample(&mut self, ctx: &mut Ctx<'_>, id: NodeId, value: f32, finished: bool) {
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
        let Some(size) = ctx.scene.node(id).map(|node| node.frame.size()) else {
            return;
        };
        let center = Point::new(size.width / 2.0, size.height / 2.0);
        ctx.scene.set_layer_override(
            id,
            LayerSlot::Presenter,
            LayerOverride {
                transform: Affine::rotation(value).around(center),
                ..LayerOverride::NONE
            },
        );
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
}
