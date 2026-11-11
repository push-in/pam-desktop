//! Scroll views and lists: scroll events, programmatic scrolling, sticky
//! headers, list windowing, sections, string rows and refresh.
//!
//! The viewport itself is the backend's native scroll view; this controller
//! owns everything the protocol defines on top of it.

use pam_desktop_wire::{NodeKind, PropKey, PropValue};

use crate::animator::{Curve, Easing, Owner, Timer};
use crate::backend::{KeyEvent, NodeId, ViewClass};
use crate::context::{Ctx, HostCommand, event_kind};
use crate::geometry::{Affine, Point, Rect};
use crate::props::PropMap;
use crate::scene::{Change, IdMap, LayerOverride, LayerSlot, Scene, StyleLayer};
use crate::wiremap::{WireMap, decode_string_list};

/// No offset for this long means the scroll gesture ended.
const REST_MS: u64 = 140;
const WINDOW_REPEAT_MS: u64 = 250;
const REQUEST_GLIDE_MS: u32 = 250;
const REFRESH_FALLBACK_MS: u64 = 600;
const STRING_ROW_HEIGHT: f32 = 48.0;

#[derive(Debug, Default)]
struct Scroll {
    horizontal: bool,
    /// `ListInverted`: row 0 rests at the visual end. `offset` stays
    /// logical (0 = row 0 at that edge); the native offset is mirrored.
    inverted: bool,
    /// The native offset matches the logical one at least once.
    inverted_placed: bool,
    offset: Point,
    /// Largest reachable offset on the scroll axis at the last sync.
    max: f32,
    /// A user gesture is in progress since this offset.
    gesture: Option<f32>,
    last_step: Option<(u64, f32)>,
    velocity: f32,
    end_armed: bool,
    end_content: f32,
    end_viewport: f32,
    page: Option<i64>,
    request: Option<i64>,
    /// A `contentOffset` request waiting for its content to exist.
    pending_offset: Option<Point>,
    offset_applied_once: bool,
    anchored_to_end: bool,
    initial_index: Option<i64>,
    /// Whether the last real scroll left the list at its end.
    at_end: bool,
    window_report: Option<(String, u64)>,
    section_offsets: Vec<(String, f32)>,
    active_section: String,
    /// Synthetic rows of a string list.
    rows: Vec<NodeId>,
    sync: bool,
    request_changed: bool,
    offset_changed: bool,
    items_changed: bool,
    /// A `Scroll` event waiting for the end of the drain.
    report: bool,
}

#[derive(Debug, Default)]
struct Refresh {
    indicator: Option<NodeId>,
    /// The disc behind the spinner, when the host asks for a background.
    backdrop: Option<NodeId>,
    /// The user asked for a refresh PHP has not acknowledged yet.
    requested: bool,
    sync: bool,
}

#[derive(Default)]
pub struct Scrolling {
    scrolls: IdMap<Scroll>,
    refreshes: IdMap<Refresh>,
    /// Sticky node -> its scroll view.
    sticky: IdMap<NodeId>,
    /// Nodes that want `Layout` events -> the frame last reported.
    layout_listeners: IdMap<Option<Rect>>,
    layout_dirty: bool,
    dirty: bool,
}

fn is_scrollable(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Scroll | NodeKind::List | NodeKind::SectionList | NodeKind::VirtualList
    )
}

fn axis(point: Point, horizontal: bool) -> f32 {
    if horizontal { point.x } else { point.y }
}

fn nearest_scroll(scene: &Scene, id: NodeId) -> Option<NodeId> {
    let mut current = scene.node(id)?.parent;
    while let Some(node) = scene.node(current) {
        if node.class == ViewClass::Scroll {
            return Some(current);
        }
        current = node.parent;
    }
    None
}

/// CSS `position: sticky` (PositionType 4).
fn is_css_sticky(props: crate::props::Props<'_>) -> bool {
    props.integer(PropKey::PositionType) == Some(4)
}

/// How far a `position: sticky` box moves at a scroll offset, as CSS
/// positioned layout computes it: it stays in flow until its edge would pass
/// the inset from the scroll view's edge, then sticks there, never leaving
/// its parent (its containing block). Only the scrolled axis moves.
fn css_sticky_shift(
    scene: &Scene,
    id: NodeId,
    viewport: Rect,
    offset: Point,
    horizontal: bool,
) -> Point {
    let Some(node) = scene.node(id) else {
        return Point::new(0.0, 0.0);
    };
    let Some(parent) = scene.node(node.parent) else {
        return Point::new(0.0, 0.0);
    };
    let props = node.props();
    let (start, end) = if horizontal {
        (PropKey::Left, PropKey::Right)
    } else {
        (PropKey::Top, PropKey::Bottom)
    };
    let (position, size, parent_start, parent_size, view_start, view_size, scrolled) = if horizontal
    {
        (
            node.frame.x,
            node.frame.width,
            parent.frame.x,
            parent.frame.width,
            viewport.x,
            viewport.width,
            offset.x,
        )
    } else {
        (
            node.frame.y,
            node.frame.height,
            parent.frame.y,
            parent.frame.height,
            viewport.y,
            viewport.height,
            offset.y,
        )
    };
    // Room inside the containing block before and after the box.
    let room_before = parent_start - position;
    let room_after = parent_start + parent_size - (position + size);
    let mut shift = 0.0_f32;
    if let Some(inset) = props.number(end).filter(|value| value.is_finite()) {
        let limit = view_start + scrolled + view_size - inset;
        shift = shift
            .min(limit - (position + size))
            .max(room_before.min(0.0));
    }
    if let Some(inset) = props.number(start).filter(|value| value.is_finite()) {
        let limit = view_start + scrolled + inset;
        shift = shift.max(limit - position).min(room_after.max(0.0));
    }
    if horizontal {
        Point::new(shift, 0.0)
    } else {
        Point::new(0.0, shift)
    }
}

/// Moves a sticky box by `shift` without a new layout; a moved box draws
/// above the content it covers and takes its clicks.
fn pin(ctx: &mut Ctx<'_>, id: NodeId, shift: Point) {
    ctx.scene.set_layer_override(
        id,
        LayerSlot::Presenter,
        LayerOverride {
            transform: Affine::translation(shift.x, shift.y),
            ..LayerOverride::NONE
        },
    );
    let overlay = (shift.x != 0.0 || shift.y != 0.0).then(|| {
        let mut overlay = PropMap::new();
        overlay.insert(PropKey::ZIndex, PropValue::Integer(1_000_000));
        overlay
    });
    ctx.scene
        .set_style_overlay(id, StyleLayer::Presenter, overlay);
}

