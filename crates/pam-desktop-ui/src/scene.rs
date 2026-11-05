//! The retained scene: every node the engine mounted, its resolved paint
//! state, and the minimal set of backend commands needed after each batch of
//! mutations.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use pam_desktop_wire::{Mutation, Node, NodeKind, PropKey, PropValue};

use crate::backend::{Backend, NodeId, TextContent, VerticalAlign, ViewClass, WindowId, MAIN_WINDOW};
use crate::color::Color;
use crate::geometry::{Insets, Point, Rect, Size};
use crate::props::{PropMap, Props};
use crate::style::{resolve_transform, Decoration, LayerState};
use crate::text::{Ellipsize, TextAlign, TextBlock, TextShadow, TextSource};

/// Node ids are already well distributed 64-bit values: hashing them again
/// would only cost time.
#[derive(Default)]
pub struct IdHasher(u64);

impl Hasher for IdHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = self.0.rotate_left(8) ^ u64::from(*byte);
        }
    }

    fn write_u64(&mut self, value: u64) {
        // Fold the high half into the low bits used for bucket selection.
        self.0 = value ^ (value >> 32);
    }
}

pub type IdMap<V> = HashMap<NodeId, V, BuildHasherDefault<IdHasher>>;

/// What must be pushed to the backend for a node.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Dirty(u16);

impl Dirty {
    pub const NONE: Self = Self(0);
    pub const CREATED: Self = Self(1 << 0);
    pub const CHILDREN: Self = Self(1 << 1);
    pub const FRAME: Self = Self(1 << 2);
    pub const DECORATION: Self = Self(1 << 3);
    pub const LAYER: Self = Self(1 << 4);
    pub const TEXT: Self = Self(1 << 5);
    pub const CONFIG: Self = Self(1 << 6);
    pub const ORDER: Self = Self(1 << 7);
    pub const CHILD_TEXT: Self = Self(1 << 8);
    pub const ALL: Self = Self(0x7F);

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Debug)]
pub struct SceneNode {
    pub id: NodeId,
    pub kind: NodeKind,
    pub parent: NodeId,
    pub index: u32,
    pub props: PropMap,
    pub children: Vec<NodeId>,
    /// Engine frame in window (root) coordinates.
    pub frame: Rect,
    pub class: ViewClass,
    pub decoration: Decoration,
    pub layer: LayerState,
    dirty: Dirty,
    /// Frame last pushed to the backend (parent relative).
    pushed_frame: Option<Rect>,
    has_text: bool,
}

impl SceneNode {
    #[must_use]
    pub fn props(&self) -> Props<'_> {
        Props(&self.props)
    }

    #[must_use]
    pub fn handles(&self, key: PropKey) -> bool {
        self.props.contains_key(&key)
    }

    #[must_use]
    pub fn is_text_bearing(&self) -> bool {
        matches!(self.kind, NodeKind::Text | NodeKind::Button)
    }
}

/// The native object that backs each node kind.
#[must_use]
pub fn view_class(kind: NodeKind, props: Props<'_>) -> ViewClass {
    match kind {
        NodeKind::Scroll | NodeKind::List | NodeKind::SectionList | NodeKind::VirtualList => ViewClass::Scroll,
        NodeKind::Input => {
            if props.flag_or(PropKey::Multiline, false) {
                ViewClass::TextArea
            } else {
                ViewClass::Input
            }
        }
        NodeKind::Canvas | NodeKind::DrawingCanvas => ViewClass::Surface,
        _ => ViewClass::Box,
    }
}

