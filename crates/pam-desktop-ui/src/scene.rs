//! The retained scene: every node the engine mounted plus the chrome the
//! core synthesizes (tab bars, modal backdrops), their resolved paint state,
//! and the minimal set of backend commands needed after each batch of
//! mutations.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use pam_desktop_wire::{Mutation, Node, NodeKind, PropKey, PropValue};

use crate::accessibility::Accessibility;
use crate::animator::MotionProperty;
use crate::backend::{
    Backend, MAIN_WINDOW, NodeId, ScrollConfig, TextContent, VerticalAlign, ViewClass, WindowId,
};
use crate::color::Color;
use crate::geometry::{Affine, Insets, Point, Rect, Size};
use crate::props::{PropMap, Props};
use crate::shell::Cursor;
use crate::style::{
    Decoration, LayerState, compose_transform, resolve_transform, transform_origin,
};
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

/// Ids of nodes created by the core itself. PHP ids are positive signed
/// 64-bit integers, so the top bit never collides with them.
pub const SYNTHETIC_BIT: NodeId = 1 << 63;

/// Synthetic children are ordered after every engine child.
const SYNTHETIC_INDEX_BASE: u32 = 1 << 30;

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

/// What a press on a synthetic node does (it has no PHP handler).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    TabSelect {
        host: NodeId,
        index: u32,
    },
    ModalBackdrop {
        modal: NodeId,
    },
    NavigationBack {
        host: NodeId,
    },
    /// Flip a switch.
    Toggle {
        id: NodeId,
    },
    /// A button of the host's own chrome (error reports).
    Chrome {
        command: u16,
    },
}

/// Compositing applied on top of what the node's properties say. Several
/// independent sources (presentation, press feedback, motion) each own a slot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerOverride {
    /// Multiplies the node's opacity.
    pub opacity: f32,
    /// Applied after the node's own transform, in the same space.
    pub transform: Affine,
    pub hidden: bool,
}

impl Default for LayerOverride {
    fn default() -> Self {
        Self::NONE
    }
}

impl LayerOverride {
    pub const NONE: Self = Self {
        opacity: 1.0,
        transform: Affine::IDENTITY,
        hidden: false,
    };
    pub const HIDDEN: Self = Self {
        opacity: 1.0,
        transform: Affine::IDENTITY,
        hidden: true,
    };

    #[must_use]
    pub fn is_none(&self) -> bool {
        *self == Self::NONE
    }
}

/// Who owns a [`LayerOverride`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayerSlot {
    /// Navigation, tabs and modal presentation.
    Presenter = 0,
    /// Press and hover feedback.
    Interaction = 1,
}

const LAYER_SLOTS: usize = 2;

/// Who owns a style overlay (properties layered over the node's own).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StyleLayer {
    /// Interaction state styles (hover, focus, pressed) and control defaults.
    Interaction = 0,
    /// Presentation chrome (sheet corners) and animated fills.
    Presenter = 1,
}

const STYLE_LAYERS: usize = 2;

/// Presentation values an animation writes over a node's own properties:
/// absolute opacity, translation (logical units), scale factors and rotation
/// (degrees). `None` leaves the property to the node.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MotionValues {
    pub opacity: Option<f32>,
    pub translate_x: Option<f32>,
    pub translate_y: Option<f32>,
    pub scale_x: Option<f32>,
    pub scale_y: Option<f32>,
    pub rotate: Option<f32>,
}

impl MotionValues {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    #[must_use]
    pub fn get(&self, property: MotionProperty) -> Option<f32> {
        match property {
            MotionProperty::Opacity => self.opacity,
            MotionProperty::TranslateX => self.translate_x,
            MotionProperty::TranslateY => self.translate_y,
            MotionProperty::ScaleX => self.scale_x,
            MotionProperty::ScaleY => self.scale_y,
            MotionProperty::Rotate => self.rotate,
        }
    }

    pub fn set(&mut self, property: MotionProperty, value: Option<f32>) {
        match property {
            MotionProperty::Opacity => self.opacity = value,
            MotionProperty::TranslateX => self.translate_x = value,
            MotionProperty::TranslateY => self.translate_y = value,
            MotionProperty::ScaleX => self.scale_x = value,
            MotionProperty::ScaleY => self.scale_y = value,
            MotionProperty::Rotate => self.rotate = value,
        }
    }
}

/// The wire property behind each presentation property.
#[must_use]
pub fn motion_prop_key(property: MotionProperty) -> PropKey {
    match property {
        MotionProperty::Opacity => PropKey::Opacity,
        MotionProperty::TranslateX => PropKey::TranslationX,
        MotionProperty::TranslateY => PropKey::TranslationY,
        MotionProperty::ScaleX => PropKey::ScaleX,
        MotionProperty::ScaleY => PropKey::ScaleY,
        MotionProperty::Rotate => PropKey::Rotation,
    }
}

fn motion_property(key: PropKey) -> Option<MotionProperty> {
    match key {
        PropKey::Opacity => Some(MotionProperty::Opacity),
        PropKey::TranslationX => Some(MotionProperty::TranslateX),
        PropKey::TranslationY => Some(MotionProperty::TranslateY),
        PropKey::ScaleX => Some(MotionProperty::ScaleX),
        PropKey::ScaleY => Some(MotionProperty::ScaleY),
        PropKey::Rotation => Some(MotionProperty::Rotate),
        _ => None,
    }
}

/// The value a presentation property has when the node does not set it.
#[must_use]
pub fn motion_default(property: MotionProperty) -> f32 {
    match property {
        MotionProperty::Opacity | MotionProperty::ScaleX | MotionProperty::ScaleY => 1.0,
        MotionProperty::TranslateX | MotionProperty::TranslateY | MotionProperty::Rotate => 0.0,
    }
}

/// What changed in the scene during `apply`, for the controllers that react
/// to it (navigation, tabs, modals, lists, inputs, images…).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Change {
    Created(NodeId, NodeKind),
    Removed(NodeId, NodeKind),
    Updated(NodeId, PropKey),
    /// A presentation property changed; `previous` is the value the node had
    /// (`None` when it was unset). Reported in addition to `Updated`.
    Motion {
        id: NodeId,
        property: MotionProperty,
        previous: Option<f32>,
    },
    /// The node gained, lost or reordered children.
    Children(NodeId),
    Frame(NodeId),
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
    /// Final compositing state (properties and overrides combined).
    pub layer: LayerState,
    /// Set for chrome created by the core.
    pub action: Option<Action>,
    dirty: Dirty,
    /// Frame last pushed to the backend (parent relative).
    pushed_frame: Option<Rect>,
    has_text: bool,
    pushed_content: Option<Size>,
    pushed_scroll: Option<ScrollConfig>,
    pushed_focusable: bool,
    pushed_cursor: Cursor,
    has_tooltip: bool,
    /// Fingerprint of the accessibility last pushed; zero for the defaults.
    pushed_accessibility: u64,
}