fn node_children_of(scene: &Scene, id: NodeId) -> Vec<NodeId> {
    scene
        .node(id)
        .map(|node| node.children.clone())
        .unwrap_or_default()
}

/// Viewport length, content length and largest offset on the scroll axis.
fn extents(scene: &Scene, id: NodeId, horizontal: bool) -> (f32, f32, f32) {
    let Some(node) = scene.node(id) else {
        return (0.0, 0.0, 0.0);
    };
    let content = scene.content_size(id);
    let (viewport, content) = if horizontal {
        (node.frame.width, content.width)
    } else {
        (node.frame.height, content.height)
    };
    (viewport, content, (content - viewport).max(0.0))
}

/// Whether a list shows its rows from the visual end.
fn is_inverted(scene: &Scene, id: NodeId) -> bool {
    scene
        .node(id)
        .is_some_and(|node| node.props().flag_or(PropKey::ListInverted, false))
}

/// Converts between the logical offset of an inverted list and the native
/// one (the mapping is its own inverse).
fn mirrored(scene: &Scene, id: NodeId, horizontal: bool, offset: f32) -> f32 {
    if is_inverted(scene, id) {
        (extents(scene, id, horizontal).2 - offset).max(0.0)
    } else {
        offset
    }
}

impl Scrolling {
    /// Native backends report wheel input only when no native scroller used it.
    /// Resolve that input against PAM's pointer-events tree, so transparent
    /// portal layers cannot hide a scroll viewport from native hit testing.
    pub fn on_wheel(&mut self, ctx: &mut Ctx<'_>, position: Point, delta: Point) -> bool {
        if !delta.x.is_finite() || !delta.y.is_finite() {
            return false;
        }
        let mut current = ctx.scene.hit_test(position).unwrap_or(0);
        while let Some(node) = ctx.scene.node(current) {
            let parent = node.parent;
            if node.class == ViewClass::Scroll
                && node.props().flag_or(PropKey::ScrollEnabled, true)
                && node.props().flag_or(PropKey::ScrollBothAxes, false)
            {
                // `overflow: auto`: the wheel moves both axes of one viewport.
                let offset = ctx.scene.scroll_offset(current);
                let target = Point::new(
                    (offset.x + delta.x).clamp(0.0, extents(ctx.scene, current, true).2),
                    (offset.y + delta.y).clamp(0.0, extents(ctx.scene, current, false).2),
                );
                if (target.x - offset.x).abs() > 0.01 || (target.y - offset.y).abs() > 0.01 {
                    ctx.animator.cancel(Owner::Scroll(current), ctx.now_ms);
                    ctx.commands.push(HostCommand::ScrollTo {
                        id: current,
                        offset: target,
                        animated: false,
                    });
                    return true;
                }
            } else if node.class == ViewClass::Scroll
                && node.props().flag_or(PropKey::ScrollEnabled, true)
            {
                let horizontal = node.props().flag_or(PropKey::ScrollHorizontal, false)
                    || node.props().flag_or(PropKey::ListHorizontal, false);
                let amount = axis(delta, horizontal);
                let offset = ctx.scene.scroll_offset(current);
                let before = axis(offset, horizontal);
                let after = (before + amount).clamp(0.0, extents(ctx.scene, current, horizontal).2);
                if (after - before).abs() > 0.01 {
                    ctx.animator.cancel(Owner::Scroll(current), ctx.now_ms);
                    ctx.commands.push(HostCommand::ScrollTo {
                        id: current,
                        offset: if horizontal {
                            Point::new(after, offset.y)
                        } else {
                            Point::new(offset.x, after)
                        },
                        animated: false,
                    });
                    return true;
                }
            }
            current = parent;
        }
        false
    }

    pub fn on_change(&mut self, ctx: &mut Ctx<'_>, change: Change) {
        match change {
            Change::Created(id, kind) => {
                if is_scrollable(kind) {
                    self.scrolls.insert(
                        id,
                        Scroll {
                            end_armed: true,
                            sync: true,
                            request_changed: true,
                            offset_changed: true,
                            items_changed: true,
                            ..Scroll::default()
                        },
                    );
                    self.dirty = true;
                } else if kind == NodeKind::RefreshControl {
                    self.refreshes.insert(
                        id,
                        Refresh {
                            sync: true,
                            ..Refresh::default()
                        },
                    );
                    self.dirty = true;
                }
                self.watch(ctx, id);
            }
            Change::Removed(id, _) => {
                self.scrolls.remove(&id);
                self.refreshes.remove(&id);
                self.sticky.remove(&id);
                self.layout_listeners.remove(&id);
                ctx.timers.cancel(Timer::ScrollRest(id));
                ctx.timers.cancel(Timer::ListWindow(id));
                ctx.timers.cancel(Timer::RefreshFallback(id));
            }
            Change::Updated(id, key) => {
                if matches!(
                    key,
                    PropKey::StickyHeader | PropKey::PositionType | PropKey::OnLayout
                ) {
                    self.watch(ctx, id);
                }
                if let Some(scroll) = self.scrolls.get_mut(&id) {
                    match key {
                        PropKey::ScrollRequest => scroll.request_changed = true,
                        PropKey::ScrollContentOffsetX | PropKey::ScrollContentOffsetY => {
                            scroll.offset_changed = true
                        }
                        PropKey::Items
                        | PropKey::SectionItems
                        | PropKey::ListRowHeight
                        | PropKey::TextColor => scroll.items_changed = true,
                        _ => {}
                    }
                    scroll.sync = true;
                    self.dirty = true;
                } else if let Some(refresh) = self.refreshes.get_mut(&id) {
                    refresh.sync = true;
                    self.dirty = true;
                }
            }
            Change::Frame(id) => {
                if self.layout_listeners.contains_key(&id) {
                    self.layout_dirty = true;
                    self.dirty = true;
                }
                // The view, a row or anything inside its content moved.
                let scroll = if self.scrolls.contains_key(&id) {
                    Some(id)
                } else {
                    nearest_scroll(ctx.scene, id)
                };
                if let Some(scroll) = scroll.and_then(|scroll| self.scrolls.get_mut(&scroll)) {
                    scroll.sync = true;
                    self.dirty = true;
                }
                if let Some(refresh) = self.refreshes.get_mut(&id) {
                    refresh.sync = true;
                    self.dirty = true;
                }
            }
            Change::Children(id) => {
                if let Some(scroll) = self.scrolls.get_mut(&id) {
                    scroll.sync = true;
                    self.dirty = true;
                }
            }
            Change::Motion { .. } | Change::Style { .. } => {}
        }
    }

