//! Scrolling of `ViewClass::Scroll` views inside the retained tree: content
//! offset, wheel input, smooth animation, overlay scrollbars and keyboard
//! paging. The scroll view clips its children and paints the bars itself.

use pam_desktop_ui::backend::{BackendEvent, NodeId, ScrollConfig};
use pam_desktop_ui::geometry::{Point, Rect, Size};

use crate::scene::Scene;

/// Logical units scrolled per wheel line (three lines make a 100 unit notch).
pub const LINE_EXTENT: f32 = 100.0 / 3.0;
/// Time constant of the wheel glide, in milliseconds.
const GLIDE_TAU_MS: f32 = 70.0;
/// Duration of an animated `scroll_to`.
const TIMED_MS: f32 = 250.0;
/// How long the bars stay after the last activity, and how long they fade.
pub const BAR_HOLD_MS: u64 = 1200;
const BAR_FADE_MS: f32 = 220.0;
const BAR_THICKNESS: f32 = 6.0;
const BAR_MARGIN: f32 = 2.0;
const BAR_MIN_LENGTH: f32 = 28.0;
/// Width of the strip along the edge where the pointer reveals and grabs a bar.
const BAR_GRAB_EXTENT: f32 = 14.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Animation {
    None,
    /// Approaches `target` exponentially (wheel notches accumulate).
    Glide,
    /// Eases from `from` to `target` in a fixed time.
    Timed { from: Point, elapsed_ms: f32 },
}

#[derive(Debug)]
pub struct ScrollState {
    pub content: Size,
    pub offset: Point,
    pub config: ScrollConfig,
    pub target: Point,
    pub animation: Animation,
    /// 0..1 opacity of the overlay bars.
    pub bar_opacity: f32,
    /// When the bars start fading (monotonic milliseconds).
    pub bar_deadline: u64,
    /// The pointer is over a bar or dragging it: no fading.
    pub bar_held: bool,
    /// An offset asked with `scroll_to` that the content did not reach yet
    /// (a list told to open at its end before its rows were laid out).
    pub pending: Option<Point>,
}

impl Default for ScrollState {
    fn default() -> Self {
        Self {
            content: Size::ZERO,
            offset: Point::ZERO,
            config: ScrollConfig::default(),
            target: Point::ZERO,
            animation: Animation::None,
            bar_opacity: 0.0,
            bar_deadline: 0,
            bar_held: false,
            pending: None,
        }
    }
}

/// A scrollbar thumb being dragged.
#[derive(Clone, Copy, Debug)]
pub struct BarDrag {
    pub id: NodeId,
    pub vertical: bool,
    /// Distance from the start of the thumb to the pointer when grabbed.
    pub grab: f32,
    /// The press landed on the track and paged: the pointer owns the bar
    /// until release, without dragging it.
    pub paged: bool,
}

impl ScrollState {
    pub fn max_offset(&self, viewport: Size) -> Point {
        Point::new((self.content.width - viewport.width).max(0.0), (self.content.height - viewport.height).max(0.0))
    }

    fn clamp(&self, offset: Point, viewport: Size) -> Point {
        let max = self.max_offset(viewport);
        Point::new(offset.x.clamp(0.0, max.x), offset.y.clamp(0.0, max.y))
    }

    /// Track and thumb of one bar in the view's coordinates, when that axis scrolls.
    pub fn bar(&self, viewport: Size, vertical: bool) -> Option<(Rect, Rect)> {
        if !self.config.shows_indicators || !self.config.enabled {
            return None;
        }
        let (extent, content, offset) =
            if vertical { (viewport.height, self.content.height, self.offset.y) } else { (viewport.width, self.content.width, self.offset.x) };
        if content <= extent + 0.5 || extent <= BAR_MIN_LENGTH {
            return None;
        }
        let track_length = extent - 2.0 * BAR_MARGIN;
        let thumb_length = (track_length * extent / content).max(BAR_MIN_LENGTH).min(track_length);
        let travel = track_length - thumb_length;
        let position = BAR_MARGIN + if content > extent { travel * offset / (content - extent) } else { 0.0 };
        Some(if vertical {
            let x = viewport.width - BAR_THICKNESS - BAR_MARGIN;
            (Rect::new(x, BAR_MARGIN, BAR_THICKNESS, track_length), Rect::new(x, position, BAR_THICKNESS, thumb_length))
        } else {
            let y = viewport.height - BAR_THICKNESS - BAR_MARGIN;
            (Rect::new(BAR_MARGIN, y, track_length, BAR_THICKNESS), Rect::new(position, y, thumb_length, BAR_THICKNESS))
        })
    }