impl SceneNode {
    #[must_use]
    pub fn props(&self) -> Props<'_> {
        Props::new(&self.props)
    }

    #[must_use]
    pub fn handles(&self, key: PropKey) -> bool {
        self.props.contains_key(&key)
    }

    #[must_use]
    pub fn is_text_bearing(&self) -> bool {
        matches!(self.kind, NodeKind::Text | NodeKind::Button)
    }

    #[must_use]
    pub fn is_synthetic(&self) -> bool {
        self.id & SYNTHETIC_BIT != 0
    }

    /// Reacts to presses, either through PHP handlers or a core action.
    #[must_use]
    pub fn is_pressable(&self) -> bool {
        self.action.is_some()
            || matches!(
                self.kind,
                NodeKind::Button | NodeKind::Pressable | NodeKind::Switch
            )
            || self.handles(PropKey::OnPress)
            || self.handles(PropKey::OnLongPress)
            || self.handles(PropKey::OnPressIn)
            || self.handles(PropKey::OnPressOut)
            || self.handles(PropKey::OnDoubleTap)
    }
}

/// The native object that backs each node kind.
#[must_use]
pub fn view_class(kind: NodeKind, props: Props<'_>) -> ViewClass {
    match kind {
        NodeKind::Scroll | NodeKind::List | NodeKind::SectionList | NodeKind::VirtualList => {
            ViewClass::Scroll
        }
        NodeKind::Input => {
            if props.flag_or(PropKey::Multiline, false) {
                ViewClass::TextArea
            } else {
                ViewClass::Input
            }
        }
        NodeKind::Canvas | NodeKind::DrawingCanvas => ViewClass::Surface,
        NodeKind::Media => ViewClass::Media,
        NodeKind::WebView => ViewClass::WebView,
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
        Self {
            scale: 1.0,
            font_scale: 1.0,
        }
    }
}

#[derive(Default)]
pub struct Scene {
    nodes: IdMap<SceneNode>,
    root: NodeId,
    root_dirty: bool,
    touched: Vec<NodeId>,
    removed: Vec<NodeId>,
    changes: Vec<Change>,
    layer_overrides: IdMap<[LayerOverride; LAYER_SLOTS]>,
    motion: IdMap<MotionValues>,
    /// Properties that win over the node's own while resolving paint state.
    style_overlays: IdMap<[Option<PropMap>; STYLE_LAYERS]>,
    scroll_offsets: IdMap<Point>,
    /// Views stacked above the root, back to front (presented modals).
    overlays: Vec<NodeId>,
    /// Host chrome above everything (error reports, toasts): parentless
    /// synthetic nodes placed by their own frame.
    host_overlays: Vec<NodeId>,
    /// Modal nodes shown as the root of a window of their own.
    window_roots: Vec<NodeId>,
    /// The window root pointer events are being resolved in; `None` is the
    /// main window.
    pointer_root: Option<NodeId>,
    overlays_dirty: bool,
    next_synthetic: u64,
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

    pub fn nodes(&self) -> impl Iterator<Item = &SceneNode> {
        self.nodes.values()
    }

