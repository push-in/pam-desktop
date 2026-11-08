//! Toasts: one short message over the application, drawn as host chrome.
//!
//! Two forms share the `toast.show` request: a plain capsule near the
//! bottom, and a card with a title, an accent strip and an optional icon at
//! the top or the bottom. Presses outside the toast reach the application.

use pam_desktop_wire::{NodeKind, PropKey, PropValue};

use crate::animator::{Curve, Easing, Owner, Timer};
use crate::backend::NodeId;
use crate::chrome::{Builder, Measure, TextStyle};
use crate::context::Ctx;
use crate::geometry::{Affine, Rect};
use crate::scene::{LayerOverride, LayerSlot};
use crate::wiremap::WireMap;

/// Command of a press on the toast (carried by `Action::Chrome`).
pub const DISMISS_COMMAND: u16 = 100;
const ENTER_MS: u32 = 250;
const EXIT_MS: u32 = 200;
/// How far the toast travels while it fades.
const TRAVEL: f32 = 24.0;

#[derive(Default)]
pub struct Toasts {
    root: Option<NodeId>,
    /// Slides in from above (top toasts) or from below.
    from_top: bool,
    leaving: bool,
    roots: Vec<NodeId>,
}

fn color(request: &WireMap, key: &str, default: i64) -> i64 {
    request.integer(key).unwrap_or(default)
}

fn length(request: &WireMap, key: &str, default: f32, minimum: f32, maximum: f32) -> f32 {
    request
        .number(key)
        .map_or(default, |value| value as f32)
        .clamp(minimum, maximum)
}

impl Toasts {
    #[must_use]
    pub fn roots(&self) -> &[NodeId] {
        &self.roots
    }

    /// Shows a toast, replacing the one on screen.
    pub fn show(&mut self, ctx: &mut Ctx<'_>, measure: Measure<'_>, request: &WireMap) {
        self.remove(ctx);
        let viewport = ctx.viewport;
        let reduced_motion = ctx.reduced_motion;
        let now = ctx.now_ms;
        let mut builder = Builder::new(ctx, measure);
        let (root, duration) = if request.text("title").is_some() {
            self.from_top = request.boolean("bottom") != Some(true);
            (
                Self::card(&mut builder, request, viewport.width, viewport.height),
                request
                    .integer("durationMs")
                    .unwrap_or(4000)
                    .clamp(500, 60_000) as u64,
            )
        } else {
            self.from_top = false;
            (
                Self::capsule(&mut builder, request, viewport.width, viewport.height),
                if request.boolean("long") == Some(true) {
                    3500
                } else {
                    2000
                },
            )
        };
        self.root = Some(root);
        self.roots = vec![root];
        self.leaving = false;
        ctx.timers.schedule(Timer::ToastDismiss, now + duration);
        if reduced_motion {
            ctx.animator.cancel(Owner::Toast, now);
        } else {
            self.apply(ctx, 0.0);
            ctx.animator.start(
                Owner::Toast,
                0.0,
                1.0,
                Curve::timing(ENTER_MS, Easing::OutCubic),
                0,
                now,
            );
        }
    }

    /// The plain form: a dark capsule above the bottom edge.
    fn capsule(
        builder: &mut Builder<'_, '_, '_>,
        request: &WireMap,
        window_width: f32,
        window_height: f32,
    ) -> NodeId {
        let message = request.text("message").unwrap_or_default();
        let style = TextStyle::new(14.0, 500, 0xFFFF_FFFF).lines(3).centered();
        let limit = (window_width - 32.0 - 32.0).max(80.0);
        let text_width = builder.text_width(message, &style).min(limit).ceil();
        let text_height = builder.text_height(text_width, message, &style);
        let (width, height) = (text_width + 32.0, text_height + 18.0);
        let frame = Rect::new(
            (window_width - width) / 2.0,
            window_height - 48.0 - height,
            width,
            height,
        );
        let root = builder.view(
            0,
            NodeKind::View,
            frame,
            &[
                (PropKey::BackgroundColor, PropValue::Integer(0xD100_0000)),
                (PropKey::BorderRadius, PropValue::Float(18.0)),
                (PropKey::AccessibilityRole, PropValue::Integer(7)),
                (
                    PropKey::AccessibilityLabel,
                    PropValue::String(message.to_owned()),
                ),
                (PropKey::AccessibilityLiveRegion, PropValue::Integer(2)),
            ],
            Some(DISMISS_COMMAND),
        );
        builder.text(
            root,
            frame.x + 16.0,
            frame.y + 9.0,
            text_width,
            message,
            &style,
        );
        root
    }