    /// Registers sticky headers and layout listeners.
    fn watch(&mut self, ctx: &Ctx<'_>, id: NodeId) {
        let Some(node) = ctx.scene.node(id) else {
            return;
        };
        if node.is_synthetic() {
            return;
        }
        if node.props().flag_or(PropKey::StickyHeader, false) || is_css_sticky(node.props()) {
            if let Some(scroll) = nearest_scroll(ctx.scene, id) {
                self.sticky.insert(id, scroll);
                if let Some(state) = self.scrolls.get_mut(&scroll) {
                    state.sync = true;
                }
                self.dirty = true;
            }
        } else if self.sticky.remove(&id).is_some() {
            self.dirty = true;
        }
        if node.handles(PropKey::OnLayout) {
            self.layout_listeners.entry(id).or_insert(None);
            self.layout_dirty = true;
            self.dirty = true;
        } else {
            self.layout_listeners.remove(&id);
        }
    }

    pub fn after_commit(&mut self, ctx: &mut Ctx<'_>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let ids: Vec<NodeId> = self
            .scrolls
            .iter()
            .filter(|(_, scroll)| scroll.sync)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.sync(ctx, id);
        }
        let refreshes: Vec<NodeId> = self
            .refreshes
            .iter()
            .filter(|(_, refresh)| refresh.sync)
            .map(|(id, _)| *id)
            .collect();
        for id in refreshes {
            self.sync_refresh(ctx, id);
        }
        if std::mem::take(&mut self.layout_dirty) {
            self.report_layouts(ctx);
        }
    }

    /// `Layout`: the frame relative to the parent, on mount and on change.
    fn report_layouts(&mut self, ctx: &mut Ctx<'_>) {
        let ids: Vec<NodeId> = self.layout_listeners.keys().copied().collect();
        for id in ids {
            let Some(node) = ctx.scene.node(id) else {
                continue;
            };
            let origin = ctx
                .scene
                .node(node.parent)
                .map_or(Point::ZERO, |parent| parent.frame.origin());
            let frame = Rect::new(
                node.frame.x - origin.x,
                node.frame.y - origin.y,
                node.frame.width,
                node.frame.height,
            );
            if self.layout_listeners.get(&id).copied().flatten() == Some(frame) {
                continue;
            }
            self.layout_listeners.insert(id, Some(frame));
            let payload = WireMap::new()
                .with("x", frame.x)
                .with("y", frame.y)
                .with("width", frame.width)
                .with("height", frame.height)
                .encode();
            ctx.emit(id, event_kind::LAYOUT, payload);
        }
    }

    fn scroll_to(
        ctx: &mut Ctx<'_>,
        id: NodeId,
        horizontal: bool,
        target: f32,
        animated: bool,
        duration: u32,
        easing: Easing,
    ) {
        let current = mirrored(
            ctx.scene,
            id,
            horizontal,
            axis(ctx.scene.scroll_offset(id), horizontal),
        );
        if animated && !ctx.reduced_motion && (current - target).abs() > 0.5 {
            ctx.animator.start(
                Owner::Scroll(id),
                current,
                target,
                Curve::timing(duration.max(1), easing),
                0,
                ctx.now_ms,
            );
        } else {
            ctx.animator.cancel(Owner::Scroll(id), ctx.now_ms);
            Self::jump(ctx, id, horizontal, target);
        }
    }

    fn jump(ctx: &mut Ctx<'_>, id: NodeId, horizontal: bool, target: f32) {
        let target = mirrored(ctx.scene, id, horizontal, target);
        let offset = if horizontal {
            Point::new(target, 0.0)
        } else {
            Point::new(0.0, target)
        };
        ctx.commands.push(HostCommand::ScrollTo {
            id,
            offset,
            animated: false,
        });
    }

    /// Frame-clock sample of a gliding offset.
    pub fn on_sample(&mut self, ctx: &mut Ctx<'_>, id: NodeId, value: f32) {
        if let Some(scroll) = self.scrolls.get(&id) {
            Self::jump(ctx, id, scroll.horizontal, value);
        }
    }

    /// Re-evaluates a scroll view after its content, frame or props changed.
    fn sync(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(node) = ctx.scene.node(id) else {
            return;
        };
        let props = node.props();
        let kind = node.kind;
        let horizontal = props.flag_or(PropKey::ScrollHorizontal, false)
            || props.flag_or(PropKey::ListHorizontal, false);
        let anchor_to_end = props.flag_or(PropKey::ScrollAnchorToEnd, false);
        let maintain = props.flag_or(PropKey::ScrollMaintainVisibleContentPosition, false);
        let anchor_threshold = props
            .number_or(PropKey::ScrollAutoScrollToEndThreshold, 24.0)
            .max(0.0);
        let follow_end =
            props.flag_or(PropKey::ListFollowEndOnResize, true) && kind == NodeKind::VirtualList;
        let stack_from_end = props.flag_or(PropKey::ListStackFromEnd, false);
        let initial_index = props.integer_or(PropKey::ListInitialScrollIndex, 0);
        let request_token = props.integer(PropKey::ScrollRequest);
        let request_test_id = props.text_or_empty(PropKey::ScrollTargetTestId).to_owned();
        let request_offset = props.number_or(PropKey::ScrollTargetOffset, -1.0);
        let alignment = props.integer_or(PropKey::ScrollTargetAlignment, 1);
        let animated = props.flag_or(PropKey::ScrollAnimated, false);
        let content_offset = (
            props.number(PropKey::ScrollContentOffsetX),
            props.number(PropKey::ScrollContentOffsetY),
        );
        let glide_ms = props
            .integer_or(PropKey::ScrollOffsetAnimationMs, 0)
            .clamp(0, 2000) as u32;
        let active_section = props.text_or_empty(PropKey::ListActiveSection).to_owned();
        let frame = node.frame;
        let inverted = props.flag_or(PropKey::ListInverted, false) && kind != NodeKind::Scroll;
        let rows: Vec<NodeId> = if inverted {
            node.children.clone()
        } else {
            Vec::new()
        };

        let Some(scroll) = self.scrolls.get_mut(&id) else {
            return;
        };
        scroll.sync = false;
        scroll.horizontal = horizontal;
        if scroll.inverted && !inverted {
            for row in &node_children_of(ctx.scene, id) {
                ctx.scene
                    .set_layer_override(*row, LayerSlot::Presenter, LayerOverride::NONE);
            }
        }
        scroll.inverted = inverted;
        if std::mem::take(&mut scroll.items_changed)
            && matches!(kind, NodeKind::List | NodeKind::SectionList)
        {
            Self::build_string_rows(ctx, id, scroll);
            scroll.end_armed = true;
        }
        if kind == NodeKind::VirtualList {
            Self::apply_sections(ctx, id, scroll, &active_section);
        }

        let (viewport, content, max) = extents(ctx.scene, id, horizontal);
        let previous_max = scroll.max;
        let offset_before = axis(scroll.offset, horizontal);
        scroll.max = max;
        let mut target: Option<(f32, bool, u32, Easing)> = None;

        if inverted {
            // Rows are laid out from the start by the engine and shown from
            // the end: each one moves to its mirrored place.
            let extent = content.max(viewport);
            for row in rows {
                let Some(child) = ctx.scene.node(row) else {
                    continue;
                };
                let shift = if horizontal {
                    Affine::translation(
                        extent - 2.0 * (child.frame.x - frame.x) - child.frame.width,
                        0.0,
                    )
                } else {
                    Affine::translation(
                        0.0,
                        extent - 2.0 * (child.frame.y - frame.y) - child.frame.height,
                    )
                };
                ctx.scene.set_layer_override(
                    row,
                    LayerSlot::Presenter,
                    LayerOverride {
                        transform: shift,
                        ..LayerOverride::NONE
                    },
                );
            }
            // The logical offset holds while content is added at the far end.
            if (max != previous_max || !scroll.inverted_placed) && scroll.gesture.is_none() {
                scroll.inverted_placed = true;
                Self::jump(ctx, id, horizontal, offset_before.min(max));
            }
        }

        // 1. Where the view stays when its content grows or shrinks.
        if inverted {
            // Anchoring and following the end are not for inverted lists.
        } else if kind == NodeKind::Scroll {
            let distance = (previous_max - offset_before).max(0.0);
            if anchor_to_end && !scroll.anchored_to_end {
                scroll.anchored_to_end = true;
                target = Some((max, false, 0, Easing::Linear));
            } else if anchor_to_end && max != previous_max && distance <= anchor_threshold {
                target = Some((max, false, 0, Easing::Linear));
            } else if maintain && max > previous_max {
                target = Some((
                    offset_before + (max - previous_max),
                    false,
                    0,
                    Easing::Linear,
                ));
            }
            if !anchor_to_end {
                scroll.anchored_to_end = false;
            }
        } else if (follow_end || stack_from_end) && scroll.gesture.is_none() && max != previous_max
        {
            // A list resting at its end stays there.
            let rested_at_end = scroll.at_end || (stack_from_end && previous_max <= 0.0);
            if rested_at_end && offset_before >= previous_max - 1.0 && max > 0.0 {
                target = Some((max, false, 0, Easing::Linear));
            }
        }

        // 2. `contentOffset`: applied once its content can reach it.
        if std::mem::take(&mut scroll.offset_changed) {
            scroll.pending_offset = match content_offset {
                (None, None) => None,
                (x, y) => Some(Point::new(
                    x.unwrap_or(0.0).max(0.0),
                    y.unwrap_or(0.0).max(0.0),
                )),
            };
        }
        if let Some(pending) = scroll.pending_offset {
            let wanted = axis(pending, horizontal);
            if wanted <= max || content > viewport {
                scroll.pending_offset = None;
                let glides = glide_ms > 0 && scroll.offset_applied_once;
                scroll.offset_applied_once = true;
                target = Some((wanted.min(max), glides, glide_ms, Easing::EASE_IN_OUT));
            }
        }

        // 3. The initial row of a list, once it has rows.
        if kind == NodeKind::VirtualList
            && initial_index > 0
            && scroll.initial_index != Some(initial_index)
        {
            let starts = Self::row_starts(ctx.scene, id, horizontal);
            if !starts.is_empty() {
                scroll.initial_index = Some(initial_index);
                scroll.at_end = true;
                let row = starts
                    .iter()
                    .find(|(index, rows, _, _)| {
                        initial_index >= i64::from(*index)
                            && initial_index < i64::from(*index) + i64::from(*rows)
                    })
                    .or(starts.last());
                if let Some((_, _, start, _)) = row {
                    target = Some((start.min(max), false, 0, Easing::Linear));
                }
            }
        }

        // 4. A scroll request (a new token) wins over everything above.
        if std::mem::take(&mut scroll.request_changed)
            && request_token.is_some()
            && scroll.request != request_token
        {
            scroll.request = request_token;
            let origin = axis(frame.origin(), horizontal);
            let aligned = |start: f32, extent: f32| {
                let adjust = match alignment {
                    2 => ((viewport - extent) / 2.0).max(0.0),
                    3 => (viewport - extent).max(0.0),
                    _ => 0.0,
                };
                (start - adjust).clamp(0.0, max)
            };
            let wanted = if !request_test_id.is_empty() {
                Self::find_test_id(ctx.scene, id, &request_test_id)
                    .and_then(|target| ctx.scene.node(target))
                    .map(|target| {
                        let (start, extent) = if horizontal {
                            (target.frame.x, target.frame.width)
                        } else {
                            (target.frame.y, target.frame.height)
                        };
                        aligned(start - origin, extent)
                    })
            } else if request_offset >= 0.0 {
                Some(aligned(request_offset, 0.0))
            } else if kind == NodeKind::Scroll {
                Some(max)
            } else {
                None
            };
            if let Some(wanted) = wanted {
                target = Some((wanted, animated, REQUEST_GLIDE_MS, Easing::EASE_OUT));
            }
        }

        if let Some((wanted, glides, duration, easing)) = target
            && (wanted - offset_before).abs() > 0.01
        {
            Self::scroll_to(ctx, id, horizontal, wanted, glides, duration, easing);
        }
        self.after_offset(ctx, id);
    }

    /// `(row index, rows covered, start, extent)` of every shown cell of a list, in order.
    fn row_starts(scene: &Scene, id: NodeId, horizontal: bool) -> Vec<(u32, u32, f32, f32)> {
        let Some(node) = scene.node(id) else {
            return Vec::new();
        };
        let origin = axis(node.frame.origin(), horizontal);
        let mut rows: Vec<(u32, u32, f32, f32)> = node
            .children
            .iter()
            .filter_map(|child| scene.node(*child))
            .filter(|child| !child.is_synthetic() && child.layer.visible)
            .map(|child| {
                let props = child.props();
                let covered = if props.flag_or(PropKey::ListPlaceholder, false) {
                    props
                        .integer_or(PropKey::ListPlaceholderRows, 1)
                        .clamp(1, i64::from(u32::MAX)) as u32
                } else {
                    1
                };
                let (start, extent) = if horizontal {
                    (child.frame.x, child.frame.width)
                } else {
                    (child.frame.y, child.frame.height)
                };
                (child.index, covered, start - origin, extent)
            })
            .collect();
        rows.sort_by_key(|left| left.0);
        rows
    }

    fn find_test_id(scene: &Scene, root: NodeId, test_id: &str) -> Option<NodeId> {
        let mut queue = std::collections::VecDeque::from([root]);
        while let Some(id) = queue.pop_front() {
            let node = scene.node(id)?;
            if id != root && node.props().text(PropKey::TestId) == Some(test_id) {
                return Some(id);
            }
            queue.extend(node.children.iter().copied());
        }
        None
    }

    /// Rows of the active section are shown; the others stay mounted but hidden.
    fn apply_sections(ctx: &mut Ctx<'_>, id: NodeId, scroll: &mut Scroll, active: &str) {
        let Some(node) = ctx.scene.node(id) else {
            return;
        };
        let children = node.children.clone();
        let mut sectioned = false;
        for child in children {
            let Some(section) = ctx
                .scene
                .node(child)
                .map(|child| child.props().text(PropKey::ListSection).map(str::to_owned))
            else {
                continue;
            };
            let Some(section) = section else { continue };
            sectioned = true;
            let layer = if section == active {
                LayerOverride::NONE
            } else {
                LayerOverride::HIDDEN
            };
            ctx.scene
                .set_layer_override(child, LayerSlot::Presenter, layer);
        }
        if sectioned && scroll.active_section != active {
            // Each section remembers where it was left.
            let offset = axis(scroll.offset, scroll.horizontal);
            let previous = std::mem::replace(&mut scroll.active_section, active.to_owned());
            scroll
                .section_offsets
                .retain(|(section, _)| *section != previous);
            scroll.section_offsets.push((previous, offset));
            if let Some((_, saved)) = scroll
                .section_offsets
                .iter()
                .find(|(section, _)| section == active)
            {
                Self::jump(ctx, id, scroll.horizontal, *saved);
            }
        }
    }

    /// `List` and `SectionList` carry only strings: the host draws one text row each.
    fn build_string_rows(ctx: &mut Ctx<'_>, id: NodeId, scroll: &mut Scroll) {
        for row in scroll.rows.drain(..) {
            ctx.scene.remove_synthetic(row);
        }
        let Some(node) = ctx.scene.node(id) else {
            return;
        };
        let props = node.props();
        let frame = node.frame;
        let row_height = props
            .number_or(PropKey::ListRowHeight, STRING_ROW_HEIGHT)
            .max(1.0);
        let color = props.integer(PropKey::TextColor);
        let mut entries: Vec<(String, bool)> = Vec::new();
        if node.kind == NodeKind::SectionList {
            if let Some(bytes) = props.bytes(PropKey::SectionItems) {
                entries = decode_sections(bytes).unwrap_or_default();
            }
        } else if let Some(items) = props.bytes(PropKey::Items).and_then(decode_string_list) {
            entries = items.into_iter().map(|item| (item, false)).collect();
        }
        for (position, (text, header)) in entries.into_iter().enumerate() {
            let mut row = PropMap::new();
            row.insert(PropKey::Text, PropValue::String(text));
            row.insert(PropKey::PaddingHorizontal, PropValue::Integer(20));
            row.insert(PropKey::NumberOfLines, PropValue::Integer(1));
            row.insert(PropKey::PointerEvents, PropValue::Integer(2));
            if header {
                row.insert(PropKey::FontSize, PropValue::Integer(13));
                row.insert(PropKey::FontWeight, PropValue::Integer(700));
                row.insert(PropKey::LetterSpacing, PropValue::Float(0.455));
                row.insert(PropKey::TextColor, PropValue::Integer(0xFF00_6C49));
                row.insert(PropKey::BackgroundColor, PropValue::Integer(0x1800_875A));
            } else {
                row.insert(PropKey::FontSize, PropValue::Integer(16));
                if let Some(color) = color {
                    row.insert(PropKey::TextColor, PropValue::Integer(color));
                }
            }
            let row_frame = Rect::new(
                frame.x,
                frame.y + position as f32 * row_height,
                frame.width,
                row_height,
            );
            scroll.rows.push(ctx.scene.add_synthetic(
                id,
                NodeKind::Text,
                row,
                row_frame,
                position as u32,
                None,
            ));
        }
    }

    // ---- offsets ---------------------------------------------------------

    /// The native viewport moved.
    pub fn on_scroll(&mut self, ctx: &mut Ctx<'_>, id: NodeId, offset: Point) {
        ctx.scene.set_scroll_offset(id, offset);
        let Some(scroll) = self.scrolls.get_mut(&id) else {
            return;
        };
        // An inverted list reports and reasons in logical offsets.
        let offset = if scroll.inverted {
            let logical = (scroll.max - axis(offset, scroll.horizontal)).max(0.0);
            if scroll.horizontal {
                Point::new(logical, 0.0)
            } else {
                Point::new(0.0, logical)
            }
        } else {
            offset
        };
        let position = axis(offset, scroll.horizontal);
        let previous = axis(scroll.offset, scroll.horizontal);
        scroll.offset = offset;
        if (position - previous).abs() < f32::EPSILON {
            return;
        }
        scroll.report = true;
        // Offsets the core asked for are not user gestures.
        let programmatic = ctx.animator.is_running(Owner::Scroll(id));
        if !programmatic {
            if let Some((at, last)) = scroll.last_step {
                let elapsed = ctx.now_ms.saturating_sub(at).max(1) as f32 / 1000.0;
                scroll.velocity = (position - last) / elapsed;
            }
            scroll.last_step = Some((ctx.now_ms, position));
            if scroll.gesture.is_none() {
                scroll.gesture = Some(previous);
                let payload = Self::phase_payload(ctx.scene, id, scroll, 0.0);
                ctx.emit_if(
                    id,
                    PropKey::OnScrollBeginDrag,
                    event_kind::SCROLL_BEGIN_DRAG,
                    payload,
                );
            }
            ctx.timers
                .schedule(Timer::ScrollRest(id), ctx.now_ms + REST_MS);
        }
        self.after_offset(ctx, id);
    }

    /// Everything that depends on the offset: sticky headers, end reached,
    /// pages and the list window.
    fn after_offset(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        self.pin_sticky(ctx, id);
        let Some(node) = ctx.scene.node(id) else {
            return;
        };
        let props = node.props();
        let threshold = props
            .number_or(PropKey::EndReachedThreshold, 0.5)
            .clamp(0.0, 10.0);
        let paging = props.flag_or(PropKey::ScrollPagingEnabled, false);
        let snap = props.number_or(PropKey::ScrollSnapInterval, 0.0);
        let windowed = props.flag_or(PropKey::ListWindowed, false);
        let Some(scroll) = self.scrolls.get_mut(&id) else {
            return;
        };
        let (viewport, content, max) = extents(ctx.scene, id, scroll.horizontal);
        let offset = axis(scroll.offset, scroll.horizontal);
        scroll.at_end = max > 0.0 && offset >= max - 1.0;

        // EndReached fires once and re-arms when the content or viewport changes.
        if scroll.end_content != content || scroll.end_viewport != viewport {
            scroll.end_content = content;
            scroll.end_viewport = viewport;
            scroll.end_armed = true;
        }
        if scroll.end_armed
            && content > 0.0
            && viewport > 0.0
            && content - (offset + viewport) <= viewport * threshold
        {
            scroll.end_armed = false;
            ctx.emit_if(
                id,
                PropKey::OnEndReached,
                event_kind::END_REACHED,
                Vec::new(),
            );
        }

        let extent = if snap > 0.0 { snap } else { viewport };
        let page = if extent > 0.0 {
            ((offset / extent).round() as i64).max(0)
        } else {
            0
        };
        let _ = paging;
        match scroll.page {
            Some(previous) if previous != page => {
                scroll.page = Some(page);
                let payload = WireMap::new()
                    .with("page", page)
                    .with("previousPage", previous)
                    .encode();
                ctx.emit_if(id, PropKey::OnPageChange, event_kind::PAGE_CHANGE, payload);
            }
            // The first observation only records the page.
            None => scroll.page = Some(page),
            _ => {}
        }
        if windowed {
            self.report_window(ctx, id);
        }
    }

    fn phase_payload(scene: &Scene, id: NodeId, scroll: &Scroll, velocity: f32) -> Vec<u8> {
        let size = scene
            .node(id)
            .map(|node| node.frame.size())
            .unwrap_or_default();
        let content = scene.content_size(id);
        let (velocity_x, velocity_y) = if scroll.horizontal {
            (velocity, 0.0)
        } else {
            (0.0, velocity)
        };
        WireMap::new()
            .with("x", scroll.offset.x)
            .with("y", scroll.offset.y)
            .with("velocityX", velocity_x)
            .with("velocityY", velocity_y)
            .with("page", scroll.page.unwrap_or(0))
            .with("contentWidth", content.width.max(size.width))
            .with("contentHeight", content.height.max(size.height))
            .with("layoutWidth", size.width)
            .with("layoutHeight", size.height)
            .encode()
    }

    /// Sticky headers pin to the top of their viewport and are pushed out by
    /// the next one; CSS `position: sticky` boxes follow their insets.
    fn pin_sticky(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(scroll) = self.scrolls.get(&id) else {
            return;
        };
        let (horizontal, offset) = (scroll.horizontal, scroll.offset);
        let Some(viewport) = ctx.scene.node(id).map(|node| node.frame) else {
            return;
        };
        let css: Vec<NodeId> = self
            .sticky
            .iter()
            .filter(|(node, owner)| {
                **owner == id
                    && ctx
                        .scene
                        .node(**node)
                        .is_some_and(|node| is_css_sticky(node.props()))
            })
            .map(|(node, _)| *node)
            .collect();
        for node in css {
            let shift = css_sticky_shift(ctx.scene, node, viewport, offset, horizontal);
            pin(ctx, node, shift);
        }
        if horizontal {
            return;
        }
        let origin = viewport.y;
        let mut headers: Vec<(NodeId, f32, f32)> = self
            .sticky
            .iter()
            .filter(|(header, owner)| {
                **owner == id
                    && ctx
                        .scene
                        .node(**header)
                        .is_some_and(|node| !is_css_sticky(node.props()))
            })
            .filter_map(|(header, _)| {
                ctx.scene
                    .node(*header)
                    .map(|node| (*header, node.frame.y - origin, node.frame.height))
            })
            .collect();
        if headers.is_empty() {
            return;
        }
        headers.sort_by(|left, right| left.1.total_cmp(&right.1));
        let offset = scroll.offset.y;
        for position in 0..headers.len() {
            let (header, top, height) = headers[position];
            let next = headers
                .get(position + 1)
                .map_or(f32::INFINITY, |next| next.1);
            let shift = if offset <= top {
                0.0
            } else {
                (offset - top).min((next - top - height).max(0.0))
            };
            pin(ctx, header, Point::new(0.0, shift));
        }
    }

    /// Windowed lists build rows on demand: tell PHP which rows are on screen
    /// whenever an unbuilt one comes near.
    fn report_window(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(scroll) = self.scrolls.get_mut(&id) else {
            return;
        };
        let rows = Self::row_starts(ctx.scene, id, scroll.horizontal);
        if rows.is_empty() {
            return;
        }
        let (viewport, _, _) = extents(ctx.scene, id, scroll.horizontal);
        let offset = axis(scroll.offset, scroll.horizontal);
        let overscan = viewport * 1.5;
        let Some(node) = ctx.scene.node(id) else {
            return;
        };
        let placeholder_near = node
            .children
            .iter()
            .filter_map(|child| ctx.scene.node(*child))
            .any(|child| {
                if !child.props().flag_or(PropKey::ListPlaceholder, false) {
                    return false;
                }
                let start = child.frame.y - node.frame.y;
                start < offset + viewport + overscan
                    && start + child.frame.height > offset - overscan
            });
        if !placeholder_near {
            ctx.timers.cancel(Timer::ListWindow(id));
            return;
        }
        let row_at = |position: f32| {
            rows.iter()
                .find(|(_, _, start, extent)| position < start + extent)
                .or(rows.last())
                .map_or(0, |(index, covered, start, extent)| {
                    let into = (position - start).max(0.0);
                    let each = extent / *covered as f32;
                    let inside = if each > 0.0 {
                        ((into / each).floor() as u32).min(covered - 1)
                    } else {
                        0
                    };
                    index + inside
                })
        };
        let first = row_at(offset);
        let last = row_at(offset + viewport - 0.5).max(first);
        let total: u32 = rows.iter().map(|(_, covered, _, _)| *covered).sum();
        let payload = format!("@{first},{last},{total}");
        let repeated = scroll.window_report.as_ref().is_some_and(|(sent, at)| {
            *sent == payload && ctx.now_ms.saturating_sub(*at) < WINDOW_REPEAT_MS
        });
        if !repeated {
            scroll.window_report = Some((payload.clone(), ctx.now_ms));
            ctx.emit(id, event_kind::LIST_WINDOW, payload.into_bytes());
        }
        // Ask again if PHP has not built the rows by then.
        ctx.timers
            .schedule(Timer::ListWindow(id), ctx.now_ms + WINDOW_REPEAT_MS);
    }

    pub fn on_timer(&mut self, ctx: &mut Ctx<'_>, timer: Timer) {
        match timer {
            Timer::ScrollRest(id) => {
                let Some(scroll) = self.scrolls.get_mut(&id) else {
                    return;
                };
                if scroll.gesture.take().is_none() {
                    return;
                }
                let velocity = std::mem::take(&mut scroll.velocity);
                scroll.last_step = None;
                let released = Self::phase_payload(ctx.scene, id, scroll, velocity);
                let rested = Self::phase_payload(ctx.scene, id, scroll, 0.0);
                ctx.emit_if(
                    id,
                    PropKey::OnScrollEndDrag,
                    event_kind::SCROLL_END_DRAG,
                    released,
                );
                ctx.emit_if(
                    id,
                    PropKey::OnMomentumScrollEnd,
                    event_kind::MOMENTUM_SCROLL_END,
                    rested,
                );
            }
            Timer::ListWindow(id) => self.report_window(ctx, id),
            Timer::RefreshFallback(id) => {
                if let Some(refresh) = self.refreshes.get_mut(&id) {
                    refresh.requested = false;
                    refresh.sync = true;
                    self.dirty = true;
                    self.sync_refresh(ctx, id);
                }
            }
            _ => {}
        }
    }

    /// Sends the scroll offsets coalesced during this drain.
    pub fn flush(&mut self, ctx: &mut Ctx<'_>) {
        let reports: Vec<NodeId> = self
            .scrolls
            .iter()
            .filter(|(_, scroll)| scroll.report)
            .map(|(id, _)| *id)
            .collect();
        for id in reports {
            let Some(scroll) = self.scrolls.get_mut(&id) else {
                continue;
            };
            scroll.report = false;
            let offset = axis(scroll.offset, scroll.horizontal);
            ctx.emit_if(
                id,
                PropKey::OnScroll,
                event_kind::SCROLL,
                format!("{offset}").into_bytes(),
            );
        }
    }

    // ---- refresh ---------------------------------------------------------

    /// The spinner of a refresh host: shown while PHP says it is refreshing,
    /// or right after the user asked for a refresh.
    fn sync_refresh(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(node) = ctx.scene.node(id) else {
            return;
        };
        let props = node.props();
        let frame = node.frame;
        let refreshing = props.flag_or(PropKey::Refreshing, false);
        let offset = props.number_or(PropKey::RefreshProgressViewOffset, 0.0);
        // The tint, else the first of the refresh colours, else a neutral grey.
        let first_color = props
            .text_or_empty(PropKey::RefreshColors)
            .split(',')
            .next()
            .and_then(|color| color.trim().parse::<i64>().ok());
        let color = props
            .integer(PropKey::TintColor)
            .or(first_color)
            .unwrap_or(0xFF8E_8E93);
        let background = props.integer(PropKey::RefreshProgressBackgroundColor);
        let large = props.integer(PropKey::RefreshIndicatorSize) == Some(2);
        let Some(refresh) = self.refreshes.get_mut(&id) else {
            return;
        };
        refresh.sync = false;
        if refreshing {
            refresh.requested = false;
            ctx.timers.cancel(Timer::RefreshFallback(id));
        }
        let shown = refreshing || refresh.requested;
        if !shown {
            for node in [refresh.indicator.take(), refresh.backdrop.take()]
                .into_iter()
                .flatten()
            {
                ctx.scene.remove_synthetic(node);
            }
            return;
        }
        let size = if large { 36.0 } else { 28.0 };
        let padding = if background.is_some() { 6.0 } else { 0.0 };
        let disc = Rect::new(
            frame.x + (frame.width - size) / 2.0 - padding,
            frame.y + 12.0 + offset,
            size + padding * 2.0,
            size + padding * 2.0,
        );
        match (background, refresh.backdrop) {
            (Some(background), existing) => {
                let backdrop = existing.unwrap_or_else(|| {
                    let mut props = PropMap::new();
                    props.insert(PropKey::PointerEvents, PropValue::Integer(2));
                    // A hairline keeps a disc of the list's own colour visible.
                    props.insert(PropKey::BorderWidth, PropValue::Float(1.0));
                    props.insert(PropKey::BorderColor, PropValue::Integer(0x1F00_0000));
                    ctx.scene
                        .add_synthetic(id, NodeKind::View, props, disc, 0, None)
                });
                ctx.scene.set_prop(
                    backdrop,
                    PropKey::BackgroundColor,
                    Some(PropValue::Integer(background)),
                );
                ctx.scene.set_prop(
                    backdrop,
                    PropKey::BorderRadius,
                    Some(PropValue::Float(f64::from(disc.width / 2.0))),
                );
                ctx.scene.set_synthetic_frame(backdrop, disc);
                refresh.backdrop = Some(backdrop);
            }
            (None, Some(backdrop)) => {
                ctx.scene.remove_synthetic(backdrop);
                refresh.backdrop = None;
            }
            (None, None) => {}
        }
        let indicator_frame = Rect::new(disc.x + padding, disc.y + padding, size, size);
        let indicator = refresh.indicator.unwrap_or_else(|| {
            let mut spinner = PropMap::new();
            spinner.insert(PropKey::PointerEvents, PropValue::Integer(2));
            ctx.scene.add_synthetic(
                id,
                NodeKind::ActivityIndicator,
                spinner,
                indicator_frame,
                1,
                None,
            )
        });
        ctx.scene.set_prop(
            indicator,
            PropKey::ProgressColor,
            Some(PropValue::Integer(color)),
        );
        ctx.scene.set_synthetic_frame(indicator, indicator_frame);
        refresh.indicator = Some(indicator);
    }

    /// F5 or Ctrl/Cmd+R refreshes the visible refresh host: desktops have no
    /// pull gesture. Returns whether the key was used.
    pub fn on_key(&mut self, ctx: &mut Ctx<'_>, key: &KeyEvent) -> bool {
        let chord = key.pressed
            && !key.repeat
            && (key.key == "F5"
                || (key.key.eq_ignore_ascii_case("r")
                    && (key.modifiers.control || key.modifiers.meta)
                    && !key.modifiers.alt));
        if !chord {
            return false;
        }
        let target = self.refreshes.keys().copied().find(|id| {
            ctx.scene.is_effectively_visible(*id)
                && ctx.scene.node(*id).is_some_and(|node| {
                    node.handles(PropKey::OnRefresh)
                        && node.props().flag_or(PropKey::Enabled, true)
                        && node.props().flag_or(PropKey::RefreshEnabled, true)
                })
        });
        let Some(id) = target else { return false };
        let Some(refresh) = self.refreshes.get_mut(&id) else {
            return false;
        };
        if refresh.requested
            || ctx
                .scene
                .node(id)
                .is_some_and(|node| node.props().flag_or(PropKey::Refreshing, false))
        {
            return true;
        }
        refresh.requested = true;
        ctx.emit(id, event_kind::REFRESH, Vec::new());
        // Without an answer the spinner returns to its controlled state.
        ctx.timers
            .schedule(Timer::RefreshFallback(id), ctx.now_ms + REFRESH_FALLBACK_MS);
        self.sync_refresh(ctx, id);
        true
    }
}

