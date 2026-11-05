//! The retained view tree: one lightweight `View` per node, positioned at
//! the frames the core computes. Nothing here talks to Win32 windows; the
//! tree is plain data plus cached Direct2D resources.

use std::collections::HashMap;

use pam_desktop_ui::accessibility::Accessibility;
use pam_desktop_ui::backend::{DrawCommand, ImageContent, NodeId, TextContent, ViewClass, WindowId};
use pam_desktop_ui::geometry::{Affine, Point, Rect, Size};
use pam_desktop_ui::scene::IdMap;
use pam_desktop_ui::shell::Cursor;
use pam_desktop_ui::style::{Decoration, LayerState};

use crate::paint::PaintCache;
use crate::scroll::ScrollState;
use crate::text::Layout;
use crate::window::WindowEntry;

pub struct TextState {
    pub content: TextContent,
    pub layout: Option<Layout>,
    /// Width the layout was built for.
    pub wrapped_to: f32,
    /// Device generation the run colors were applied for (0 = never).
    pub colored_for: u64,
    /// Selected UTF-16 range.
    pub selection: Option<(u32, u32)>,
}

pub struct View {
    pub class: ViewClass,
    pub parent: Option<NodeId>,
    /// Back to front.
    pub children: Vec<NodeId>,
    /// The window this view is the root or an overlay of.
    pub layer_of: Option<WindowId>,
    /// Relative to the parent (content coordinates under a scroll view).
    pub frame: Rect,
    pub decoration: Decoration,
    pub layer: LayerState,
    pub text: Option<TextState>,
    pub image: Option<ImageContent>,
    pub drawing: Vec<DrawCommand>,
    pub scroll: Option<Box<ScrollState>>,
    pub focusable: bool,
    pub cursor: Cursor,
    pub tooltip: Option<String>,
    pub accessibility: Option<Box<Accessibility>>,
    /// The accessible name last announced (live regions speak on change).
    pub announced: String,
    /// Direct2D resources derived from the decoration and the drawing.
    pub cache: Option<Box<PaintCache>>,
    pub input: Option<Box<crate::input::InputHost>>,
    pub media: Option<Box<crate::media::MediaPlayer>>,
    pub web: Option<Box<crate::webview::WebHost>>,
}

impl View {
    fn new(class: ViewClass) -> Self {
        Self {
            class,
            parent: None,
            children: Vec::new(),
            layer_of: None,
            frame: Rect::ZERO,
            decoration: Decoration::default(),
            layer: LayerState::default(),
            text: None,
            image: None,
            drawing: Vec::new(),
            scroll: (class == ViewClass::Scroll).then(|| Box::new(ScrollState::default())),
            focusable: false,
            cursor: Cursor::Default,
            tooltip: None,
            accessibility: None,
            announced: String::new(),
            cache: None,
            input: None,
            media: None,
            web: None,
        }
    }

    pub fn size(&self) -> Size {
        self.frame.size()
    }

    pub fn bounds(&self) -> Rect {
        Rect::from_size(self.frame.size())
    }

    /// Own coordinates to the parent's.
    pub fn to_parent(&self) -> Affine {
        if self.layer.transform.is_identity() {
            Affine::translation(self.frame.x, self.frame.y)
        } else {
            self.layer.transform.then(&Affine::translation(self.frame.x, self.frame.y))
        }
    }

    /// Children are positioned in content coordinates under a scroll view.
    pub fn content_offset(&self) -> Point {
        self.scroll.as_ref().map_or(Point::ZERO, |scroll| scroll.offset)
    }

    /// Whether descendants are cut at this view's bounds.
    pub fn clips(&self) -> bool {
        self.decoration.clip || self.class == ViewClass::Scroll
    }

    pub fn has_native(&self) -> bool {
        self.input.is_some() || self.web.is_some()
    }

    /// The plain text the view shows.
    pub fn plain_text(&self) -> Option<String> {
        self.text.as_ref().map(|text| text.content.block.plain_text())
    }
}

/// Stands for "anywhere" where an extent is not known.
const UNBOUNDED: Rect = Rect::new(-1.0e6, -1.0e6, 2.0e6, 2.0e6);

/// Where a native child window goes, in device pixels of its top-level window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub window: WindowId,
    /// The node's box.
    pub rect: Rect,
    /// What ancestors leave visible of the window (logical window coordinates).
    pub clip: Option<Rect>,
    pub visible: bool,
    pub scale: f32,
}