    /// The card form: accent strip, optional icon, title and message.
    fn card(
        builder: &mut Builder<'_, '_, '_>,
        request: &WireMap,
        window_width: f32,
        window_height: f32,
    ) -> NodeId {
        let title = request.text("title").unwrap_or_default();
        let message = request.text("message").unwrap_or_default();
        let accent_width = length(request, "accentWidth", 5.0, 0.0, 32.0);
        let (padding_x, padding_y) = (
            length(request, "paddingHorizontal", 15.0, 0.0, 64.0),
            length(request, "paddingVertical", 10.0, 0.0, 64.0),
        );
        let radius = length(request, "cornerRadius", 6.0, 0.0, 64.0);
        let elevation = length(request, "elevation", 2.0, 0.0, 24.0);
        let family = request.text("fontFamily").unwrap_or_default().to_owned();
        let title_color = color(request, "titleColor", 0xFF00_0000);
        let mut title_style = TextStyle::new(
            length(request, "titleSize", 12.0, 1.0, 96.0),
            700,
            title_color,
        )
        .lines(request.integer("titleLines").unwrap_or(1).clamp(1, 4));
        let mut message_style = TextStyle::new(
            length(request, "messageSize", 10.0, 1.0, 96.0),
            400,
            color(request, "messageColor", 0xFF97_9797),
        )
        .lines(2);
        title_style.family.clone_from(&family);
        message_style.family = family;

        let glyph = request.text("iconGlyph").filter(|glyph| !glyph.is_empty());
        let image = request
            .text("iconSource")
            .filter(|source| !source.is_empty());
        let icon_size = if glyph.is_some() || image.is_some() {
            length(request, "iconSize", 18.0, 1.0, 64.0)
        } else {
            0.0
        };
        let icon_space = if icon_size > 0.0 {
            icon_size + 10.0
        } else {
            0.0
        };

        let left = padding_x + accent_width;
        let chrome = left + icon_space + padding_x;
        let mut width = (window_width - 32.0).clamp(120.0, 340.0);
        if request.boolean("fitContent") == Some(true) {
            let content = builder
                .text_width(title, &title_style)
                .max(if message.is_empty() {
                    0.0
                } else {
                    builder.text_width(message, &message_style)
                });
            width = (content.ceil() + chrome).min((window_width - 32.0).max(120.0));
        }
        let text_width = (width - chrome).max(40.0);
        let title_height = builder.text_height(text_width, title, &title_style);
        let message_height = if message.is_empty() {
            0.0
        } else {
            builder.text_height(text_width, message, &message_style) + 2.0
        };
        let content_height = (title_height + message_height).max(icon_size);
        let height =
            (content_height + padding_y * 2.0).max(length(request, "minHeight", 60.0, 0.0, 200.0));
        let offset = length(request, "offset", 40.0, 0.0, 10_000.0);
        let top = if request.boolean("bottom") == Some(true) {
            window_height - offset - height
        } else {
            offset
        };
        let frame = Rect::new((window_width - width) / 2.0, top, width, height);

        let label = if message.is_empty() {
            title.to_owned()
        } else {
            format!("{title}, {message}")
        };
        let mut props = vec![
            (
                PropKey::BackgroundColor,
                PropValue::Integer(color(request, "backgroundColor", 0xFFFF_FFFF)),
            ),
            (PropKey::BorderRadius, PropValue::Float(f64::from(radius))),
            (PropKey::Overflow, PropValue::Integer(2)),
            (PropKey::AccessibilityRole, PropValue::Integer(7)),
            (PropKey::AccessibilityLabel, PropValue::String(label)),
            (PropKey::AccessibilityLiveRegion, PropValue::Integer(2)),
        ];
        if elevation > 0.0 {
            // offset x, offset y, blur, spread, colour, inset.
            props.push((
                PropKey::BoxShadows,
                PropValue::String(format!(
                    "[[0,{},{},0,{},0]]",
                    elevation,
                    elevation * 3.0,
                    0x3300_0000_u32
                )),
            ));
        }
        if let (Some(border_color), Some(border_width)) = (
            request.integer("borderColor"),
            request.number("borderWidth"),
        ) {
            props.push((PropKey::BorderColor, PropValue::Integer(border_color)));
            props.push((
                PropKey::BorderWidth,
                PropValue::Float(border_width.clamp(0.0, 8.0)),
            ));
        }
        let root = builder.view(0, NodeKind::View, frame, &props, Some(DISMISS_COMMAND));
        if accent_width > 0.0 {
            let accent = color(request, "accentColor", 0xFF87_CEFA);
            builder.view(
                root,
                NodeKind::View,
                Rect::new(frame.x, frame.y, accent_width, height),
                &[
                    (PropKey::BackgroundColor, PropValue::Integer(accent)),
                    (PropKey::PointerEvents, PropValue::Integer(2)),
                ],
                None,
            );
        }
        let center = frame.y + height / 2.0;
        if icon_size > 0.0 {
            let icon_frame = Rect::new(
                frame.x + left,
                center - icon_size / 2.0,
                icon_size,
                icon_size,
            );
            if let Some(glyph) = glyph {
                let mut props = vec![
                    (PropKey::Text, PropValue::String(glyph.to_owned())),
                    (PropKey::FontSize, PropValue::Float(f64::from(icon_size))),
                    (
                        PropKey::TextColor,
                        PropValue::Integer(color(request, "iconColor", title_color)),
                    ),
                    (PropKey::TextAlign, PropValue::Integer(2)),
                    (PropKey::PointerEvents, PropValue::Integer(2)),
                ];
                if let Some(font) = request
                    .text("iconFontFamily")
                    .filter(|font| !font.is_empty())
                {
                    props.push((PropKey::FontFamily, PropValue::String(font.to_owned())));
                }
                builder.view(root, NodeKind::Text, icon_frame, &props, None);
            } else if let Some(source) = image {
                let mut props = vec![
                    (PropKey::Source, PropValue::String(source.to_owned())),
                    (PropKey::ImageFit, PropValue::Integer(2)),
                    (PropKey::PointerEvents, PropValue::Integer(2)),
                ];
                if let Some(tint) = request.integer("iconColor") {
                    props.push((PropKey::TintColor, PropValue::Integer(tint)));
                }
                builder.view(root, NodeKind::Image, icon_frame, &props, None);
            }
        }
        let text_x = frame.x + left + icon_space;
        let text_top = center - (title_height + message_height) / 2.0;
        builder.text(root, text_x, text_top, text_width, title, &title_style);
        if !message.is_empty() {
            builder.text(
                root,
                text_x,
                text_top + title_height + 2.0,
                text_width,
                message,
                &message_style,
            );
        }
        root
    }