/// Packed sections: `u32 sections`, then per section a title and its items.
/// Flattened to `(text, is header)` entries.
fn decode_sections(bytes: &[u8]) -> Option<Vec<(String, bool)>> {
    let mut offset = 0;
    let read_u32 = |bytes: &[u8], offset: &mut usize| -> Option<u32> {
        let value = u32::from_le_bytes(bytes.get(*offset..*offset + 4)?.try_into().ok()?);
        *offset += 4;
        Some(value)
    };
    let read_text = |bytes: &[u8], offset: &mut usize, length: u32| -> Option<String> {
        let end = offset.checked_add(length as usize)?;
        let text = std::str::from_utf8(bytes.get(*offset..end)?)
            .ok()?
            .to_owned();
        *offset = end;
        Some(text)
    };
    let sections = read_u32(bytes, &mut offset)?;
    if sections > 10_000 {
        return None;
    }
    let mut entries = Vec::new();
    for _ in 0..sections {
        let length = read_u32(bytes, &mut offset)?;
        entries.push((read_text(bytes, &mut offset, length)?, true));
        let items = read_u32(bytes, &mut offset)?;
        if entries.len() + items as usize > 100_000 {
            return None;
        }
        for _ in 0..items {
            let length = read_u32(bytes, &mut offset)?;
            entries.push((read_text(bytes, &mut offset, length)?, false));
        }
    }
    (offset == bytes.len()).then_some(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_sticky_boxes_hold_their_inset_inside_their_containing_block() {
        use pam_desktop_wire::{Layout, Mutation, Node};
        let create =
            |id: u64, parent: u64, kind: NodeKind, properties: Vec<(PropKey, PropValue)>| {
                Mutation::Create(Node {
                    id,
                    parent,
                    index: 0,
                    kind,
                    properties: properties.into_iter().collect(),
                })
            };
        let layout = |id: u64, y: f32, height: f32| Mutation::Layout {
            id,
            frame: Layout {
                x: 0.0,
                y,
                width: 400.0,
                height,
            },
        };
        let mut scene = Scene::new();
        scene.apply(vec![
            Mutation::SetRoot { id: 1 },
            create(1, 0, NodeKind::Scroll, vec![]),
            create(2, 1, NodeKind::Column, vec![]),
            create(
                3,
                2,
                NodeKind::Row,
                vec![
                    (PropKey::PositionType, PropValue::Integer(4)),
                    (PropKey::Top, PropValue::Float(0.0)),
                ],
            ),
            create(
                4,
                2,
                NodeKind::Row,
                vec![
                    (PropKey::PositionType, PropValue::Integer(4)),
                    (PropKey::Bottom, PropValue::Float(10.0)),
                ],
            ),
            layout(1, 50.0, 300.0),
            layout(2, 50.0, 1000.0),
            layout(3, 50.0, 37.0),
            layout(4, 950.0, 40.0),
        ]);
        let viewport = Rect::new(0.0, 50.0, 400.0, 300.0);
        let shift = |id, y| css_sticky_shift(&scene, id, viewport, Point::new(0.0, y), false).y;
        // Top: in flow until scrolled past, then held at the top edge, and
        // pushed out at the end of its parent (1000 - 37).
        assert_eq!(shift(3, 0.0), 0.0);
        assert_eq!(shift(3, 200.0), 200.0);
        assert_eq!(shift(3, 990.0), 963.0);
        // Bottom: held 10px above the viewport's bottom while its place is
        // below it, never above the top of its parent.
        assert_eq!(shift(4, 0.0), 50.0 + 300.0 - 10.0 - 990.0);
        assert_eq!(shift(4, 700.0), 0.0);
    }

    #[test]
    fn sections_flatten_to_headers_and_items() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        for (title, items) in [("Fruits", vec!["Apple", "Pear"]), ("Empty", vec![])] {
            bytes.extend_from_slice(&(title.len() as u32).to_le_bytes());
            bytes.extend_from_slice(title.as_bytes());
            bytes.extend_from_slice(&(items.len() as u32).to_le_bytes());
            for item in items {
                bytes.extend_from_slice(&(item.len() as u32).to_le_bytes());
                bytes.extend_from_slice(item.as_bytes());
            }
        }
        assert_eq!(
            decode_sections(&bytes),
            Some(vec![
                ("Fruits".into(), true),
                ("Apple".into(), false),
                ("Pear".into(), false),
                ("Empty".into(), true)
            ]),
        );
        bytes.push(0);
        assert!(decode_sections(&bytes).is_none());
    }
}