/// Which backend state a property feeds.
#[must_use]
pub fn dirty_for(key: PropKey) -> Dirty {
    use PropKey as K;
    match key {
        K::BackgroundColor
        | K::BorderRadius
        | K::BorderTopLeftRadius
        | K::BorderTopRightRadius
        | K::BorderBottomRightRadius
        | K::BorderBottomLeftRadius
        | K::BorderColor
        | K::BorderLeftColor
        | K::BorderTopColor
        | K::BorderRightColor
        | K::BorderBottomColor
        | K::BorderStyle
        | K::ShadowOffsetX
        | K::ShadowOffsetY
        | K::ShadowBlurRadius
        | K::ShadowSpreadRadius
        | K::ShadowColor
        | K::BoxShadows
        | K::Elevation
        | K::BackgroundGradient
        | K::BorderGradient
        | K::Overflow
        | K::BlurRadius
        | K::FilterColorMatrix
        | K::BackdropBlurRadius
        | K::BackdropColorMatrix => Dirty::DECORATION,
        // Borders and padding also move text away from the box edges.
        K::BorderWidth
        | K::BorderLeftWidth
        | K::BorderTopWidth
        | K::BorderRightWidth
        | K::BorderBottomWidth => Dirty::DECORATION.union(Dirty::TEXT),
        K::Padding
        | K::PaddingHorizontal
        | K::PaddingVertical
        | K::PaddingLeft
        | K::PaddingTop
        | K::PaddingRight
        | K::PaddingBottom => Dirty::TEXT.union(Dirty::CONFIG),
        K::Opacity
        | K::TranslationX
        | K::TranslationY
        | K::TranslationXPercent
        | K::TranslationYPercent
        | K::ScaleX
        | K::ScaleY
        | K::Rotation
        | K::TransformOriginX
        | K::TransformOriginY
        | K::Visible => Dirty::LAYER,
        K::Text
        | K::TextColor
        | K::FontSize
        | K::FontWeight
        | K::FontStyle
        | K::FontFamily
        | K::LetterSpacing
        | K::LineHeight
        | K::TextAlign
        | K::NumberOfLines
        | K::TextDecoration
        | K::TextTransform
        | K::TextSpans
        | K::TextEllipsizeMode
        | K::TextShadowOffsetX
        | K::TextShadowOffsetY
        | K::TextShadowRadius
        | K::TextShadowColor
        | K::FontFeatureSettings
        | K::TextAllowFontScaling
        | K::TextMaxFontSizeMultiplier
        | K::TextSelectable
        | K::Width
        | K::MinWidth
        | K::FlexGrow
        | K::AlignSelf => Dirty::TEXT,
        // A parent's main/cross alignment decides how unaligned child text sits.
        K::AlignItems | K::JustifyContent | K::FlexDirection => Dirty::CHILD_TEXT,
        K::ZIndex => Dirty::ORDER,
        _ => Dirty::CONFIG,
    }
}

/// Rendering environment shared by every node of a window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Environment {
    /// Device pixels per logical unit, used to snap frames to the pixel grid.
    pub scale: f32,
    pub font_scale: f32,
}

impl Default for Environment {
    fn default() -> Self {
        Self { scale: 1.0, font_scale: 1.0 }
    }
}