    /// The strip along the edge where the pointer can grab a bar.
    fn grab_zone(&self, viewport: Size, vertical: bool) -> Option<Rect> {
        self.bar(viewport, vertical)?;
        Some(if vertical {
            Rect::new(viewport.width - BAR_GRAB_EXTENT, 0.0, BAR_GRAB_EXTENT, viewport.height)
        } else {
            Rect::new(0.0, viewport.height - BAR_GRAB_EXTENT, viewport.width, BAR_GRAB_EXTENT)
        })
    }
}

/// What scrolling needs from its surroundings.
pub struct ScrollContext<'a> {
    pub scene: &'a mut Scene,
    pub events: &'a crate::app::Outbox,
    pub now_ms: u64,
    pub reduced_motion: bool,
}

impl ScrollContext<'_> {
    fn scale_of(&self, id: NodeId) -> f32 {
        self.scene.window_of(id).and_then(|window| self.scene.windows.get(&window)).map_or(1.0, |entry| entry.scale())
    }

    /// Moves the viewport. Returns whether the offset changed.
    pub fn set_offset(&mut self, id: NodeId, offset: Point) -> bool {
        let scale = self.scale_of(id);
        let Some(view) = self.scene.views.get_mut(&id) else { return false };
        let viewport = view.frame.size();
        let Some(scroll) = view.scroll.as_mut() else { return false };
        let clamped = scroll.clamp(offset, viewport);
        let max = scroll.max_offset(viewport);
        // Whole device pixels keep text and one pixel lines sharp while
        // scrolling; rounding must not pass the end of the content.
        let snapped = Point::new(((clamped.x * scale).round() / scale).min(max.x), ((clamped.y * scale).round() / scale).min(max.y));
        if snapped == scroll.offset {
            return false;
        }
        scroll.offset = snapped;
        self.events.push(BackendEvent::ScrollChanged { id, offset: snapped });
        self.reveal_bars(id);
        self.scene.damage_view(id);
        if !self.scene.natives.is_empty() {
            if let Some(entry) = self.scene.window_of(id).and_then(|window| self.scene.windows.get_mut(&window)) {
                entry.natives_dirty = true;
            }
        }
        true
    }

    /// Shows the bars and restarts their fade deadline.
    pub fn reveal_bars(&mut self, id: NodeId) {
        let Some(scroll) = self.scene.views.get_mut(&id).and_then(|view| view.scroll.as_mut()) else { return };
        if !scroll.config.shows_indicators || !scroll.config.enabled {
            return;
        }
        scroll.bar_deadline = self.now_ms + BAR_HOLD_MS;
        if scroll.bar_opacity < 1.0 {
            scroll.bar_opacity = 1.0;
            self.scene.damage_view(id);
        }
        if !self.scene.active_scrolls.contains(&id) {
            self.scene.active_scrolls.push(id);
        }
    }

    pub fn set_content(&mut self, id: NodeId, size: Size) {
        let Some(view) = self.scene.views.get_mut(&id) else { return };
        let Some(scroll) = view.scroll.as_mut() else { return };
        if scroll.content == size {
            return;
        }
        scroll.content = size;
        self.settle(id);
    }

    /// The content or the viewport changed size: an offset that was asked
    /// before it could be reached is applied, and the viewport is pulled
    /// back inside a shorter content.
    pub fn settle(&mut self, id: NodeId) {
        let Some(view) = self.scene.views.get_mut(&id) else { return };
        let viewport = view.frame.size();
        let Some(scroll) = view.scroll.as_mut() else { return };
        let wanted = match scroll.pending {
            Some(pending) => {
                let reachable = scroll.clamp(pending, viewport);
                if reachable.distance(pending) < 0.5 {
                    scroll.pending = None;
                }
                scroll.animation = Animation::None;
                scroll.target = reachable;
                reachable
            }
            None => {
                scroll.target = scroll.clamp(scroll.target, viewport);
                scroll.offset
            }
        };
        if !self.set_offset(id, wanted) {
            self.scene.damage_view(id);
        }
    }

    pub fn scroll_to(&mut self, id: NodeId, offset: Point, animated: bool) {
        let Some(view) = self.scene.views.get_mut(&id) else { return };
        let viewport = view.frame.size();
        let Some(scroll) = view.scroll.as_mut() else { return };
        let target = scroll.clamp(offset, viewport);
        scroll.target = target;
        // Beyond the content as it is now: remembered until the content grows.
        scroll.pending = (target.distance(offset) >= 0.5 && offset.x >= 0.0 && offset.y >= 0.0).then_some(offset);
        if animated && !self.reduced_motion && target.distance(scroll.offset) > 0.5 {
            scroll.animation = Animation::Timed { from: scroll.offset, elapsed_ms: 0.0 };
            if !self.scene.active_scrolls.contains(&id) {
                self.scene.active_scrolls.push(id);
            }
        } else {
            scroll.animation = Animation::None;
            self.set_offset(id, target);
        }
    }

    /// Scrolls the innermost scroll view on `path` that can move by
    /// `delta` (positive scrolls the content up / left). Returns whether a
    /// view consumed it.
    pub fn wheel(&mut self, path: &[NodeId], delta: Point, smooth: bool) -> bool {
        for id in path.iter().rev() {
            let Some(view) = self.scene.views.get(id) else { continue };
            let viewport = view.frame.size();
            let Some(scroll) = view.scroll.as_ref() else { continue };
            if !scroll.config.enabled {
                continue;
            }
            let max = scroll.max_offset(viewport);
            let mut delta = delta;
            // A view that only scrolls sideways takes the vertical wheel.
            if max.y <= 0.0 && max.x > 0.0 && delta.x == 0.0 {
                delta = Point::new(delta.y, 0.0);
            }
            let base = if scroll.animation == Animation::Glide { scroll.target } else { scroll.offset };
            let wanted = scroll.clamp(Point::new(base.x + delta.x, base.y + delta.y), viewport);
            if wanted.distance(base) < 0.01 {
                // At its limit (or not scrollable on this axis): the parent takes it.
                continue;
            }
            let id = *id;
            if let Some(scroll) = self.state_mut(id) {
                // The user took over: a deferred programmatic offset is void.
                scroll.pending = None;
            }
            if smooth && !self.reduced_motion {
                if let Some(scroll) = self.scene.views.get_mut(&id).and_then(|view| view.scroll.as_mut()) {
                    scroll.target = wanted;
                    scroll.animation = Animation::Glide;
                }
                if !self.scene.active_scrolls.contains(&id) {
                    self.scene.active_scrolls.push(id);
                }
            } else {
                if let Some(scroll) = self.scene.views.get_mut(&id).and_then(|view| view.scroll.as_mut()) {
                    scroll.target = wanted;
                    scroll.animation = Animation::None;
                }
                self.set_offset(id, wanted);
            }
            return true;
        }
        false
    }

    fn state_mut(&mut self, id: NodeId) -> Option<&mut ScrollState> {
        self.scene.views.get_mut(&id).and_then(|view| view.scroll.as_deref_mut())
    }

    /// Advances one scroll view. Returns (keep it active, needs another frame).
    fn tick_one(&mut self, id: NodeId, elapsed_ms: f32) -> (bool, bool) {
        let Some(scroll) = self.state_mut(id) else { return (false, false) };
        let (animation, offset, target) = (scroll.animation, scroll.offset, scroll.target);
        let mut animating = false;
        match animation {
            Animation::None => {}
            Animation::Glide => {
                let remaining = Point::new(target.x - offset.x, target.y - offset.y);
                let factor = 1.0 - (-elapsed_ms / GLIDE_TAU_MS).exp();
                let next = Point::new(offset.x + remaining.x * factor, offset.y + remaining.y * factor);
                let close = remaining.x.abs() < 0.5 && remaining.y.abs() < 0.5;
                // A step smaller than a pixel would stall the glide: finish it.
                if close || !self.set_offset(id, next) {
                    if let Some(scroll) = self.state_mut(id) {
                        scroll.animation = Animation::None;
                    }
                    self.set_offset(id, target);
                } else {
                    animating = true;
                }
            }
            Animation::Timed { from, elapsed_ms: before } => {
                let elapsed = before + elapsed_ms;
                if elapsed >= TIMED_MS {
                    if let Some(scroll) = self.state_mut(id) {
                        scroll.animation = Animation::None;
                    }
                    self.set_offset(id, target);
                } else {
                    if let Some(scroll) = self.state_mut(id) {
                        scroll.animation = Animation::Timed { from, elapsed_ms: elapsed };
                    }
                    let progress = 1.0 - (1.0 - elapsed / TIMED_MS).powi(3);
                    self.set_offset(id, Point::new(from.x + (target.x - from.x) * progress, from.y + (target.y - from.y) * progress));
                    animating = true;
                }
            }
        }
        let now_ms = self.now_ms;
        let Some(scroll) = self.state_mut(id) else { return (false, false) };
        if scroll.bar_opacity <= 0.0 {
            return (animating, animating);
        }
        if scroll.bar_held || animating {
            scroll.bar_deadline = now_ms + BAR_HOLD_MS;
            return (true, animating);
        }
        if now_ms < scroll.bar_deadline {
            // Waiting for the hold to end: the bar timer wakes the fade.
            return (true, false);
        }
        scroll.bar_opacity = (scroll.bar_opacity - elapsed_ms / BAR_FADE_MS).max(0.0);
        let fading = scroll.bar_opacity > 0.0;
        self.scene.damage_view(id);
        (fading, fading)
    }

    /// Advances animations and bar fades by `elapsed_ms`. Returns whether
    /// another frame is needed.
    pub fn tick(&mut self, elapsed_ms: f32) -> bool {
        let mut active = std::mem::take(&mut self.scene.active_scrolls);
        let mut needs_frame = false;
        active.retain(|id| {
            let (keep, again) = self.tick_one(*id, elapsed_ms);
            needs_frame |= again;
            keep
        });
        // Views that became active during the pass were pushed to the scene's list.
        for id in std::mem::take(&mut self.scene.active_scrolls) {
            if !active.contains(&id) {
                active.push(id);
            }
        }
        self.scene.active_scrolls = active;
        needs_frame
    }

    /// Earliest moment a held bar starts fading, if any is waiting.
    pub fn next_bar_deadline(&self) -> Option<u64> {
        self.scene
            .active_scrolls
            .iter()
            .filter_map(|id| self.scene.views.get(id).and_then(|view| view.scroll.as_ref()))
            .filter(|scroll| scroll.bar_opacity > 0.0 && !scroll.bar_held && scroll.animation == Animation::None)
            .map(|scroll| scroll.bar_deadline)
            .min()
    }

    /// Pointer movement over the window: reveals the bar under the pointer
    /// and keeps it while hovered.
    pub fn hover(&mut self, path: &[NodeId], point: Point) {
        let mut hovered: Option<NodeId> = None;
        for id in path.iter().rev() {
            let Some(view) = self.scene.views.get(id) else { continue };
            let Some(scroll) = view.scroll.as_ref() else { continue };
            let Some(local) = self.scene.world_transform(*id).and_then(|(_, transform)| transform.invert()).map(|inverse| inverse.apply(point)) else {
                continue;
            };
            let viewport = view.frame.size();
            let over = [true, false].into_iter().any(|vertical| scroll.grab_zone(viewport, vertical).is_some_and(|zone| zone.contains(local)));
            if over {
                hovered = Some(*id);
            }
            break;
        }
        let active = self.scene.active_scrolls.clone();
        for id in active {
            if Some(id) != hovered {
                if let Some(scroll) = self.scene.views.get_mut(&id).and_then(|view| view.scroll.as_mut()) {
                    scroll.bar_held = false;
                }
            }
        }
        if let Some(id) = hovered {
            self.reveal_bars(id);
            if let Some(scroll) = self.scene.views.get_mut(&id).and_then(|view| view.scroll.as_mut()) {
                scroll.bar_held = true;
            }
        }
    }

    /// A primary button press: grabs the bar under the pointer, or pages
    /// when the track is pressed. `None` leaves the press to the core.
    pub fn press(&mut self, path: &[NodeId], point: Point) -> Option<BarDrag> {
        for id in path.iter().rev() {
            let Some(view) = self.scene.views.get(id) else { continue };
            let Some(scroll) = view.scroll.as_ref() else { continue };
            let local = self.scene.world_transform(*id).and_then(|(_, transform)| transform.invert()).map(|inverse| inverse.apply(point))?;
            let viewport = view.frame.size();
            for vertical in [true, false] {
                let Some(zone) = scroll.grab_zone(viewport, vertical) else { continue };
                if !zone.contains(local) || scroll.bar_opacity <= 0.0 {
                    continue;
                }
                let (_, thumb) = scroll.bar(viewport, vertical)?;
                let (position, start, length) = if vertical { (local.y, thumb.y, thumb.height) } else { (local.x, thumb.x, thumb.width) };
                let id = *id;
                if position < start || position > start + length {
                    // The track: one page towards the pointer.
                    let page = if vertical { viewport.height } else { viewport.width } * 0.9;
                    let step = if position < start { -page } else { page };
                    let offset = scroll.offset;
                    let wanted = if vertical { Point::new(offset.x, offset.y + step) } else { Point::new(offset.x + step, offset.y) };
                    self.scroll_to(id, wanted, true);
                    return Some(BarDrag { id, vertical, grab: 0.0, paged: true });
                }
                return Some(BarDrag { id, vertical, grab: position - start, paged: false });
            }
            return None;
        }
        None
    }

    /// Follows the pointer with a grabbed thumb.
    pub fn drag(&mut self, drag: BarDrag, point: Point) {
        if drag.paged {
            return;
        }
        let Some(view) = self.scene.views.get(&drag.id) else { return };
        let Some(scroll) = view.scroll.as_ref() else { return };
        let Some(local) = self.scene.world_transform(drag.id).and_then(|(_, transform)| transform.invert()).map(|inverse| inverse.apply(point)) else {
            return;
        };
        let viewport = view.frame.size();
        let Some((track, thumb)) = scroll.bar(viewport, drag.vertical) else { return };
        let max = scroll.max_offset(viewport);
        let (position, track_start, travel, limit) = if drag.vertical {
            (local.y, track.y, track.height - thumb.height, max.y)
        } else {
            (local.x, track.x, track.width - thumb.width, max.x)
        };
        if travel <= 0.0 {
            return;
        }
        let along = ((position - drag.grab - track_start) / travel).clamp(0.0, 1.0) * limit;
        let offset = scroll.offset;
        let wanted = if drag.vertical { Point::new(offset.x, along) } else { Point::new(along, offset.y) };
        if let Some(scroll) = self.scene.views.get_mut(&drag.id).and_then(|view| view.scroll.as_mut()) {
            scroll.animation = Animation::None;
            scroll.target = wanted;
            scroll.bar_held = true;
            scroll.pending = None;
        }
        self.set_offset(drag.id, wanted);
    }

    /// Paging keys for the scroll view around the focused view. Returns
    /// whether the key scrolled.
    pub fn key(&mut self, key: &str, control: bool) -> bool {
        let Some(focus) = self.scene.focus else { return false };
        let mut current = Some(focus);
        while let Some(id) = current {
            let Some(view) = self.scene.views.get(&id) else { return false };
            if let Some(scroll) = view.scroll.as_ref().filter(|scroll| scroll.config.enabled) {
                let viewport = view.frame.size();
                let max = scroll.max_offset(viewport);
                let horizontal = max.y <= 0.0 && max.x > 0.0;
                let page = if horizontal { viewport.width } else { viewport.height } * 0.9;
                let base = if scroll.animation == Animation::None { scroll.offset } else { scroll.target };
                let along = |amount: f32| if horizontal { Point::new(base.x + amount, base.y) } else { Point::new(base.x, base.y + amount) };
                let wanted = match (key, control) {
                    ("PageDown", false) => along(page),
                    ("PageUp", false) => along(-page),
                    ("Home", _) => along(-f32::MAX / 4.0),
                    ("End", _) => along(f32::MAX / 4.0),
                    ("ArrowDown", true) => Point::new(base.x, base.y + LINE_EXTENT),
                    ("ArrowUp", true) => Point::new(base.x, base.y - LINE_EXTENT),
                    ("ArrowRight", true) => Point::new(base.x + LINE_EXTENT, base.y),
                    ("ArrowLeft", true) => Point::new(base.x - LINE_EXTENT, base.y),
                    _ => return false,
                };
                let wanted = scroll.clamp(wanted, viewport);
                if wanted.distance(base) >= 0.5 {
                    self.scroll_to(id, wanted, true);
                    return true;
                }
            }
            current = view.parent;
        }
        false
    }
}