    /// The changes recorded since the last call.
    pub fn take_changes(&mut self) -> Vec<Change> {
        std::mem::take(&mut self.changes)
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
        let known: std::collections::HashSet<NodeId> = ids.iter().copied().collect();
        ids.extend(self.nodes.keys().copied().filter(|id| !known.contains(id)));
        backend.set_window_overlays(MAIN_WINDOW, &[]);
        backend.set_window_root(MAIN_WINDOW, None);
        for id in ids {
            if let Some(node) = self.nodes.get(&id) {
                self.changes.push(Change::Removed(id, node.kind));
            }
            backend.destroy_view(id);
        }
        self.nodes.clear();
        self.touched.clear();
        self.removed.clear();
        self.layer_overrides.clear();
        self.motion.clear();
        self.style_overlays.clear();
        self.scroll_offsets.clear();
        self.overlays.clear();
        self.host_overlays.clear();
        self.window_roots.clear();
        self.pointer_root = None;
        self.overlays_dirty = false;
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
                    self.set_frame(id, Rect::new(frame.x, frame.y, frame.width, frame.height));
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

    fn set_frame(&mut self, id: NodeId, frame: Rect) {
        let children = match self.nodes.get_mut(&id) {
            Some(node) => {
                if node.frame == frame {
                    return;
                }
                node.frame = frame;
                node.children.clone()
            }
            None => return,
        };
        self.changes.push(Change::Frame(id));
        // A resize can change percent transforms and text wrapping.
        self.touch(id, Dirty::FRAME.union(Dirty::LAYER));
        // Children are positioned relative to this origin.
        for child in children {
            self.touch(child, Dirty::FRAME);
        }
    }

    fn insert(&mut self, node: Node, action: Option<Action>) {
        if self.nodes.contains_key(&node.id) {
            self.remove(node.id);
        }
        let class = view_class(node.kind, Props::new(&node.properties));
        let (id, parent, kind) = (node.id, node.parent, node.kind);
        self.nodes.insert(
            id,
            SceneNode {
                id,
                kind,
                parent,
                index: node.index,
                props: node.properties,
                children: Vec::new(),
                frame: Rect::ZERO,
                class,
                decoration: Decoration::default(),
                layer: LayerState::default(),
                action,
                dirty: Dirty::NONE,
                pushed_frame: None,
                has_text: false,
                pushed_content: None,
                pushed_scroll: None,
                pushed_focusable: false,
                pushed_cursor: Cursor::Default,
                has_tooltip: false,
                pushed_accessibility: 0,
            },
        );
        self.touch(id, Dirty::ALL);
        self.changes.push(Change::Created(id, kind));
        if let Some(parent_node) = self.nodes.get_mut(&parent) {
            parent_node.children.push(id);
            self.touch(parent, Dirty::CHILDREN);
            self.changes.push(Change::Children(parent));
        }
    }

    fn create(&mut self, node: Node) {
        self.insert(node, None);
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
        let Some(parent) = self.nodes.get(&id).map(|node| node.parent) else {
            return;
        };
        if let Some(parent_node) = self.nodes.get_mut(&parent) {
            parent_node.children.retain(|child| *child != id);
            self.touch(parent, Dirty::CHILDREN);
            self.changes.push(Change::Children(parent));
        }
        let mut subtree = Vec::new();
        self.collect_post_order(id, &mut subtree);
        for removed in subtree {
            if let Some(node) = self.nodes.remove(&removed) {
                self.changes.push(Change::Removed(removed, node.kind));
            }
            self.layer_overrides.remove(&removed);
            self.motion.remove(&removed);
            self.style_overlays.remove(&removed);
            self.scroll_offsets.remove(&removed);
            if let Some(position) = self.overlays.iter().position(|overlay| *overlay == removed) {
                self.overlays.remove(position);
                self.overlays_dirty = true;
            }
            if let Some(position) = self
                .host_overlays
                .iter()
                .position(|overlay| *overlay == removed)
            {
                self.host_overlays.remove(position);
                self.overlays_dirty = true;
            }
            self.removed.push(removed);
        }
        if self.root == id {
            self.root = 0;
            self.root_dirty = true;
        }
    }

    fn update(&mut self, id: NodeId, key: PropKey, value: Option<PropValue>) {
        let Some(node) = self.nodes.get_mut(&id) else {
            return;
        };
        let previous = node.props.get(&key).and_then(PropValue::as_number);
        match value {
            Some(value) => {
                if node.props.get(&key) == Some(&value) {
                    return;
                }
                node.props.insert(key, value);
            }
            None => {
                if node.props.remove(&key).is_none() {
                    return;
                }
            }
        }
        self.changes.push(Change::Updated(id, key));
        if let Some(property) = motion_property(key) {
            self.changes.push(Change::Motion {
                id,
                property,
                previous,
            });
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
        let Some(old_parent) = self.nodes.get(&id).map(|node| node.parent) else {
            return;
        };
        if old_parent != parent {
            if let Some(old) = self.nodes.get_mut(&old_parent) {
                old.children.retain(|child| *child != id);
            }
            if let Some(new) = self.nodes.get_mut(&parent) {
                new.children.push(id);
            }
            self.touch(old_parent, Dirty::CHILDREN);
            self.changes.push(Change::Children(old_parent));
        }
        if let Some(node) = self.nodes.get_mut(&id) {
            node.parent = parent;
            node.index = index;
        }
        self.touch(parent, Dirty::CHILDREN);
        self.changes.push(Change::Children(parent));
        self.touch(id, Dirty::FRAME.union(Dirty::TEXT));
    }

    // ---- chrome created by the core --------------------------------------

    /// Adds a node that exists only in the host: a tab bar item, a modal
    /// backdrop, a navigation header. It paints, hit-tests and animates like
    /// any other node; presses run `action` instead of a PHP handler.
    /// `order` sorts synthetic siblings; they follow every engine child unless
    /// their `ZIndex` says otherwise.
    pub fn add_synthetic(
        &mut self,
        parent: NodeId,
        kind: NodeKind,
        props: PropMap,
        frame: Rect,
        order: u32,
        action: Option<Action>,
    ) -> NodeId {
        self.next_synthetic += 1;
        let id = SYNTHETIC_BIT | self.next_synthetic;
        self.insert(
            Node {
                id,
                parent,
                index: SYNTHETIC_INDEX_BASE + order,
                kind,
                properties: props,
            },
            action,
        );
        self.set_frame(id, frame);
        id
    }

    /// Changes a property of any node from the host side.
    pub fn set_prop(&mut self, id: NodeId, key: PropKey, value: Option<PropValue>) {
        self.update(id, key, value);
    }

    /// Moves a synthetic node (frames are window coordinates, like the engine's).
    pub fn set_synthetic_frame(&mut self, id: NodeId, frame: Rect) {
        self.set_frame(id, frame);
    }

    pub fn remove_synthetic(&mut self, id: NodeId) {
        if id & SYNTHETIC_BIT != 0 {
            self.remove(id);
        }
    }

    // ---- overrides ---------------------------------------------------------

    pub fn set_layer_override(&mut self, id: NodeId, slot: LayerSlot, value: LayerOverride) {
        if !self.nodes.contains_key(&id) {
            return;
        }
        let slots = self
            .layer_overrides
            .entry(id)
            .or_insert([LayerOverride::NONE; LAYER_SLOTS]);
        if slots[slot as usize] == value {
            return;
        }
        slots[slot as usize] = value;
        if slots.iter().all(LayerOverride::is_none) {
            self.layer_overrides.remove(&id);
        }
        self.touch(id, Dirty::LAYER);
    }

    #[must_use]
    pub fn layer_override(&self, id: NodeId, slot: LayerSlot) -> LayerOverride {
        self.layer_overrides
            .get(&id)
            .map_or(LayerOverride::NONE, |slots| slots[slot as usize])
    }

    /// Presentation values written by animations; an empty set removes them.
    pub fn set_motion(&mut self, id: NodeId, values: MotionValues) {
        if !self.nodes.contains_key(&id) {
            return;
        }
        if values.is_empty() {
            if self.motion.remove(&id).is_none() {
                return;
            }
        } else if self.motion.insert(id, values) == Some(values) {
            return;
        }
        self.touch(id, Dirty::LAYER);
    }

    #[must_use]
    pub fn motion(&self, id: NodeId) -> MotionValues {
        self.motion.get(&id).copied().unwrap_or_default()
    }

    /// What a presentation property currently shows: the animated value, or
    /// the node's own.
    #[must_use]
    pub fn displayed(&self, id: NodeId, property: MotionProperty) -> f32 {
        if let Some(value) = self.motion.get(&id).and_then(|values| values.get(property)) {
            return value;
        }
        self.effective_props(id)
            .and_then(|props| props.number(motion_prop_key(property)))
            .unwrap_or_else(|| motion_default(property))
    }

    /// Replaces the properties one layer puts over a node's own (state
    /// styles, presentation chrome). `None` removes them.
    pub fn set_style_overlay(&mut self, id: NodeId, layer: StyleLayer, overlay: Option<PropMap>) {
        if !self.nodes.contains_key(&id) {
            return;
        }
        let overlay = overlay.filter(|overlay| !overlay.is_empty());
        let current = self
            .style_overlays
            .get(&id)
            .and_then(|layers| layers[layer as usize].as_ref());
        if current == overlay.as_ref() {
            return;
        }
        let layers = self
            .style_overlays
            .entry(id)
            .or_insert_with(|| [None, None]);
        layers[layer as usize] = overlay;
        if layers.iter().all(Option::is_none) {
            self.style_overlays.remove(&id);
        }
        self.touch(id, Dirty::DECORATION.union(Dirty::TEXT).union(Dirty::LAYER));
        // An overlay may change the paint order among siblings.
        if let Some(parent) = self.nodes.get(&id).map(|node| node.parent) {
            self.touch(parent, Dirty::CHILDREN);
        }
    }

    fn overlaid<'a>(&'a self, node: &'a SceneNode) -> Props<'a> {
        match self.style_overlays.get(&node.id) {
            Some(layers) => {
                Props::with_overlays(&node.props, [layers[0].as_ref(), layers[1].as_ref()])
            }
            None => Props::new(&node.props),
        }
    }

    /// The node's properties with its style overlays applied.
    #[must_use]
    pub fn effective_props(&self, id: NodeId) -> Option<Props<'_>> {
        self.nodes.get(&id).map(|node| self.overlaid(node))
    }

    /// Views stacked above the root view, back to front.
    pub fn set_overlays(&mut self, overlays: Vec<NodeId>) {
        if self.overlays != overlays {
            self.overlays = overlays;
            self.overlays_dirty = true;
        }
    }

    #[must_use]
    pub fn overlays(&self) -> &[NodeId] {
        &self.overlays
    }

    /// The modal nodes presented as roots of windows of their own.
    pub fn set_window_roots(&mut self, roots: Vec<NodeId>) {
        self.window_roots = roots;
    }

    /// Resolves the pointer in the window whose root is `root` (`None` for
    /// the main window) until told otherwise.
    pub fn set_pointer_root(&mut self, root: Option<NodeId>) {
        self.pointer_root = root.filter(|root| self.nodes.contains_key(root));
    }

    /// Replaces the host chrome shown above the application and its modals,
    /// back to front. Unlike modals these only take the pointer inside
    /// their own frame.
    pub fn set_host_overlays(&mut self, overlays: Vec<NodeId>) {
        if self.host_overlays != overlays {
            self.host_overlays = overlays;
            self.overlays_dirty = true;
        }
    }

    /// The paragraph a text node shows, exactly as the backend receives it.
    #[must_use]
    pub fn text_block(&self, id: NodeId) -> Option<TextBlock> {
        let node = self.nodes.get(&id)?;
        self.text_content(node, self.overlaid(node))
            .map(|content| content.block)
    }

    pub fn set_scroll_offset(&mut self, id: NodeId, offset: Point) {
        if self.nodes.contains_key(&id) {
            self.scroll_offsets.insert(id, offset);
        }
    }

    #[must_use]
    pub fn scroll_offset(&self, id: NodeId) -> Point {
        self.scroll_offsets.get(&id).copied().unwrap_or(Point::ZERO)
    }

    // ---- queries -----------------------------------------------------------

    /// Children in paint order: `zIndex` first, then document order. Modals
    /// are presented above the window, never inside their logical parent.
    #[must_use]
    pub fn ordered_children(&self, id: NodeId) -> Vec<NodeId> {
        let Some(node) = self.nodes.get(&id) else {
            return Vec::new();
        };
        let mut children: Vec<(f32, u32, NodeId)> = node
            .children
            .iter()
            .filter_map(|child| {
                let child_node = self.nodes.get(child)?;
                if child_node.kind == NodeKind::Modal {
                    return None;
                }
                // Overlays may raise a node (a pinned sticky header).
                Some((
                    self.overlaid(child_node).number_or(PropKey::ZIndex, 0.0),
                    child_node.index,
                    *child,
                ))
            })
            .collect();
        children.sort_by(|left, right| left.0.total_cmp(&right.0).then(left.1.cmp(&right.1)));
        children.into_iter().map(|(_, _, child)| child).collect()
    }

    /// True when `ancestor` is `id` or one of its ancestors.
    #[must_use]
    pub fn is_within(&self, id: NodeId, ancestor: NodeId) -> bool {
        let mut current = id;
        loop {
            if current == ancestor {
                return true;
            }
            match self.nodes.get(&current) {
                Some(node) if node.parent != 0 => current = node.parent,
                _ => return false,
            }
        }
    }

    /// Whether the node and every ancestor are shown.
    #[must_use]
    pub fn is_effectively_visible(&self, id: NodeId) -> bool {
        let mut current = id;
        loop {
            let Some(node) = self.nodes.get(&current) else {
                return false;
            };
            if !node.layer.visible {
                return false;
            }
            if node.kind == NodeKind::Modal {
                return self.overlays.contains(&current) || self.window_roots.contains(&current);
            }
            if node.parent == 0 {
                return true;
            }
            current = node.parent;
        }
    }

    /// The node named `reference` (its `NativeRef`): inside `scope` first,
    /// then anywhere.
    #[must_use]
    pub fn find_native_ref(&self, scope: NodeId, reference: &str) -> Option<NodeId> {
        let named = |id: &NodeId| {
            self.nodes
                .get(id)
                .is_some_and(|node| node.props().text(PropKey::NativeRef) == Some(reference))
        };
        let mut stack = vec![scope];
        while let Some(current) = stack.pop() {
            if named(&current) {
                return Some(current);
            }
            if let Some(node) = self.nodes.get(&current) {
                stack.extend(node.children.iter().rev());
            }
        }
        self.nodes.keys().copied().find(|id| named(id))
    }

    /// The text a node shows, without styling.
    #[must_use]
    pub fn plain_text(&self, id: NodeId) -> Option<String> {
        let node = self.nodes.get(&id)?;
        self.text_content(node, self.overlaid(node))
            .map(|content| content.block.plain_text())
    }

    /// How much of a node is on screen, 0..1: its box clipped by every
    /// scrolling or clipping ancestor and by the window. Zero when hidden.
    #[must_use]
    pub fn visible_fraction(&self, id: NodeId, viewport: Size) -> f32 {
        let Some(node) = self.nodes.get(&id) else {
            return 0.0;
        };
        let area = node.frame.width * node.frame.height;
        if area <= 0.0 || !self.is_effectively_visible(id) {
            return 0.0;
        }
        let origin = self.window_origin(id);
        let (mut left, mut top) = (origin.x.max(0.0), origin.y.max(0.0));
        let (mut right, mut bottom) = (
            (origin.x + node.frame.width).min(viewport.width),
            (origin.y + node.frame.height).min(viewport.height),
        );
        let mut current = node.parent;
        while let Some(ancestor) = self.nodes.get(&current) {
            if ancestor.class == ViewClass::Scroll || ancestor.decoration.clip {
                let origin = self.window_origin(current);
                left = left.max(origin.x);
                top = top.max(origin.y);
                right = right.min(origin.x + ancestor.frame.width);
                bottom = bottom.min(origin.y + ancestor.frame.height);
            }
            current = ancestor.parent;
        }
        ((right - left).max(0.0) * (bottom - top).max(0.0) / area).clamp(0.0, 1.0)
    }

    /// Deepest node under `point` (window coordinates), front to back. A
    /// presented modal takes every hit: nothing below it is reachable.
    #[must_use]
    pub fn hit_test(&self, point: Point) -> Option<NodeId> {
        // A secondary window only holds its own tree.
        if let Some(root) = self.pointer_root {
            return self.hit_test_node(root, point);
        }
        for overlay in self.host_overlays.iter().rev() {
            if let Some(hit) = self.hit_test_node(*overlay, point) {
                return Some(hit);
            }
        }
        if let Some(top) = self.overlays.last() {
            return self.hit_test_node(*top, point).or(Some(*top));
        }
        self.hit_test_node(self.root, point)
    }

    fn hit_test_node(&self, id: NodeId, point: Point) -> Option<NodeId> {
        let node = self.nodes.get(&id)?;
        if !node.layer.visible || node.layer.opacity <= 0.0 {
            return None;
        }
        let pointer_events = node.props().integer_or(PropKey::PointerEvents, 1);
        // 2 none: the subtree is transparent to the pointer.
        if pointer_events == 2 {
            return None;
        }
        // Undo the node's transform: frames are laid out untransformed.
        let point = if node.layer.transform.is_identity() {
            point
        } else {
            let origin = node.frame.origin();
            let inverse = node.layer.transform.invert()?;
            let local = inverse.apply(Point::new(point.x - origin.x, point.y - origin.y));
            Point::new(local.x + origin.x, local.y + origin.y)
        };
        let inside = node.frame.contains(point);
        let clips = node.decoration.clip || node.class == ViewClass::Scroll;
        if !inside && clips {
            return None;
        }
        // 4 box-only: children never take the hit.
        if pointer_events != 4 {
            // Scrolled content is laid out unscrolled: move the point into it.
            let child_point = match self.scroll_offsets.get(&id) {
                Some(offset) => point.offset(offset.x, offset.y),
                None => point,
            };
            for child in self.ordered_children(id).into_iter().rev() {
                if let Some(hit) = self.hit_test_node(child, child_point) {
                    return Some(hit);
                }
            }
        }
        // 3 box-none: the box itself is transparent, only children hit.
        (inside && pointer_events != 3).then_some(id)
    }

    /// The node's origin on screen: its frame moved by every scroll offset above it.
    #[must_use]
    /// Pushes the accessibility of a node when it changed; returns the
    /// number of backend commands issued.
    fn push_accessibility<B: Backend>(&mut self, backend: &mut B, id: NodeId) -> u64 {
        let Some(node) = self.nodes.get(&id) else {
            return 0;
        };
        if node.is_synthetic() {
            return 0;
        }
        let pressable = node.is_pressable();
        let mut resolved = Accessibility::resolve(node.kind, node.props(), pressable);
        if let Some(accessibility) = &mut resolved
            && pressable
            && accessibility.label.is_empty()
            && !node.is_text_bearing()
            && !accessibility.hidden
        {
            accessibility.label = self.descendant_text(id);
        }
        let fingerprint = resolved.as_ref().map_or(0, Accessibility::fingerprint);
        if fingerprint == node.pushed_accessibility {
            return 0;
        }
        backend.set_accessibility(id, resolved.as_ref());
        if let Some(node) = self.nodes.get_mut(&id) {
            node.pushed_accessibility = fingerprint;
        }
        1
    }

    /// The texts inside a node joined by ", ", bounded in size and effort.
    fn descendant_text(&self, id: NodeId) -> String {
        let mut label = String::new();
        let mut stack = vec![id];
        let mut visited = 0;
        while let Some(current) = stack.pop() {
            visited += 1;
            if visited > 64 || label.len() > 240 {
                break;
            }
            let Some(node) = self.nodes.get(&current) else {
                continue;
            };
            if current != id
                && let Some(text) = node
                    .is_text_bearing()
                    .then(|| node.props().text_or_empty(PropKey::Text))
                    .filter(|text| !text.is_empty())
            {
                if !label.is_empty() {
                    label.push_str(", ");
                }
                label.push_str(text);
                // Nested spans are part of the text already read.
                continue;
            }
            stack.extend(node.children.iter().rev());
        }
        label
    }

    /// The nearest pressable above a text node that has no label of its own.
    fn unnamed_pressable_ancestor(&self, id: NodeId) -> Option<NodeId> {
        let node = self.nodes.get(&id)?;
        if !node.is_text_bearing() {
            return None;
        }
        let mut current = node.parent;
        for _ in 0..8 {
            let ancestor = self.nodes.get(&current)?;
            if ancestor.is_pressable() && !ancestor.is_text_bearing() {
                return (!ancestor.handles(PropKey::AccessibilityLabel)).then_some(current);
            }
            current = ancestor.parent;
        }
        None
    }

    pub fn window_origin(&self, id: NodeId) -> Point {
        let Some(node) = self.nodes.get(&id) else {
            return Point::ZERO;
        };
        let mut origin = node.frame.origin();
        let mut current = node.parent;
        while let Some(ancestor) = self.nodes.get(&current) {
            if let Some(offset) = self.scroll_offsets.get(&current) {
                origin = origin.offset(-offset.x, -offset.y);
            }
            current = ancestor.parent;
        }
        origin
    }

    /// Frame relative to the parent view, snapped to the device pixel grid.
    fn relative_frame(&self, node: &SceneNode) -> Rect {
        let scale = self.environment.scale;
        let own = node.frame.snapped(scale);
        if node.kind == NodeKind::Modal {
            // Presented in the window, not inside its logical parent.
            return own;
        }
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
    fn text_align(&self, node: &SceneNode, props: Props<'_>) -> TextAlign {
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
        let Some(parent) = self.nodes.get(&node.parent) else {
            return TextAlign::Start;
        };
        let allocated = props.has(PropKey::Width)
            || props.has(PropKey::MinWidth)
            || props.number_or(PropKey::FlexGrow, 0.0) > 0.0;
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

    fn text_content(&self, node: &SceneNode, props: Props<'_>) -> Option<TextContent> {
        if !node.is_text_bearing() {
            return None;
        }
        let source = TextSource::from_props(node.kind, props, self.environment.font_scale);
        if source.text.is_empty() {
            return None;
        }
        let shadow_color = props
            .color(PropKey::TextShadowColor)
            .unwrap_or(Color::TRANSPARENT);
        let block = TextBlock {
            runs: source.runs(
                props.color(PropKey::TextColor),
                props.integer_or(PropKey::TextDecoration, 1),
            ),
            align: self.text_align(node, props),
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
            // Like the mobile renderers: a box taller than its text centers it.
            vertical: VerticalAlign::Center,
        })
    }

    fn resolve_layer(&self, node: &SceneNode, props: Props<'_>) -> LayerState {
        let size = node.frame.size();
        let mut layer = match self.motion.get(&node.id) {
            None => LayerState {
                opacity: props.number_or(PropKey::Opacity, 1.0).clamp(0.0, 1.0),
                transform: resolve_transform(props, size),
                visible: props.flag_or(PropKey::Visible, true),
            },
            // Animated values replace the node's own, component by component.
            Some(motion) => {
                let translate_x = motion
                    .translate_x
                    .unwrap_or_else(|| props.number_or(PropKey::TranslationX, 0.0))
                    + props.number_or(PropKey::TranslationXPercent, 0.0) / 100.0 * size.width;
                let translate_y = motion
                    .translate_y
                    .unwrap_or_else(|| props.number_or(PropKey::TranslationY, 0.0))
                    + props.number_or(PropKey::TranslationYPercent, 0.0) / 100.0 * size.height;
                LayerState {
                    opacity: motion
                        .opacity
                        .unwrap_or_else(|| props.number_or(PropKey::Opacity, 1.0))
                        .clamp(0.0, 1.0),
                    transform: compose_transform(
                        translate_x,
                        translate_y,
                        motion
                            .scale_x
                            .unwrap_or_else(|| props.number_or(PropKey::ScaleX, 1.0)),
                        motion
                            .scale_y
                            .unwrap_or_else(|| props.number_or(PropKey::ScaleY, 1.0)),
                        motion
                            .rotate
                            .unwrap_or_else(|| props.number_or(PropKey::Rotation, 0.0)),
                        transform_origin(props, size),
                    ),
                    visible: props.flag_or(PropKey::Visible, true),
                }
            }
        };
        if let Some(slots) = self.layer_overrides.get(&node.id) {
            for slot in slots {
                layer.opacity *= slot.opacity.clamp(0.0, 1.0);
                if !slot.transform.is_identity() {
                    layer.transform = layer.transform.then(&slot.transform);
                }
                layer.visible &= !slot.hidden;
            }
        }
        layer
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
            if let Some(node) = self.nodes.get(id)
                && node.dirty.contains(Dirty::CREATED)
            {
                backend.create_view(*id, node.class);
                commands += 1;
            }
        }

        // 3. Paint state.
        let mut relabel: Vec<NodeId> = Vec::new();
        for id in &touched {
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            let dirty = node.dirty;
            if dirty.contains(Dirty::DECORATION) {
                let decoration = Decoration::resolve(node.kind, self.overlaid(node));
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
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            let props = self.overlaid(node);
            if dirty.contains(Dirty::TEXT) && node.is_text_bearing() {
                let content = self.text_content(node, props);
                if content.is_some() || node.has_text {
                    backend.set_text(*id, content.as_ref());
                    commands += 1;
                }
                let has_text = content.is_some();
                if let Some(node) = self.nodes.get_mut(id) {
                    node.has_text = has_text;
                }
            }
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            let props = self.overlaid(node);
            if dirty.contains(Dirty::LAYER) {
                let layer = self.resolve_layer(node, props);
                if layer != node.layer {
                    backend.set_layer(*id, &layer);
                    commands += 1;
                    if let Some(node) = self.nodes.get_mut(id) {
                        node.layer = layer;
                    }
                }
            }
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            if dirty.contains(Dirty::CONFIG) {
                let props = node.props();
                // Selectable text shows the text cursor unless styled otherwise.
                let selectable =
                    node.is_text_bearing() && props.flag_or(PropKey::TextSelectable, false);
                let cursor = props.text(PropKey::Cursor).map_or(
                    if selectable {
                        Cursor::Text
                    } else {
                        Cursor::Default
                    },
                    Cursor::parse,
                );
                let tooltip = props.text(PropKey::Tooltip).filter(|text| !text.is_empty());
                let (cursor_changed, tooltip_changed) = (
                    cursor != node.pushed_cursor,
                    tooltip.is_some() || node.has_tooltip,
                );
                if cursor_changed {
                    backend.set_cursor(*id, cursor);
                    commands += 1;
                }
                if tooltip_changed {
                    backend.set_tooltip(*id, tooltip);
                    commands += 1;
                }
                let has_tooltip = tooltip.is_some();
                if (cursor_changed || tooltip_changed)
                    && let Some(node) = self.nodes.get_mut(id)
                {
                    node.pushed_cursor = cursor;
                    node.has_tooltip = has_tooltip;
                }
            }
            if dirty.contains(Dirty::CONFIG) {
                commands += self.push_accessibility(backend, *id);
            }
            if dirty.contains(Dirty::TEXT) {
                // A pressable is named after the text inside it.
                if let Some(ancestor) = self.unnamed_pressable_ancestor(*id)
                    && !relabel.contains(&ancestor)
                {
                    relabel.push(ancestor);
                }
            }
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            if dirty.contains(Dirty::CONFIG) && node.class == ViewClass::Box {
                // Pressable boxes take part in keyboard focus traversal.
                let focusable = node.is_pressable() && node.props().flag_or(PropKey::Enabled, true);
                if focusable != node.pushed_focusable {
                    backend.set_focusable(*id, focusable);
                    commands += 1;
                    if let Some(node) = self.nodes.get_mut(id) {
                        node.pushed_focusable = focusable;
                    }
                }
            }
        }

        for id in relabel {
            commands += self.push_accessibility(backend, id);
        }

        // 4. Structure.
        for id in &touched {
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
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
        if self.overlays_dirty {
            self.overlays_dirty = false;
            if self.host_overlays.is_empty() {
                backend.set_window_overlays(window, &self.overlays);
            } else {
                let stacked: Vec<NodeId> = self
                    .overlays
                    .iter()
                    .chain(&self.host_overlays)
                    .copied()
                    .collect();
                backend.set_window_overlays(window, &stacked);
            }
            commands += 1;
        }

        // 5. Geometry.
        for id in &touched {
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
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

        // 6. Scroll views: behaviour and content extent (after their children moved).
        let mut scrolls: Vec<NodeId> = Vec::new();
        for id in &touched {
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            if node.class == ViewClass::Scroll && !scrolls.contains(id) {
                scrolls.push(*id);
            }
            if (node.dirty.contains(Dirty::FRAME) || node.dirty.contains(Dirty::CREATED))
                && let Some(parent) = self.nodes.get(&node.parent)
                && parent.class == ViewClass::Scroll
                && !scrolls.contains(&parent.id)
            {
                scrolls.push(parent.id);
            }
        }
        for id in scrolls {
            let Some(node) = self.nodes.get(&id) else {
                continue;
            };
            let props = node.props();
            let config = ScrollConfig {
                horizontal: props.flag_or(PropKey::ScrollHorizontal, false)
                    || props.flag_or(PropKey::ListHorizontal, false),
                enabled: props.flag_or(PropKey::ScrollEnabled, true),
                shows_indicators: props.flag_or(PropKey::ShowsScrollIndicator, true),
            };
            let content = self.content_size(id);
            let (pushed_scroll, pushed_content) = (node.pushed_scroll, node.pushed_content);
            if pushed_scroll != Some(config) {
                backend.set_scroll_config(id, &config);
                commands += 1;
            }
            if pushed_content != Some(content) {
                backend.set_scroll_content(id, content);
                commands += 1;
            }
            if let Some(node) = self.nodes.get_mut(&id) {
                node.pushed_scroll = Some(config);
                node.pushed_content = Some(content);
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
        let Some(node) = self.nodes.get(&id) else {
            return Size::ZERO;
        };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Command, RecordingBackend};
    use pam_desktop_wire::Layout;

    fn node(
        id: u64,
        parent: u64,
        index: u32,
        kind: NodeKind,
        properties: &[(PropKey, PropValue)],
    ) -> Mutation {
        Mutation::Create(Node {
            id,
            parent,
            index,
            kind,
            properties: properties.iter().cloned().collect(),
        })
    }

    fn layout(id: u64, x: f32, y: f32, width: f32, height: f32) -> Mutation {
        Mutation::Layout {
            id,
            frame: Layout {
                x,
                y,
                width,
                height,
            },
        }
    }

    fn mounted() -> (Scene, RecordingBackend) {
        let mut scene = Scene::new();
        let mut backend = RecordingBackend::new();
        scene.apply(vec![
            Mutation::SetRoot { id: 1 },
            node(
                1,
                0,
                0,
                NodeKind::Column,
                &[(PropKey::BackgroundColor, PropValue::Integer(0xFFFF_FFFF))],
            ),
            node(
                2,
                1,
                0,
                NodeKind::Text,
                &[(PropKey::Text, PropValue::String("Hello".into()))],
            ),
            node(
                3,
                1,
                1,
                NodeKind::Button,
                &[(PropKey::Text, PropValue::String("Go".into()))],
            ),
            layout(1, 0.0, 0.0, 400.0, 300.0),
            layout(2, 16.0, 16.0, 368.0, 20.0),
            layout(3, 16.0, 52.0, 368.0, 48.0),
        ]);
        scene.flush(&mut backend, MAIN_WINDOW);
        (scene, backend)
    }

    #[test]
    fn mounts_a_tree_with_minimal_commands() {
        let (scene, backend) = mounted();
        assert_eq!(scene.len(), 3);
        assert_eq!(backend.children[&1], vec![2, 3]);
        assert_eq!(backend.frames[&2], Rect::new(16.0, 16.0, 368.0, 20.0));
        assert_eq!(backend.frames[&3], Rect::new(16.0, 52.0, 368.0, 48.0));
        assert_eq!(backend.texts[&2].block.plain_text(), "Hello");
        // Plain boxes never receive a decoration; the white root does.
        assert_eq!(
            backend.count(|command| matches!(command, Command::Decoration(..))),
            1
        );
        assert!(
            backend
                .commands
                .contains(&Command::SetWindowRoot(MAIN_WINDOW, Some(1)))
        );
        // Parents are created before their children.
        let position = |id| {
            backend
                .commands
                .iter()
                .position(|command| *command == Command::Create(id, ViewClass::Box))
        };
        assert!(position(1) < position(2) && position(2) < position(3));
    }

    #[test]
    fn a_text_update_touches_only_that_view() {
        let (mut scene, mut backend) = mounted();
        backend.take();
        scene.apply(vec![Mutation::Update {
            id: 2,
            key: PropKey::Text,
            value: Some(PropValue::String("Bye".into())),
        }]);
        scene.flush(&mut backend, MAIN_WINDOW);
        assert_eq!(backend.take(), vec![Command::Text(2, Some("Bye".into()))]);
        // Nothing changed: nothing is sent.
        scene.apply(vec![Mutation::Update {
            id: 1,
            key: PropKey::BackgroundColor,
            value: Some(PropValue::Integer(0xFFFF_FFFF)),
        }]);
        scene.flush(&mut backend, MAIN_WINDOW);
        assert!(backend.take().is_empty());
    }

    #[test]
    fn children_frames_are_relative_to_their_parent() {
        let (mut scene, mut backend) = mounted();
        backend.take();
        // The whole column moves: children keep their relative frames.
        scene.apply(vec![
            layout(1, 10.0, 20.0, 400.0, 300.0),
            layout(2, 26.0, 36.0, 368.0, 20.0),
            layout(3, 26.0, 72.0, 368.0, 48.0),
        ]);
        scene.flush(&mut backend, MAIN_WINDOW);
        assert_eq!(
            backend.take(),
            vec![Command::Frame(1, Rect::new(10.0, 20.0, 400.0, 300.0))]
        );
        // Only the parent moves: the child is repositioned inside it.
        scene.apply(vec![layout(1, 0.0, 0.0, 400.0, 300.0)]);
        scene.flush(&mut backend, MAIN_WINDOW);
        let commands = backend.take();
        assert!(commands.contains(&Command::Frame(2, Rect::new(26.0, 36.0, 368.0, 20.0))));
    }

    #[test]
    fn removes_subtrees_children_first_and_reorders_by_z_index() {
        let (mut scene, mut backend) = mounted();
        scene.apply(vec![node(4, 3, 0, NodeKind::View, &[])]);
        scene.flush(&mut backend, MAIN_WINDOW);
        backend.take();
        scene.apply(vec![Mutation::Remove { id: 3 }]);
        scene.flush(&mut backend, MAIN_WINDOW);
        assert_eq!(
            backend.take(),
            vec![
                Command::Destroy(4),
                Command::Destroy(3),
                Command::Children(1, vec![2])
            ]
        );
        assert_eq!(scene.len(), 2);

        scene.apply(vec![
            node(5, 1, 1, NodeKind::View, &[]),
            Mutation::Update {
                id: 2,
                key: PropKey::ZIndex,
                value: Some(PropValue::Integer(3)),
            },
        ]);
        scene.flush(&mut backend, MAIN_WINDOW);
        assert_eq!(backend.children[&1], vec![5, 2]);
    }

    #[test]
    fn moves_a_node_between_parents() {
        let (mut scene, mut backend) = mounted();
        scene.apply(vec![
            node(4, 1, 2, NodeKind::View, &[]),
            layout(4, 0.0, 100.0, 400.0, 100.0),
        ]);
        scene.flush(&mut backend, MAIN_WINDOW);
        scene.apply(vec![Mutation::Move {
            id: 2,
            parent: 4,
            index: 0,
        }]);
        scene.flush(&mut backend, MAIN_WINDOW);
        assert_eq!(backend.children[&4], vec![2]);
        assert_eq!(backend.children[&1], vec![3, 4]);
        assert_eq!(backend.frames[&2], Rect::new(16.0, -84.0, 368.0, 20.0));
    }

    #[test]
    fn scroll_views_receive_their_content_extent() {
        let mut scene = Scene::new();
        let mut backend = RecordingBackend::new();
        scene.apply(vec![
            Mutation::SetRoot { id: 1 },
            node(
                1,
                0,
                0,
                NodeKind::Scroll,
                &[(PropKey::PaddingBottom, PropValue::Integer(24))],
            ),
            node(2, 1, 0, NodeKind::Column, &[]),
            layout(1, 0.0, 0.0, 300.0, 200.0),
            layout(2, 0.0, 0.0, 300.0, 900.0),
        ]);
        scene.flush(&mut backend, MAIN_WINDOW);
        assert!(
            backend
                .commands
                .contains(&Command::Create(1, ViewClass::Scroll))
        );
        assert!(
            backend
                .commands
                .contains(&Command::ScrollContent(1, Size::new(300.0, 924.0)))
        );
        backend.take();
        scene.apply(vec![layout(2, 0.0, 0.0, 300.0, 1200.0)]);
        scene.flush(&mut backend, MAIN_WINDOW);
        assert!(
            backend
                .take()
                .contains(&Command::ScrollContent(1, Size::new(300.0, 1224.0)))
        );
    }

    #[test]
    fn unaligned_text_follows_its_flex_parent() {
        let mut scene = Scene::new();
        let mut backend = RecordingBackend::new();
        scene.apply(vec![
            Mutation::SetRoot { id: 1 },
            node(
                1,
                0,
                0,
                NodeKind::Column,
                &[(PropKey::AlignItems, PropValue::Integer(2))],
            ),
            node(
                2,
                1,
                0,
                NodeKind::Text,
                &[(PropKey::Text, PropValue::String("Centered".into()))],
            ),
            node(
                3,
                1,
                1,
                NodeKind::Text,
                &[
                    (PropKey::Text, PropValue::String("End".into())),
                    (PropKey::TextAlign, PropValue::Integer(3)),
                ],
            ),
        ]);
        scene.flush(&mut backend, MAIN_WINDOW);
        assert_eq!(backend.texts[&2].block.align, TextAlign::Center);
        assert_eq!(backend.texts[&3].block.align, TextAlign::End);
        scene.apply(vec![Mutation::Update {
            id: 1,
            key: PropKey::AlignItems,
            value: None,
        }]);
        scene.flush(&mut backend, MAIN_WINDOW);
        assert_eq!(backend.texts[&2].block.align, TextAlign::Start);
    }

    #[test]
    fn clearing_destroys_every_view() {
        let (mut scene, mut backend) = mounted();
        scene.clear(&mut backend);
        assert!(scene.is_empty());
        assert!(backend.views.is_empty());
    }

    #[test]
    fn layer_overrides_compose_with_properties() {
        let (mut scene, mut backend) = mounted();
        backend.take();
        scene.set_layer_override(
            2,
            LayerSlot::Presenter,
            LayerOverride {
                opacity: 0.5,
                transform: Affine::translation(100.0, 0.0),
                hidden: false,
            },
        );
        scene.apply(vec![Mutation::Update {
            id: 2,
            key: PropKey::Opacity,
            value: Some(PropValue::Float(0.5)),
        }]);
        scene.flush(&mut backend, MAIN_WINDOW);
        let layer = scene.node(2).unwrap().layer;
        assert_eq!(layer.opacity, 0.25);
        assert_eq!(layer.transform, Affine::translation(100.0, 0.0));
        scene.set_layer_override(2, LayerSlot::Presenter, LayerOverride::HIDDEN);
        scene.flush(&mut backend, MAIN_WINDOW);
        assert!(!scene.node(2).unwrap().layer.visible);
        assert_eq!(scene.hit_test(Point::new(20.0, 20.0)), Some(1));
        scene.set_layer_override(2, LayerSlot::Presenter, LayerOverride::NONE);
        scene.flush(&mut backend, MAIN_WINDOW);
        assert_eq!(scene.hit_test(Point::new(20.0, 20.0)), Some(2));
    }

    #[test]
    fn synthetic_nodes_paint_above_engine_children_and_carry_actions() {
        let (mut scene, mut backend) = mounted();
        let mut props = PropMap::new();
        props.insert(PropKey::BackgroundColor, PropValue::Integer(0xFF11_2233));
        let bar = scene.add_synthetic(
            1,
            NodeKind::View,
            props,
            Rect::new(0.0, 236.0, 400.0, 64.0),
            0,
            Some(Action::TabSelect { host: 1, index: 2 }),
        );
        scene.flush(&mut backend, MAIN_WINDOW);
        assert_eq!(backend.children[&1], vec![2, 3, bar]);
        assert_eq!(backend.frames[&bar], Rect::new(0.0, 236.0, 400.0, 64.0));
        assert_eq!(scene.hit_test(Point::new(10.0, 250.0)), Some(bar));
        assert!(scene.node(bar).unwrap().is_pressable());
        scene.remove_synthetic(bar);
        scene.flush(&mut backend, MAIN_WINDOW);
        assert_eq!(backend.children[&1], vec![2, 3]);
        assert!(!backend.views.contains_key(&bar));
    }

    #[test]
    fn modals_are_presented_as_window_overlays() {
        let (mut scene, mut backend) = mounted();
        scene.apply(vec![
            node(9, 3, 0, NodeKind::Modal, &[]),
            node(10, 9, 0, NodeKind::View, &[]),
            layout(9, 0.0, 0.0, 400.0, 300.0),
            layout(10, 100.0, 100.0, 200.0, 100.0),
        ]);
        scene.set_overlays(vec![9]);
        scene.flush(&mut backend, MAIN_WINDOW);
        // Not mounted inside its logical parent, and framed in window coordinates.
        assert!(backend.children.get(&3).is_none_or(Vec::is_empty));
        assert_eq!(backend.frames[&9], Rect::new(0.0, 0.0, 400.0, 300.0));
        assert!(
            backend
                .commands
                .contains(&Command::Overlays(MAIN_WINDOW, vec![9]))
        );
        // The presented modal takes every hit.
        assert_eq!(scene.hit_test(Point::new(150.0, 150.0)), Some(10));
        assert_eq!(scene.hit_test(Point::new(10.0, 10.0)), Some(9));
        scene.apply(vec![Mutation::Remove { id: 9 }]);
        scene.flush(&mut backend, MAIN_WINDOW);
        assert!(scene.overlays().is_empty());
    }

    #[test]
    fn hit_testing_honours_scroll_offsets_pointer_events_and_transforms() {
        let mut scene = Scene::new();
        let mut backend = RecordingBackend::new();
        scene.apply(vec![
            Mutation::SetRoot { id: 1 },
            node(1, 0, 0, NodeKind::Scroll, &[]),
            node(
                2,
                1,
                0,
                NodeKind::Column,
                &[(PropKey::PointerEvents, PropValue::Integer(3))],
            ),
            node(3, 2, 0, NodeKind::View, &[]),
            node(
                4,
                2,
                1,
                NodeKind::View,
                &[(PropKey::TranslationX, PropValue::Float(50.0))],
            ),
            layout(1, 0.0, 0.0, 200.0, 100.0),
            layout(2, 0.0, 0.0, 200.0, 400.0),
            layout(3, 0.0, 300.0, 100.0, 50.0),
            layout(4, 0.0, 0.0, 40.0, 40.0),
        ]);
        scene.flush(&mut backend, MAIN_WINDOW);
        // The column is box-none: an empty area falls through to the scroll view.
        assert_eq!(scene.hit_test(Point::new(150.0, 80.0)), Some(1));
        // Node 4 was translated 50 to the right.
        assert_eq!(scene.hit_test(Point::new(10.0, 10.0)), Some(1));
        assert_eq!(scene.hit_test(Point::new(60.0, 10.0)), Some(4));
        // Scrolled by 290: node 3 (content y 300) is now at window y 10.
        scene.set_scroll_offset(1, Point::new(0.0, 290.0));
        assert_eq!(scene.hit_test(Point::new(10.0, 20.0)), Some(3));
        assert_eq!(scene.window_origin(3), Point::new(0.0, 10.0));
    }

    #[test]
    fn style_overlays_win_while_resolving_paint_state() {
        let (mut scene, mut backend) = mounted();
        backend.take();
        let mut overlay = PropMap::new();
        overlay.insert(PropKey::BackgroundColor, PropValue::Integer(0xFF00_00FF));
        scene.set_style_overlay(1, StyleLayer::Interaction, Some(overlay));
        scene.flush(&mut backend, MAIN_WINDOW);
        assert_eq!(
            scene.node(1).unwrap().decoration.background,
            Color::from_argb(0xFF00_00FF)
        );
        scene.set_style_overlay(1, StyleLayer::Interaction, None);
        scene.flush(&mut backend, MAIN_WINDOW);
        assert_eq!(scene.node(1).unwrap().decoration.background, Color::WHITE);
    }

    #[test]
    fn motion_values_replace_properties_and_report_previous_values() {
        let (mut scene, mut backend) = mounted();
        scene.take_changes();
        scene.apply(vec![Mutation::Update {
            id: 2,
            key: PropKey::Opacity,
            value: Some(PropValue::Float(0.4)),
        }]);
        assert!(scene.take_changes().contains(&Change::Motion {
            id: 2,
            property: MotionProperty::Opacity,
            previous: None
        }));
        scene.apply(vec![Mutation::Update {
            id: 2,
            key: PropKey::Opacity,
            value: Some(PropValue::Float(0.8)),
        }]);
        assert!(scene.take_changes().contains(&Change::Motion {
            id: 2,
            property: MotionProperty::Opacity,
            previous: Some(0.4)
        }));
        assert_eq!(scene.displayed(2, MotionProperty::Opacity), 0.8);
        assert_eq!(scene.displayed(2, MotionProperty::ScaleX), 1.0);

        scene.set_motion(
            2,
            MotionValues {
                opacity: Some(0.1),
                translate_x: Some(30.0),
                ..MotionValues::default()
            },
        );
        scene.flush(&mut backend, MAIN_WINDOW);
        let layer = scene.node(2).unwrap().layer;
        assert_eq!(layer.opacity, 0.1);
        assert_eq!(layer.transform, Affine::translation(30.0, 0.0));
        assert_eq!(scene.displayed(2, MotionProperty::Opacity), 0.1);
        scene.set_motion(2, MotionValues::default());
        scene.flush(&mut backend, MAIN_WINDOW);
        assert_eq!(scene.node(2).unwrap().layer.opacity, 0.8);
    }

    #[test]
    fn changes_are_reported_once() {
        let (mut scene, _backend) = mounted();
        let changes = scene.take_changes();
        assert!(changes.contains(&Change::Created(2, NodeKind::Text)));
        assert!(changes.contains(&Change::Frame(3)));
        scene.apply(vec![
            Mutation::Update {
                id: 2,
                key: PropKey::Text,
                value: Some(PropValue::String("Hello".into())),
            },
            Mutation::Update {
                id: 2,
                key: PropKey::FontSize,
                value: Some(PropValue::Integer(20)),
            },
            Mutation::Remove { id: 3 },
        ]);
        assert_eq!(
            scene.take_changes(),
            vec![
                Change::Updated(2, PropKey::FontSize),
                Change::Children(1),
                Change::Removed(3, NodeKind::Button)
            ],
        );
        assert!(scene.take_changes().is_empty());
    }
}