    /// Fade and slide for the animation progress (0 away, 1 in place).
    fn apply(&self, ctx: &mut Ctx<'_>, progress: f32) {
        let Some(root) = self.root else { return };
        let travel = (1.0 - progress) * TRAVEL * if self.from_top { -1.0 } else { 1.0 };
        let value = if progress >= 1.0 {
            LayerOverride::NONE
        } else {
            LayerOverride {
                opacity: progress.clamp(0.0, 1.0),
                transform: Affine::translation(0.0, travel),
                hidden: false,
            }
        };
        ctx.scene
            .set_layer_override(root, LayerSlot::Presenter, value);
    }

    pub fn on_sample(&mut self, ctx: &mut Ctx<'_>, value: f32, finished: bool) -> bool {
        self.apply(ctx, value);
        if finished && self.leaving {
            self.remove(ctx);
            return true;
        }
        false
    }

    /// The toast's time is over, or it was pressed. Returns whether the
    /// overlay list changed.
    pub fn dismiss(&mut self, ctx: &mut Ctx<'_>) -> bool {
        if self.root.is_none() || self.leaving {
            return false;
        }
        ctx.timers.cancel(Timer::ToastDismiss);
        if ctx.reduced_motion {
            self.remove(ctx);
            return true;
        }
        self.leaving = true;
        let from = ctx.animator.cancel(Owner::Toast, ctx.now_ms).unwrap_or(1.0);
        ctx.animator.start(
            Owner::Toast,
            from,
            0.0,
            Curve::timing(EXIT_MS, Easing::InCubic),
            0,
            ctx.now_ms,
        );
        false
    }

    /// Takes the toast away at once (new toast, reload).
    pub fn remove(&mut self, ctx: &mut Ctx<'_>) {
        ctx.timers.cancel(Timer::ToastDismiss);
        ctx.animator.cancel(Owner::Toast, ctx.now_ms);
        if let Some(root) = self.root.take() {
            ctx.scene.remove_synthetic(root);
        }
        self.roots.clear();
        self.leaving = false;
    }
}