#[derive(Default)]
pub struct Scene {
    pub views: IdMap<View>,
    pub windows: HashMap<WindowId, WindowEntry>,
    /// The view with keyboard focus.
    pub focus: Option<NodeId>,
    /// Focus was moved with the keyboard: the focus ring shows.
    pub focus_visible: bool,
    /// Views backed by a child window.
    pub natives: Vec<NodeId>,
    /// Scroll views with a running animation or fading scrollbars.
    pub active_scrolls: Vec<NodeId>,
    /// Media views with a player.
    pub players: Vec<NodeId>,
}

impl Scene {
    pub fn create(&mut self, id: NodeId, class: ViewClass) {
        // Replacing a view the core forgot to destroy must not leak its links.
        if self.views.contains_key(&id) {
            self.destroy(id);
        }
        self.views.insert(id, View::new(class));
    }

    /// Removes a view from the tree and returns it so the caller can
    /// release its native resources outside the state borrow.
    pub fn destroy(&mut self, id: NodeId) -> Option<View> {
        let view = self.views.remove(&id)?;
        if let Some(window) = self.window_of_parent(view.parent, view.layer_of) {
            self.damage_all(window);
        }
        if let Some(parent) = view.parent.and_then(|parent| self.views.get_mut(&parent)) {
            parent.children.retain(|child| *child != id);
        }
        for child in &view.children {
            if let Some(child) = self.views.get_mut(child)
                && child.parent == Some(id) {
                    child.parent = None;
                }
        }
        for window in self.windows.values_mut() {
            if window.root == Some(id) {
                window.root = None;
            }
            window.overlays.retain(|overlay| *overlay != id);
            if window.hover == Some(id) {
                window.hover = None;
            }
        }
        if self.focus == Some(id) {
            self.focus = None;
        }
        self.natives.retain(|native| *native != id);
        self.active_scrolls.retain(|scroll| *scroll != id);
        self.players.retain(|player| *player != id);
        Some(view)
    }

    pub fn set_children(&mut self, parent: NodeId, children: &[NodeId]) {
        let Some(view) = self.views.get_mut(&parent) else { return };
        let previous = std::mem::take(&mut view.children);
        for child in &previous {
            if !children.contains(child)
                && let Some(child) = self.views.get_mut(child)
                    && child.parent == Some(parent) {
                        child.parent = None;
                    }
        }
        let mut kept = previous;
        kept.clear();
        for child in children {
            let Some(view) = self.views.get(child) else { continue };
            let old_parent = view.parent;
            if let Some(old) = old_parent.filter(|old| *old != parent) {
                // Where the child was must be repainted too.
                self.damage_subtree(*child);
                if let Some(old) = self.views.get_mut(&old) {
                    old.children.retain(|other| other != child);
                }
            }
            if let Some(view) = self.views.get_mut(child) {
                view.parent = Some(parent);
                view.layer_of = None;
            }
            kept.push(*child);
        }
        if let Some(view) = self.views.get_mut(&parent) {
            view.children = kept;
        }
        self.damage_subtree(parent);
    }

    /// Replaces the root and overlays of a window.
    pub fn set_layers(&mut self, window: WindowId, root: Option<Option<NodeId>>, overlays: Option<&[NodeId]>) {
        let Some(entry) = self.windows.get_mut(&window) else { return };
        let mut released: Vec<NodeId> = Vec::new();
        if let Some(root) = root {
            released.extend(entry.root);
            entry.root = root;
        }
        if let Some(overlays) = overlays {
            released.extend(entry.overlays.iter().copied());
            entry.overlays.clear();
            entry.overlays.extend_from_slice(overlays);
        }
        let layers: Vec<NodeId> = entry.root.iter().chain(entry.overlays.iter()).copied().collect();
        for id in released {
            if !layers.contains(&id)
                && let Some(view) = self.views.get_mut(&id)
                    && view.layer_of == Some(window) {
                        view.layer_of = None;
                    }
        }
        for id in layers {
            if let Some(view) = self.views.get_mut(&id) {
                view.layer_of = Some(window);
            }
        }
        self.damage_all(window);
    }

    fn window_of_parent(&self, parent: Option<NodeId>, layer_of: Option<WindowId>) -> Option<WindowId> {
        match parent {
            Some(parent) => self.window_of(parent),
            None => layer_of,
        }
    }

    /// The window a view is shown in, when it is attached to one.
    pub fn window_of(&self, id: NodeId) -> Option<WindowId> {
        let mut current = self.views.get(&id)?;
        // The depth bound guards against a malformed (cyclic) tree.
        for _ in 0..4096 {
            match current.parent {
                Some(parent) => current = self.views.get(&parent)?,
                None => return current.layer_of,
            }
        }
        None
    }