#[derive(Default)]
pub struct Scene {
    nodes: IdMap<SceneNode>,
    root: NodeId,
    root_dirty: bool,
    touched: Vec<NodeId>,
    removed: Vec<NodeId>,
    pub environment: Environment,
    /// Counters for diagnostics and tests.
    pub stats: SceneStats,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SceneStats {
    pub batches: u64,
    pub mutations: u64,
    pub commands: u64,
}

impl Scene {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn root(&self) -> NodeId {
        self.root
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&SceneNode> {
        self.nodes.get(&id)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut SceneNode> {
        self.nodes.get_mut(&id)
    }

    pub fn nodes(&self) -> impl Iterator<Item = &SceneNode> {
        self.nodes.values()
    }

    /// Marks a node for the next flush.
    pub fn touch(&mut self, id: NodeId, dirty: Dirty) {
        if let Some(node) = self.nodes.get_mut(&id) {
            if node.dirty.is_empty() {
                self.touched.push(id);
            }
            node.dirty.insert(dirty);
        }
    }

    /// Re-resolves every node (text scale or display scale changed).
    pub fn invalidate_all(&mut self) {
        let ids: Vec<NodeId> = self.nodes.keys().copied().collect();
        for id in ids {
            self.touch(id, Dirty::FRAME.union(Dirty::TEXT).union(Dirty::LAYER));
        }
    }

    /// Drops every node: the host asks the engine for a full remount next.
    pub fn clear(&mut self, backend: &mut impl Backend) {
        let mut ids = Vec::with_capacity(self.nodes.len());
        if self.nodes.contains_key(&self.root) {
            self.collect_post_order(self.root, &mut ids);
        }
        for id in self.nodes.keys() {
            if !ids.contains(id) {
                ids.push(*id);
            }
        }
        backend.set_window_root(MAIN_WINDOW, None);
        for id in ids {
            backend.destroy_view(id);
        }
        self.nodes.clear();
        self.touched.clear();
        self.removed.clear();
        self.root = 0;
        self.root_dirty = false;
    }

    pub fn apply(&mut self, mutations: Vec<Mutation>) {
        self.stats.batches += 1;
        self.stats.mutations += mutations.len() as u64;
        for mutation in mutations {
            match mutation {
                Mutation::Create(node) => self.create(node),
                Mutation::Remove { id } => self.remove(id),
                Mutation::Update { id, key, value } => self.update(id, key, value),
                Mutation::Move { id, parent, index } => self.reparent(id, parent, index),
                Mutation::Layout { id, frame } => {
                    let frame = Rect::new(frame.x, frame.y, frame.width, frame.height);
                    let children = match self.nodes.get_mut(&id) {
                        Some(node) => {
                            let moved = node.frame.origin() != frame.origin();
                            let resized = node.frame.size() != frame.size();
                            node.frame = frame;
                            (moved || resized).then(|| node.children.clone())
                        }
                        None => None,
                    };
                    // A resize can change percent transforms and text wrapping.
                    self.touch(id, Dirty::FRAME.union(Dirty::LAYER));
                    // Children are positioned relative to this origin.
                    for child in children.into_iter().flatten() {
                        self.touch(child, Dirty::FRAME);
                    }
                }
                Mutation::SetRoot { id } => {
                    if self.root != id {
                        self.root = id;
                        self.root_dirty = true;
                    }
                }
            }
        }
    }

    fn create(&mut self, node: Node) {
        if self.nodes.contains_key(&node.id) {
            self.remove(node.id);
        }
        let class = view_class(node.kind, Props(&node.properties));
        let id = node.id;
        let parent = node.parent;
        self.nodes.insert(
            id,
            SceneNode {
                id,
                kind: node.kind,
                parent,
                index: node.index,
                props: node.properties,
                children: Vec::new(),
                frame: Rect::ZERO,
                class,
                decoration: Decoration::default(),
                layer: LayerState::default(),
                dirty: Dirty::NONE,
                pushed_frame: None,
                has_text: false,
            },
        );
        self.touch(id, Dirty::ALL);
        if let Some(parent_node) = self.nodes.get_mut(&parent) {
            parent_node.children.push(id);
            self.touch(parent, Dirty::CHILDREN);
        }
    }

    fn collect_post_order(&self, id: NodeId, output: &mut Vec<NodeId>) {
        if let Some(node) = self.nodes.get(&id) {
            for child in &node.children {
                self.collect_post_order(*child, output);
            }
            output.push(id);
        }
    }

    fn remove(&mut self, id: NodeId) {
        let Some(parent) = self.nodes.get(&id).map(|node| node.parent) else { return };
        if let Some(parent_node) = self.nodes.get_mut(&parent) {
            parent_node.children.retain(|child| *child != id);
            self.touch(parent, Dirty::CHILDREN);
        }
        let mut subtree = Vec::new();
        self.collect_post_order(id, &mut subtree);
        for removed in subtree {
            self.nodes.remove(&removed);
            self.removed.push(removed);
        }
        if self.root == id {
            self.root = 0;
            self.root_dirty = true;
        }
    }

    fn update(&mut self, id: NodeId, key: PropKey, value: Option<PropValue>) {
        let Some(node) = self.nodes.get_mut(&id) else { return };
        match value {
            Some(value) => {
                node.props.insert(key, value);
            }
            None => {
                node.props.remove(&key);
            }
        }
        let dirty = dirty_for(key);
        let parent = node.parent;
        if dirty.contains(Dirty::CHILD_TEXT) {
            let children = node.children.clone();
            for child in children {
                self.touch(child, Dirty::TEXT);
            }
        }
        if dirty.contains(Dirty::ORDER) {
            self.touch(parent, Dirty::CHILDREN);
        }
        self.touch(id, dirty);
    }

    fn reparent(&mut self, id: NodeId, parent: NodeId, index: u32) {
        let Some(old_parent) = self.nodes.get(&id).map(|node| node.parent) else { return };
        if old_parent != parent {
            if let Some(old) = self.nodes.get_mut(&old_parent) {
                old.children.retain(|child| *child != id);
            }
            if let Some(new) = self.nodes.get_mut(&parent) {
                new.children.push(id);
            }
            self.touch(old_parent, Dirty::CHILDREN);
        }
        if let Some(node) = self.nodes.get_mut(&id) {
            node.parent = parent;
            node.index = index;
        }
        self.touch(parent, Dirty::CHILDREN);
        self.touch(id, Dirty::FRAME.union(Dirty::TEXT));
    }

    /// Children in paint order: `zIndex` first, then document order.
    fn ordered_children(&self, id: NodeId) -> Vec<NodeId> {
        let Some(node) = self.nodes.get(&id) else { return Vec::new() };
        let mut children: Vec<(f32, u32, NodeId)> = node
            .children
            .iter()
            .filter_map(|child| {
                let child_node = self.nodes.get(child)?;
                Some((child_node.props().number_or(PropKey::ZIndex, 0.0), child_node.index, *child))
            })
            .collect();
        children.sort_by(|left, right| left.0.total_cmp(&right.0).then(left.1.cmp(&right.1)));
        children.into_iter().map(|(_, _, child)| child).collect()
    }

    /// Frame relative to the parent view, snapped to the device pixel grid.
    fn relative_frame(&self, node: &SceneNode) -> Rect {
        let scale = self.environment.scale;
        let own = node.frame.snapped(scale);
        match self.nodes.get(&node.parent) {
            Some(parent) => {
                let origin = parent.frame.snapped(scale).origin();
                own.translate(-origin.x, -origin.y)
            }
            None => own,
        }
    }

    /// 1 start, 2 center, 3 end, 4 justify; unset text follows how its flex
    /// parent aligns it (the box moves, wrapped lines stay at the start).
    fn text_align(&self, node: &SceneNode) -> TextAlign {
        let props = node.props();
        match props.integer(PropKey::TextAlign) {
            Some(2) => return TextAlign::Center,
            Some(3) => return TextAlign::End,
            Some(4) => return TextAlign::Justify,
            Some(_) => return TextAlign::Start,
            None => {}
        }
        if node.kind == NodeKind::Button {
            return TextAlign::Center;
        }
        let Some(parent) = self.nodes.get(&node.parent) else { return TextAlign::Start };
        let allocated = props.has(PropKey::Width) || props.has(PropKey::MinWidth) || props.number_or(PropKey::FlexGrow, 0.0) > 0.0;
        if allocated {
            return TextAlign::Start;
        }
        let parent_props = parent.props();
        let default_direction = if parent.kind == NodeKind::Row { 2 } else { 1 };
        let direction = parent_props.integer_or(PropKey::FlexDirection, default_direction);
        let alignment = if direction == 1 || direction == 3 {
            match props.integer(PropKey::AlignSelf) {
                None | Some(4) => parent_props.integer_or(PropKey::AlignItems, 4),
                Some(authored) => authored,
            }
        } else {
            parent_props.integer_or(PropKey::JustifyContent, 1)
        };
        match alignment {
            2 => TextAlign::Center,
            3 => TextAlign::End,
            _ => TextAlign::Start,
        }
    }

    fn text_content(&self, node: &SceneNode) -> Option<TextContent> {
        if !node.is_text_bearing() {
            return None;
        }
        let props = node.props();
        let source = TextSource::from_props(node.kind, props, self.environment.font_scale);
        if source.text.is_empty() {
            return None;
        }
        let shadow_color = props.color(PropKey::TextShadowColor).unwrap_or(Color::TRANSPARENT);
        let block = TextBlock {
            runs: source.runs(props.color(PropKey::TextColor), props.integer_or(PropKey::TextDecoration, 1)),
            align: self.text_align(node),
            max_lines: source.max_lines,
            ellipsize: match props.integer(PropKey::TextEllipsizeMode) {
                Some(2) => Ellipsize::Head,
                Some(3) => Ellipsize::Middle,
                Some(4) => Ellipsize::Clip,
                _ => Ellipsize::Tail,
            },
            line_height: source.line_height * source.font_scale,
            shadow: (!shadow_color.is_transparent()).then(|| TextShadow {
                offset: Point::new(
                    props.number_or(PropKey::TextShadowOffsetX, 0.0),
                    props.number_or(PropKey::TextShadowOffsetY, 0.0),
                ),
                radius: props.number_or(PropKey::TextShadowRadius, 0.0).max(0.0),
                color: shadow_color,
            }),
            selectable: props.flag_or(PropKey::TextSelectable, false),
        };
        let padding = props.padding();
        let border = node.decoration.border.widths;
        Some(TextContent {
            block,
            insets: Insets::new(
                padding.left + border[0],
                padding.top + border[1],
                padding.right + border[2],
                padding.bottom + border[3],
            ),
            vertical: if node.kind == NodeKind::Button { VerticalAlign::Center } else { VerticalAlign::Top },
        })
    }

    /// Pushes everything that changed since the last flush to the backend.
    pub fn flush(&mut self, backend: &mut impl Backend, window: WindowId) {
        let mut commands = 0_u64;

        // 1. Views of removed nodes (children before their parents).
        for id in self.removed.drain(..) {
            backend.destroy_view(id);
            commands += 1;
        }

        let touched = std::mem::take(&mut self.touched);

        // 2. New views, parents first (creation order is pre-order).
        for id in &touched {
            if let Some(node) = self.nodes.get(id) {
                if node.dirty.contains(Dirty::CREATED) {
                    backend.create_view(*id, node.class);
                    commands += 1;
                }
            }
        }

        // 3. Paint state.
        for id in &touched {
            let Some(node) = self.nodes.get(id) else { continue };
            let dirty = node.dirty;
            if dirty.contains(Dirty::DECORATION) {
                let decoration = Decoration::resolve(node.kind, node.props());
                if dirty.contains(Dirty::CREATED) || decoration != node.decoration {
                    if !(dirty.contains(Dirty::CREATED) && decoration.is_plain()) {
                        backend.set_decoration(*id, &decoration);
                        commands += 1;
                    }
                    if let Some(node) = self.nodes.get_mut(id) {
                        node.decoration = decoration;
                    }
                }
            }
            let Some(node) = self.nodes.get(id) else { continue };
            if dirty.contains(Dirty::TEXT) && node.is_text_bearing() {
                let content = self.text_content(node);
                if content.is_some() || node.has_text {
                    backend.set_text(*id, content.as_ref());
                    commands += 1;
                }
                let has_text = content.is_some();
                if let Some(node) = self.nodes.get_mut(id) {
                    node.has_text = has_text;
                }
            }
            let Some(node) = self.nodes.get(id) else { continue };
            if dirty.contains(Dirty::LAYER) {
                let props = node.props();
                let layer = LayerState {
                    opacity: props.number_or(PropKey::Opacity, 1.0).clamp(0.0, 1.0),
                    transform: resolve_transform(props, node.frame.size()),
                    visible: props.flag_or(PropKey::Visible, true),
                };
                if layer != node.layer {
                    backend.set_layer(*id, &layer);
                    commands += 1;
                    if let Some(node) = self.nodes.get_mut(id) {
                        node.layer = layer;
                    }
                }
            }
        }

        // 4. Structure.
        for id in &touched {
            let Some(node) = self.nodes.get(id) else { continue };
            if node.dirty.contains(Dirty::CHILDREN) {
                let children = self.ordered_children(*id);
                backend.set_children(*id, &children);
                commands += 1;
            }
        }
        if self.root_dirty {
            self.root_dirty = false;
            let root = self.nodes.contains_key(&self.root).then_some(self.root);
            backend.set_window_root(window, root);
            commands += 1;
        }

        // 5. Geometry.
        for id in &touched {
            let Some(node) = self.nodes.get(id) else { continue };
            if node.dirty.contains(Dirty::FRAME) {
                let frame = self.relative_frame(node);
                if node.pushed_frame != Some(frame) {
                    backend.set_frame(*id, frame);
                    commands += 1;
                    if let Some(node) = self.nodes.get_mut(id) {
                        node.pushed_frame = Some(frame);
                    }
                }
            }
        }

        for id in touched {
            if let Some(node) = self.nodes.get_mut(&id) {
                node.dirty = Dirty::NONE;
            }
        }
        self.stats.commands += commands;
    }

    /// Size of the scrollable content of a scroll node (union of its children).
    #[must_use]
    pub fn content_size(&self, id: NodeId) -> Size {
        let Some(node) = self.nodes.get(&id) else { return Size::ZERO };
        let padding = node.props().padding();
        let mut size = Size::ZERO;
        for child in &node.children {
            if let Some(child) = self.nodes.get(child) {
                size.width = size.width.max(child.frame.right() - node.frame.x);
                size.height = size.height.max(child.frame.bottom() - node.frame.y);
            }
        }
        Size::new(size.width + padding.right, size.height + padding.bottom)
    }
}