    /// View coordinates to window coordinates.
    pub fn world_transform(&self, id: NodeId) -> Option<(WindowId, Affine)> {
        let mut current = self.views.get(&id)?;
        let mut transform = current.to_parent();
        for _ in 0..4096 {
            match current.parent {
                Some(parent) => {
                    current = self.views.get(&parent)?;
                    let offset = current.content_offset();
                    if offset != Point::ZERO {
                        transform = transform.then(&Affine::translation(-offset.x, -offset.y));
                    }
                    transform = transform.then(&current.to_parent());
                }
                None => return current.layer_of.map(|window| (window, transform)),
            }
        }
        None
    }

    /// Whether the view and all its ancestors are shown.
    pub fn is_shown(&self, id: NodeId) -> bool {
        let mut current = self.views.get(&id);
        for _ in 0..4096 {
            let Some(view) = current else { return false };
            if !view.layer.visible || view.layer.opacity <= 0.001 {
                return false;
            }
            match view.parent {
                Some(parent) => current = self.views.get(&parent),
                None => return view.layer_of.is_some(),
            }
        }
        false
    }

    /// The whole window must be presented again.
    pub fn damage_all(&mut self, window: WindowId) {
        let natives = !self.natives.is_empty();
        if let Some(entry) = self.windows.get_mut(&window) {
            entry.damage(None);
            entry.natives_dirty |= natives;
        }
    }

    /// Whether nothing more can be added to a window's damage.
    fn fully_damaged(&self, window: WindowId) -> bool {
        self.windows.get(&window).is_none_or(|entry| entry.dirty == crate::window::Damage::All && (entry.natives_dirty || self.natives.is_empty()))
    }

    /// What a view paints inside its own (decorated) bounds changed.
    pub fn damage_view(&mut self, id: NodeId) {
        let Some(window) = self.window_of(id) else { return };
        if self.fully_damaged(window) {
            return;
        }
        let Some((window, transform)) = self.world_transform(id) else { return };
        let Some(view) = self.views.get(&id) else { return };
        // Filters spread paint beyond the bounds: present everything.
        let mut filtered = view.decoration.blur > 0.0;
        let mut ancestor = view.parent;
        while let Some(current) = ancestor.and_then(|ancestor| self.views.get(&ancestor)) {
            filtered |= current.decoration.blur > 0.0;
            ancestor = current.parent;
        }
        // The focus ring and anti-aliasing reach a little outside the box.
        let bounds = transform.bounds(view.decoration.overflow_bounds(view.bounds()).outset(6.0));
        if let Some(entry) = self.windows.get_mut(&window) {
            entry.damage(if filtered { None } else { Some(bounds) });
        }
    }

    /// Child windows of the view's window must be looked at again (their
    /// position, or the colors around them, may have changed).
    pub fn touch_natives(&mut self, id: NodeId) {
        if self.natives.is_empty() {
            return;
        }
        if let Some(entry) = self.window_of(id).and_then(|window| self.windows.get_mut(&window)) {
            entry.natives_dirty = true;
        }
    }

    /// What a view paints changed and may reach outside its box (text that
    /// is taller or wider than the box it sits in).
    pub fn damage_content(&mut self, id: NodeId) {
        if self.views.get(&id).is_some_and(View::clips) {
            self.damage_view(id);
        } else {
            self.damage_subtree(id);
        }
    }

    /// The position, transform, visibility or structure of a view changed.
    pub fn damage_subtree(&mut self, id: NodeId) {
        let Some(window) = self.window_of(id) else { return };
        // During a relayout thousands of frames change: once everything is
        // damaged there is nothing left to compute.
        if self.fully_damaged(window) {
            return;
        }
        // Descendants may overflow any box that does not clip, so only a
        // clipping ancestor bounds the change.
        let mut region = None;
        let mut ancestor = self.views.get(&id).and_then(|view| view.parent);
        while let Some(current_id) = ancestor {
            let Some(current) = self.views.get(&current_id) else { break };
            if current.decoration.blur > 0.0 {
                region = None;
                break;
            }
            if region.is_none() && current.clips() {
                region = self.world_transform(current_id).map(|(_, transform)| transform.bounds(current.bounds()).outset(2.0));
            }
            ancestor = current.parent;
        }
        let natives = !self.natives.is_empty();
        if let Some(entry) = self.windows.get_mut(&window) {
            entry.damage(region);
            entry.natives_dirty |= natives;
        }
    }

    /// The path from a layer root to the deepest view under `point`
    /// (window coordinates), topmost layer first. Empty when nothing is hit.
    pub fn hit_path(&self, window: WindowId, point: Point, path: &mut Vec<NodeId>) {
        path.clear();
        let Some(entry) = self.windows.get(&window) else { return };
        for layer in entry.overlays.iter().rev().chain(entry.root.iter()) {
            if self.hit_view(*layer, point, path, 0) {
                return;
            }
            path.clear();
        }
    }

    fn hit_view(&self, id: NodeId, point: Point, path: &mut Vec<NodeId>, depth: usize) -> bool {
        let Some(view) = self.views.get(&id) else { return false };
        if !view.layer.visible || depth > 512 {
            return false;
        }
        let Some(inverse) = view.to_parent().invert() else { return false };
        let local = inverse.apply(point);
        let inside = view.bounds().contains(local);
        if view.clips() && !inside {
            return false;
        }
        path.push(id);
        let offset = view.content_offset();
        let content = Point::new(local.x + offset.x, local.y + offset.y);
        for child in view.children.iter().rev() {
            if self.hit_view(*child, content, path, depth + 1) {
                return true;
            }
        }
        if inside {
            return true;
        }
        path.pop();
        false
    }

    /// Bounds of everything a view and its descendants may paint, in the
    /// view's own coordinates. Text that was never laid out has no known
    /// extent: with `exact` it counts as unbounded, otherwise as fitting
    /// its box (culling must not paint a view just to learn its size).
    pub fn subtree_bounds(&self, id: NodeId, depth: usize, exact: bool) -> Rect {
        let Some(view) = self.views.get(&id) else { return Rect::ZERO };
        if depth > 64 {
            return if exact { UNBOUNDED } else { view.bounds() };
        }
        let mut own = view.decoration.overflow_bounds(view.bounds());
        if view.clips() {
            return own;
        }
        if let Some(text) = &view.text {
            match &text.layout {
                Some(layout) => {
                    // Text taller or wider than its box paints outside it.
                    let insets = text.content.insets;
                    let origin = crate::text::text_origin(view.size(), insets, text.content.vertical, layout.height);
                    let available = (view.frame.width - insets.horizontal()).max(0.0);
                    let overhang = (layout.natural_width - available).max(0.0);
                    own = own.union(Rect::new(origin.x - overhang, origin.y, available + 2.0 * overhang, layout.height));
                }
                None if exact => return UNBOUNDED,
                None => {}
            }
        }
        let offset = view.content_offset();
        let bounds = view.children.iter().fold(own, |bounds, child| {
            let Some(child_view) = self.views.get(child) else { return bounds };
            if !child_view.layer.visible {
                return bounds;
            }
            let inner = self.subtree_bounds(*child, depth + 1, exact);
            bounds.union(child_view.to_parent().bounds(inner).translate(-offset.x, -offset.y))
        });
        // A blur spreads what it filters.
        if view.decoration.blur > 0.0 { bounds.outset(view.decoration.blur * 3.0) } else { bounds }
    }

    /// Where the child window of a native view goes.
    pub fn placement(&self, id: NodeId) -> Option<Placement> {
        let (window, transform) = self.world_transform(id)?;
        let entry = self.windows.get(&window)?;
        let view = self.views.get(&id)?;
        let rect = transform.bounds(view.bounds());
        let mut visible = self.is_shown(id);
        // Ancestors that clip cut the child window too.
        let mut clip: Option<Rect> = None;
        let mut layer = id;
        let mut ancestor = view.parent;
        while let Some(current_id) = ancestor {
            let Some(current) = self.views.get(&current_id) else { break };
            if current.clips()
                && let Some((_, world)) = self.world_transform(current_id) {
                    let bounds = world.bounds(current.bounds());
                    clip = Some(match clip {
                        Some(existing) => existing.intersect(bounds).unwrap_or(Rect::ZERO),
                        None => bounds,
                    });
                }
            layer = current_id;
            ancestor = current.parent;
        }
        // Entirely cut away: hidden. Entirely inside: no region needed.
        if clip.is_some_and(|clip| clip.intersect(rect).is_none()) {
            visible = false;
        }
        let clip = clip.filter(|clip| clip.intersect(rect) != Some(rect));
        // A child window always paints above the Direct2D scene: hide it
        // while a layer stacked above its own covers it.
        let layers = entry.root.iter().chain(entry.overlays.iter());
        let mut above = false;
        for other in layers {
            if *other == layer {
                above = true;
                continue;
            }
            if above
                && let Some(cover) = self.views.get(other)
                    && cover.layer.visible && cover.frame.intersects(rect) {
                        visible = false;
                    }
        }
        Some(Placement { window, rect, clip, visible, scale: entry.scale() })
    }
}
