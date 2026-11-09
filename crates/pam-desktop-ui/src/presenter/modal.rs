//! `Modal` and bottom sheets, presented above the window content.
//!
//! A modal node covers the whole window. The host adds a backdrop behind its
//! content, animates the entrance and exit, and closes it optimistically on
//! Escape, a system back request or a backdrop click before telling PHP.

use pam_desktop_wire::{NodeKind, PropKey, PropValue};

use crate::animator::{Curve, Easing, Owner};
use crate::backend::{BackdropBoxes, NodeId, PointerButton, PointerEvent, PointerPhase};
use crate::context::{Ctx, event_kind};
use crate::geometry::{Affine, Point, Rect};
use crate::props::PropMap;
use crate::scene::{Action, Change, IdMap, LayerOverride, LayerSlot, StyleLayer};
use crate::style::{BackdropBox, Decoration};
use crate::wiremap::WireMap;

const PRESENTATION_FULL_SCREEN: i64 = 1;
const PRESENTATION_SHEET: i64 = 3;
/// A window layer (the toast stack): above every other modal, no backdrop,
/// never closed by Escape or the backdrop, presses outside its content pass
/// through.
const PRESENTATION_OVERLAY: i64 = 4;

const ANIMATION_NONE: i64 = 1;
const ANIMATION_SLIDE: i64 = 2;
const ANIMATION_FADE: i64 = 3;
const ANIMATION_SLIDE_FADE: i64 = 4;

const DEFAULT_BACKDROP: i64 = 0x5200_0000;
const HANDLE_COLOR: i64 = 0x7078_7880;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Hidden,
    Entering,
    Shown,
    Exiting,
}

#[derive(Debug)]
struct Modal {
    phase: Phase,
    backdrop: Option<NodeId>,
    handle: Option<NodeId>,
    /// `Visible` flipped (or the node appeared) in the commit being applied.
    sync: bool,
    restyle: bool,
    /// Animation style in use for the running entrance or exit.
    animation: i64,
    sheet: bool,
    overlay: bool,
    /// The dialog box whose outer shadow the backdrop composites.
    shadow_box: Option<NodeId>,
    /// Boxes inside it whose fills the backdrop composites too.
    inner_fills: Vec<NodeId>,
}

/// A bottom sheet being dragged by its top edge.
struct SheetDrag {
    modal: NodeId,
    start_y: f32,
    last_y: f32,
    last_ms: u64,
    /// Smoothed vertical speed, logical units per second.
    velocity: f32,
    /// The drag passed the threshold and owns the pointer.
    moving: bool,
}

#[derive(Default)]
pub struct Modals {
    modals: IdMap<Modal>,
    /// Presented modals, oldest first (the last one is on top).
    stack: Vec<NodeId>,
    drag: Option<SheetDrag>,
    dirty: bool,
}

/// The grab area at the top of a sheet.
const SHEET_GRIP: f32 = 48.0;
const SHEET_DRAG_SLOP: f32 = 4.0;
const SHEET_SETTLE_MS: u32 = 180;

/// `BottomSheetSnapPoints`: a count and that many fractions of the surface.
fn snap_points(bytes: Option<&[u8]>) -> Vec<f32> {
    let Some(bytes) = bytes.filter(|bytes| bytes.len() >= 2) else {
        return vec![0.5, 0.9];
    };
    let count = usize::from(u16::from_le_bytes([bytes[0], bytes[1]])).min(16);
    let snaps: Vec<f32> = bytes[2..]
        .as_chunks::<8>()
        .0
        .iter()
        .take(count)
        .map(|chunk| {
            f64::from_le_bytes([
                chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
            ]) as f32
        })
        .filter(|snap| snap.is_finite() && *snap > 0.0 && *snap <= 1.0)
        .collect();
    snaps
}

/// Kinds that paint nothing of their own with a plain decoration.
fn is_container(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::View
            | NodeKind::Column
            | NodeKind::Row
            | NodeKind::Scroll
            | NodeKind::Pressable
            | NodeKind::SafeAreaView
            | NodeKind::KeyboardAvoidingView
            | NodeKind::Spacer
    )
}

/// The first box painted in the modal content `content`, when it has an
/// outer shadow that falls straight on the backdrop (`backdrop`: the
/// backdrop's frame): only empty containers (no fill, border, transform,
/// opacity, scroll offset or z-index; clips no smaller than the backdrop)
/// are painted before it. Its shadow in the backdrop's coordinates.
fn shadow_box(ctx: &Ctx<'_>, content: NodeId, backdrop: Rect) -> Option<(NodeId, BackdropBox)> {
    let mut stack = vec![content];
    let mut visited = 0;
    while let Some(id) = stack.pop() {
        visited += 1;
        if visited > 256 {
            return None;
        }
        let node = ctx.scene.node(id)?;
        if !node.layer.visible {
            continue;
        }
        if node.layer.opacity < 0.999
            || !node.layer.transform.is_identity()
            || node
                .props()
                .integer(PropKey::ZIndex)
                .is_some_and(|z| z != 0)
        {
            return None;
        }
        // The node's own paint (the overlay that moves it is ignored).
        let decoration = Decoration::resolve(node.kind, node.props());
        let shadow = match decoration.shadows.as_slice() {
            [] => None,
            [shadow] if !shadow.inset => Some(*shadow),
            _ => return None,
        };
        let fill = (decoration.background.a > 0
            && decoration.background.a < 255
            && decoration.gradients.is_empty())
        .then_some(decoration.background);
        if shadow.is_some() || fill.is_some() {
            let rect = Rect::new(
                node.frame.x - backdrop.x,
                node.frame.y - backdrop.y,
                node.frame.width,
                node.frame.height,
            );
            return Some((
                id,
                BackdropBox {
                    rect,
                    radii: decoration.radii,
                    shadow,
                    fill,
                    clip: None,
                },
            ));
        }
        let clips = decoration.clip;
        let bare = Decoration {
            clip: false,
            ..decoration
        };
        if !is_container(node.kind) || !bare.is_plain() {
            return None;
        }
        if clips && node.frame.intersect(backdrop) != Some(backdrop) {
            return None;
        }
        if node.kind == NodeKind::Scroll && ctx.scene.scroll_offset(id) != Point::ZERO {
            return None;
        }
        stack.extend(node.children.iter().rev().copied());
    }
    None
}

/// Kinds that scroll (and clip) their content.
fn is_scroller(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Scroll | NodeKind::VirtualList | NodeKind::List | NodeKind::SectionList
    )
}

/// Boxes inside the dialog box `dialog` whose translucent plain fills
/// Chromium blends into the dialog's layer: reached through boxes that are
/// empty or themselves such fills (an opaque, gradient or shadowed box, an
/// own layer, opacity, transform, z-index or a scrolled view leaves its
/// subtree to the backend), each cut to its clipping ancestors.
fn inner_fills(ctx: &Ctx<'_>, dialog: NodeId, backdrop: Rect) -> Vec<(NodeId, BackdropBox)> {
    let local = |rect: Rect| {
        Rect::new(
            rect.x - backdrop.x,
            rect.y - backdrop.y,
            rect.width,
            rect.height,
        )
    };
    let Some(node) = ctx.scene.node(dialog) else {
        return Vec::new();
    };
    let own = Decoration::resolve(node.kind, node.props());
    let clip = own.clip.then(|| local(node.frame));
    let mut stack: Vec<(NodeId, Option<Rect>)> = node
        .children
        .iter()
        .rev()
        .map(|child| (*child, clip))
        .collect();
    let mut found = Vec::new();
    let mut visited = 0;
    while let Some((id, clip)) = stack.pop() {
        visited += 1;
        if visited > 512 || found.len() >= 64 {
            break;
        }
        let Some(node) = ctx.scene.node(id) else {
            continue;
        };
        let props = node.props();
        if !node.layer.visible
            || node.layer.opacity < 0.999
            || !node.layer.transform.is_identity()
            || props.integer(PropKey::ZIndex).is_some_and(|z| z != 0)
            || props.flag_or(PropKey::CompositedLayer, false)
            || (is_scroller(node.kind) && ctx.scene.scroll_offset(id) != Point::ZERO)
            // Hover, focus and press restyle these boxes later.
            || matches!(node.kind, NodeKind::Pressable | NodeKind::Button)
            || props.text(PropKey::NativeStateStyles).is_some_and(|styles| !styles.is_empty())
        {
            continue;
        }
        let decoration = Decoration::resolve(node.kind, props);
        let background = decoration.background;
        if !decoration.shadows.is_empty() || !decoration.gradients.is_empty() || background.a == 255
        {
            continue;
        }
        let frame = local(node.frame);
        if background.a > 0 {
            found.push((
                id,
                BackdropBox {
                    rect: frame,
                    radii: decoration.radii,
                    shadow: None,
                    fill: Some(background),
                    clip,
                },
            ));
        }
        let clips = decoration.clip || is_scroller(node.kind);
        let inner_clip = if clips {
            Some(clip.map_or(frame, |clip| clip.intersect(frame).unwrap_or(Rect::ZERO)))
        } else {
            clip
        };
        stack.extend(node.children.iter().rev().map(|child| (*child, inner_clip)));
    }
    found
}

fn content_of(ctx: &Ctx<'_>, modal: NodeId) -> Option<NodeId> {
    let node = ctx.scene.node(modal)?;
    node.children.iter().copied().find(|child| {
        ctx.scene
            .node(*child)
            .is_some_and(|node| !node.is_synthetic())
    })
}

impl Modals {
    pub fn on_change(&mut self, ctx: &mut Ctx<'_>, change: Change) {
        match change {
            // A modal marked as a window is shown by the window controller.
            Change::Created(id, NodeKind::Modal)
                if ctx
                    .scene
                    .node(id)
                    .is_some_and(|node| node.handles(PropKey::WindowSurface)) => {}
            Change::Created(id, NodeKind::Modal) => {
                self.modals.insert(
                    id,
                    Modal {
                        phase: Phase::Hidden,
                        backdrop: None,
                        handle: None,
                        sync: true,
                        restyle: false,
                        animation: ANIMATION_NONE,
                        sheet: false,
                        overlay: false,
                        shadow_box: None,
                        inner_fills: Vec::new(),
                    },
                );
                self.dirty = true;
            }
            Change::Removed(id, NodeKind::Modal) => {
                // Unmounting a visible modal closes it without a dismiss event.
                self.modals.remove(&id);
                self.stack.retain(|presented| *presented != id);
                if let Some(top) = self.top().and_then(|top| self.modals.get_mut(&top)) {
                    top.restyle = true;
                    self.dirty = true;
                }
                ctx.animator.cancel(Owner::ModalBackdrop(id), ctx.now_ms);
                ctx.animator.cancel(Owner::ModalContent(id), ctx.now_ms);
            }
            Change::Updated(id, key) => {
                let Some(modal) = self.modals.get_mut(&id) else {
                    return;
                };
                match key {
                    PropKey::Visible => modal.sync = true,
                    PropKey::ModalBackdropColor
                    | PropKey::BackdropBlurRadius
                    | PropKey::ModalTransparent
                    | PropKey::ModalPresentation
                    | PropKey::BottomSheetCornerRadius
                    | PropKey::BottomSheetHandleVisible => modal.restyle = true,
                    _ => return,
                }
                self.dirty = true;
            }
            Change::Frame(id) | Change::Children(id) => {
                // The modal or its content moved: chrome follows.
                let modal = if self.modals.contains_key(&id) {
                    Some(id)
                } else if self.modals.values().any(|modal| modal.shadow_box.is_some()) {
                    // A box deeper in a dialog moved: its shadow may be the
                    // one the backdrop composites.
                    let mut current = id;
                    let mut found = None;
                    for _ in 0..64 {
                        let Some(parent) = ctx.scene.node(current).map(|node| node.parent) else {
                            break;
                        };
                        if self.modals.contains_key(&parent) {
                            found = Some(parent);
                            break;
                        }
                        if parent == current {
                            break;
                        }
                        current = parent;
                    }
                    found
                } else {
                    ctx.scene
                        .node(id)
                        .map(|node| node.parent)
                        .filter(|parent| self.modals.contains_key(parent))
                };
                if let Some(modal) = modal.and_then(|modal| self.modals.get_mut(&modal)) {
                    modal.restyle = true;
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
        let ids: Vec<NodeId> = self.modals.keys().copied().collect();
        let mut entering = Vec::new();
        for id in ids {
            let Some(modal) = self.modals.get_mut(&id) else {
                continue;
            };
            let (sync, restyle) = (
                std::mem::take(&mut modal.sync),
                std::mem::take(&mut modal.restyle),
            );
            if sync {
                let visible = ctx
                    .scene
                    .node(id)
                    .is_some_and(|node| node.props().flag_or(PropKey::Visible, true));
                let presented = matches!(modal.phase, Phase::Entering | Phase::Shown);
                if visible && !presented {
                    entering.push(id);
                } else if !visible && presented {
                    self.dismiss(ctx, id);
                }
            } else if restyle
                && self
                    .modals
                    .get(&id)
                    .is_some_and(|modal| modal.phase != Phase::Hidden)
            {
                self.style(ctx, id);
            }
        }
        // Modals that appear in the same commit stack in tree order, like
        // dialogs opened one after another (a later sibling on top).
        if entering.len() > 1 {
            let order = ctx.scene.nodes_in_order();
            entering.sort_by_key(|id| order.iter().position(|node| node == id));
        }
        for id in entering {
            self.present(ctx, id);
        }
    }

    /// The topmost presented modal (window layers do not count).
    #[must_use]
    pub fn top(&self) -> Option<NodeId> {
        self.stack
            .iter()
            .rev()
            .find(|id| self.modals.get(id).is_none_or(|modal| !modal.overlay))
            .copied()
    }

    /// Presents the stack: window layers stay above the other modals.
    fn publish(&mut self, ctx: &mut Ctx<'_>) {
        let modals = &self.modals;
        let overlay = |id: &NodeId| modals.get(id).is_some_and(|modal| modal.overlay);
        self.stack.sort_by_key(overlay);
        let passive = self.stack.iter().copied().filter(overlay).collect();
        ctx.scene.set_overlays(self.stack.clone());
        ctx.scene.set_passive_overlays(passive);
        // Only the topmost dialog's backdrop composites its boxes.
        if ctx.scene.environment.backdrop_boxes == BackdropBoxes::Topmost {
            let presented: Vec<NodeId> = self
                .stack
                .iter()
                .copied()
                .filter(|id| {
                    self.modals
                        .get(id)
                        .is_some_and(|modal| modal.phase != Phase::Hidden && !modal.overlay)
                })
                .collect();
            for id in presented {
                self.style(ctx, id);
            }
        }
    }

    #[must_use]
    pub fn is_presented(&self, id: NodeId) -> bool {
        self.stack.contains(&id)
    }

    fn present(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(node) = ctx.scene.node(id) else {
            return;
        };
        let props = node.props();
        let presentation = props.integer_or(PropKey::ModalPresentation, 2);
        let sheet = presentation == PRESENTATION_SHEET;
        let overlay = presentation == PRESENTATION_OVERLAY;
        let animation = props.integer_or(PropKey::ModalAnimationType, ANIMATION_NONE);
        let Some(modal) = self.modals.get_mut(&id) else {
            return;
        };
        modal.overlay = overlay;
        ctx.animator.cancel(Owner::ModalBackdrop(id), ctx.now_ms);
        ctx.animator.cancel(Owner::ModalContent(id), ctx.now_ms);
        modal.sheet = sheet;
        modal.animation = animation;
        modal.phase = Phase::Entering;
        if modal.backdrop.is_none() && !overlay {
            let mut backdrop = PropMap::new();
            // Below the content, whatever its index.
            backdrop.insert(PropKey::ZIndex, PropValue::Integer(-1));
            modal.backdrop = Some(ctx.scene.add_synthetic(
                id,
                NodeKind::View,
                backdrop,
                Rect::ZERO,
                0,
                Some(Action::ModalBackdrop { modal: id }),
            ));
        }
        if !self.stack.contains(&id) {
            self.stack.push(id);
        }
        self.publish(ctx);
        ctx.scene
            .set_layer_override(id, LayerSlot::Presenter, LayerOverride::NONE);
        self.style(ctx, id);

        let (duration, backdrop_curve, content_curve) = entrance(animation, sheet);
        if duration == 0 || ctx.reduced_motion {
            self.apply(ctx, id, 1.0, 1.0);
            if let Some(modal) = self.modals.get_mut(&id) {
                modal.phase = Phase::Shown;
            }
        } else {
            self.apply(ctx, id, 0.0, 0.0);
            ctx.animator.start(
                Owner::ModalBackdrop(id),
                0.0,
                1.0,
                Curve::timing(duration, backdrop_curve),
                0,
                ctx.now_ms,
            );
            ctx.animator.start(
                Owner::ModalContent(id),
                0.0,
                1.0,
                Curve::timing(duration, content_curve),
                0,
                ctx.now_ms,
            );
        }
        ctx.emit_if(id, PropKey::OnModalShow, event_kind::MODAL_SHOW, Vec::new());
    }

    /// Starts the exit; the modal leaves the window when it ends.
    fn dismiss(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(modal) = self.modals.get_mut(&id) else {
            return;
        };
        if !matches!(modal.phase, Phase::Entering | Phase::Shown) {
            return;
        }
        modal.phase = Phase::Exiting;
        let (animation, sheet) = (modal.animation, modal.sheet);
        self.place_shadow(ctx, id, None, Vec::new());
        let backdrop_from = ctx
            .animator
            .cancel(Owner::ModalBackdrop(id), ctx.now_ms)
            .unwrap_or(1.0);
        let content_from = ctx
            .animator
            .cancel(Owner::ModalContent(id), ctx.now_ms)
            .unwrap_or(1.0);
        let (duration, backdrop_curve, content_curve) = exit(animation, sheet);
        if duration == 0 || ctx.reduced_motion {
            self.finish_exit(ctx, id);
        } else {
            ctx.animator.start(
                Owner::ModalBackdrop(id),
                backdrop_from,
                0.0,
                Curve::timing(duration, backdrop_curve),
                0,
                ctx.now_ms,
            );
            ctx.animator.start(
                Owner::ModalContent(id),
                content_from,
                0.0,
                Curve::timing(duration, content_curve),
                0,
                ctx.now_ms,
            );
        }
    }

    fn finish_exit(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(modal) = self.modals.get_mut(&id) else {
            return;
        };
        modal.phase = Phase::Hidden;
        let chrome = [modal.backdrop.take(), modal.handle.take()];
        for node in chrome.into_iter().flatten() {
            ctx.scene.remove_synthetic(node);
        }
        self.stack.retain(|presented| *presented != id);
        self.publish(ctx);
        if let Some(content) = content_of(ctx, id) {
            ctx.scene
                .set_layer_override(content, LayerSlot::Presenter, LayerOverride::NONE);
            ctx.scene
                .set_style_overlay(content, StyleLayer::Presenter, None);
        }
        ctx.emit_if(
            id,
            PropKey::OnModalDismiss,
            event_kind::MODAL_DISMISS,
            Vec::new(),
        );
    }

    /// Backdrop colour, sheet corners and grabber, and chrome geometry.
    fn style(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let topmost = self.top() == Some(id);
        let Some(node) = ctx.scene.node(id) else {
            return;
        };
        let props = node.props();
        let frame = node.frame;
        let transparent = props.flag_or(PropKey::ModalTransparent, false);
        let color = if transparent {
            0
        } else {
            props.integer_or(PropKey::ModalBackdropColor, DEFAULT_BACKDROP)
        };
        // `backdropBlurRadius` on a Modal frosts what is behind it, like a
        // CSS `backdrop-filter: blur()` mask.
        let blur = if transparent {
            0.0
        } else {
            props.number_or(PropKey::BackdropBlurRadius, 0.0).max(0.0)
        };
        let sheet = props.integer_or(PropKey::ModalPresentation, 2) == PRESENTATION_SHEET;
        let radius = props
            .number_or(PropKey::BottomSheetCornerRadius, 20.0)
            .clamp(0.0, 128.0);
        let shows_handle = sheet && props.flag_or(PropKey::BottomSheetHandleVisible, true);
        let content = content_of(ctx, id);
        let content_frame = content
            .and_then(|content| ctx.scene.node(content))
            .map(|node| node.frame);
        let Some(modal) = self.modals.get_mut(&id) else {
            return;
        };
        modal.sheet = sheet;
        if let Some(backdrop) = modal.backdrop {
            ctx.scene.set_synthetic_frame(backdrop, frame);
            ctx.scene.set_prop(
                backdrop,
                PropKey::BackgroundColor,
                Some(PropValue::Integer(color)),
            );
            ctx.scene.set_prop(
                backdrop,
                PropKey::BackdropBlurRadius,
                (blur > 0.0).then_some(PropValue::Float(f64::from(blur))),
            );
        }
        let translucent = (color >> 24) & 0xFF;
        // Only the topmost dialog: a lower one shows only through the
        // composited backdrop above it, which would keep three window-sized
        // composites for a stack of three.
        let shadow = (match ctx.scene.environment.backdrop_boxes {
            BackdropBoxes::None => false,
            BackdropBoxes::Topmost => topmost,
            BackdropBoxes::All => true,
        } && blur > 0.0
            && !sheet
            && translucent > 0
            && translucent < 255)
            .then(|| content.and_then(|content| shadow_box(ctx, content, frame)))
            .flatten();
        let inner = shadow
            .as_ref()
            .filter(|(_, painted)| painted.fill.is_some())
            .map(|(node, _)| inner_fills(ctx, *node, frame))
            .unwrap_or_default();
        if let Some(content) = content {
            // A sheet rounds its top corners and clips its content to them.
            let overlay = sheet.then(|| {
                let mut overlay = PropMap::new();
                overlay.insert(
                    PropKey::BorderTopLeftRadius,
                    PropValue::Float(f64::from(radius)),
                );
                overlay.insert(
                    PropKey::BorderTopRightRadius,
                    PropValue::Float(f64::from(radius)),
                );
                overlay.insert(PropKey::Overflow, PropValue::Integer(2));
                overlay
            });
            ctx.scene
                .set_style_overlay(content, StyleLayer::Presenter, overlay);
        }
        match (shows_handle, content_frame, modal.handle) {
            (true, Some(sheet_frame), existing) => {
                let handle_frame = Rect::new(
                    sheet_frame.x + (sheet_frame.width - 36.0) / 2.0,
                    sheet_frame.y + 10.0,
                    36.0,
                    4.0,
                );
                let handle = existing.unwrap_or_else(|| {
                    let mut handle = PropMap::new();
                    handle.insert(PropKey::BackgroundColor, PropValue::Integer(HANDLE_COLOR));
                    handle.insert(PropKey::BorderRadius, PropValue::Integer(2));
                    handle.insert(PropKey::PointerEvents, PropValue::Integer(2));
                    ctx.scene
                        .add_synthetic(id, NodeKind::View, handle, handle_frame, 1, None)
                });
                ctx.scene.set_synthetic_frame(handle, handle_frame);
                modal.handle = Some(handle);
            }
            (_, _, Some(handle)) => {
                ctx.scene.remove_synthetic(handle);
                modal.handle = None;
            }
            _ => {}
        }
        self.place_shadow(ctx, id, shadow, inner);
    }

    /// Moves the outer shadow of the dialog box `shadow` (or none) onto the
    /// backdrop of `id`: Chromium paints a dialog's shadow inside its mask's
    /// layer, blended in 8 bits with the mask's fill before the layer meets
    /// the blurred backdrop, which only a backend compositing the backdrop
    /// itself reproduces.
    fn place_shadow(
        &mut self,
        ctx: &mut Ctx<'_>,
        id: NodeId,
        shadow: Option<(NodeId, BackdropBox)>,
        inner: Vec<(NodeId, BackdropBox)>,
    ) {
        let Some(modal) = self.modals.get_mut(&id) else {
            return;
        };
        let next = shadow.as_ref().map(|(node, _)| *node);
        if modal.shadow_box != next
            && let Some(previous) = modal.shadow_box
        {
            ctx.scene
                .set_style_overlay(previous, StyleLayer::Presenter, None);
            ctx.scene.set_fill_composited(previous, false);
        }
        if let Some((node, painted)) = &shadow {
            // The box paints neither what the backdrop composites.
            let mut overlay = PropMap::new();
            if painted.shadow.is_some() {
                overlay.insert(PropKey::BoxShadows, PropValue::String("[]".to_owned()));
            }
            ctx.scene
                .set_style_overlay(*node, StyleLayer::Presenter, Some(overlay));
            ctx.scene.set_fill_composited(*node, painted.fill.is_some());
        }
        modal.shadow_box = next;
        let inner_ids: Vec<NodeId> = inner.iter().map(|(node, _)| *node).collect();
        for previous in &modal.inner_fills {
            if !inner_ids.contains(previous) {
                ctx.scene.set_fill_composited(*previous, false);
            }
        }
        for node in &inner_ids {
            ctx.scene.set_fill_composited(*node, true);
        }
        modal.inner_fills = inner_ids;
        if let Some(backdrop) = modal.backdrop {
            let boxes = shadow
                .map(|(_, painted)| {
                    std::iter::once(painted)
                        .chain(inner.into_iter().map(|(_, painted)| painted))
                        .collect()
                })
                .unwrap_or_default();
            ctx.scene.set_backdrop_boxes(backdrop, boxes);
        }
    }

    /// A view scrolled: boxes the backdrop composites inside it moved.
    pub fn on_scroll(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        if !self.modals.values().any(|modal| modal.shadow_box.is_some()) {
            return;
        }
        let mut current = id;
        for _ in 0..64 {
            if self.modals.contains_key(&current) {
                if self
                    .modals
                    .get(&current)
                    .is_some_and(|modal| modal.shadow_box.is_some())
                {
                    self.style(ctx, current);
                }
                return;
            }
            match ctx.scene.node(current).map(|node| node.parent) {
                Some(parent) if parent != current => current = parent,
                _ => return,
            }
        }
    }

    /// Frame-clock sample of a backdrop or content tween.
    pub fn on_sample(&mut self, ctx: &mut Ctx<'_>, owner: Owner, value: f32, finished: bool) {
        let id = owner.node();
        let Some(modal) = self.modals.get(&id) else {
            return;
        };
        let phase = modal.phase;
        match owner {
            Owner::ModalBackdrop(_) => self.apply_backdrop(ctx, id, value),
            Owner::ModalContent(_) => {
                self.apply_content(ctx, id, value);
                if finished {
                    match phase {
                        Phase::Entering => {
                            if let Some(modal) = self.modals.get_mut(&id) {
                                modal.phase = Phase::Shown;
                            }
                            self.style(ctx, id);
                        }
                        Phase::Exiting => self.finish_exit(ctx, id),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn apply(&self, ctx: &mut Ctx<'_>, id: NodeId, backdrop: f32, content: f32) {
        self.apply_backdrop(ctx, id, backdrop);
        self.apply_content(ctx, id, content);
    }

    fn apply_backdrop(&self, ctx: &mut Ctx<'_>, id: NodeId, progress: f32) {
        let Some(modal) = self.modals.get(&id) else {
            return;
        };
        let layer = LayerOverride {
            opacity: progress.clamp(0.0, 1.0),
            ..LayerOverride::NONE
        };
        if let Some(backdrop) = modal.backdrop {
            ctx.scene
                .set_layer_override(backdrop, LayerSlot::Presenter, layer);
        }
    }

    /// `progress` 0 = fully out, 1 = resting position.
    fn apply_content(&self, ctx: &mut Ctx<'_>, id: NodeId, progress: f32) {
        let Some(modal) = self.modals.get(&id) else {
            return;
        };
        let Some(content) = content_of(ctx, id) else {
            return;
        };
        let (Some(surface), Some(frame)) = (
            ctx.scene.node(id).map(|node| node.frame),
            ctx.scene.node(content).map(|node| node.frame),
        ) else {
            return;
        };
        let remaining = 1.0 - progress;
        let mut layer = LayerOverride::NONE;
        if modal.sheet {
            if modal.animation != ANIMATION_NONE {
                layer.transform = Affine::translation(0.0, (frame.height + 24.0) * remaining);
            }
        } else {
            match modal.animation {
                ANIMATION_SLIDE => {
                    layer.transform = Affine::translation(0.0, 0.12 * surface.height * remaining);
                    layer.opacity = progress.clamp(0.0, 1.0);
                }
                ANIMATION_FADE => layer.opacity = progress.clamp(0.0, 1.0),
                ANIMATION_SLIDE_FADE => {
                    let distance = (surface.bottom() - frame.y).max(surface.height / 2.0);
                    layer.transform = Affine::translation(0.0, distance * remaining);
                }
                _ => {}
            }
        }
        ctx.scene
            .set_layer_override(content, LayerSlot::Presenter, layer);
        if let Some(handle) = modal.handle {
            ctx.scene.set_layer_override(
                handle,
                LayerSlot::Presenter,
                LayerOverride {
                    transform: layer.transform,
                    ..LayerOverride::NONE
                },
            );
        }
    }

    /// Moves the content of a sheet by `offset` (0 = resting at its snap).
    fn apply_sheet_offset(&self, ctx: &mut Ctx<'_>, id: NodeId, offset: f32) {
        let value = if offset == 0.0 {
            LayerOverride::NONE
        } else {
            LayerOverride {
                transform: Affine::translation(0.0, offset),
                ..LayerOverride::NONE
            }
        };
        // The grabber is chrome of the modal: it travels with the sheet.
        let handle = self.modals.get(&id).and_then(|modal| modal.handle);
        for node in [content_of(ctx, id), handle].into_iter().flatten() {
            ctx.scene
                .set_layer_override(node, LayerSlot::Presenter, value);
        }
    }

    /// A frame of a released sheet returning to its snap.
    pub fn on_sheet_sample(&mut self, ctx: &mut Ctx<'_>, id: NodeId, offset: f32) {
        if self
            .modals
            .get(&id)
            .is_some_and(|modal| modal.phase == Phase::Shown)
        {
            self.apply_sheet_offset(ctx, id, offset);
        }
    }

    /// Dragging a bottom sheet by its top edge moves it between its snap
    /// points, or dismisses it. `Some(true)` means the drag just took the
    /// pointer (the press under it must be cancelled), `Some(false)` that
    /// it keeps it, `None` that the event is not for a sheet.
    pub fn on_pointer(&mut self, ctx: &mut Ctx<'_>, pointer: PointerEvent) -> Option<bool> {
        match pointer.phase {
            PointerPhase::Down if pointer.button == PointerButton::Primary => {
                self.drag = None;
                let id = self.top()?;
                let node = ctx.scene.node(id)?;
                let modal = self.modals.get(&id)?;
                if !modal.sheet
                    || modal.phase != Phase::Shown
                    || !node.props().flag_or(PropKey::BottomSheetDragEnabled, true)
                {
                    return None;
                }
                let frame = ctx.scene.node(content_of(ctx, id)?)?.frame;
                let inside = pointer.position.x >= frame.x
                    && pointer.position.x <= frame.right()
                    && pointer.position.y >= frame.y
                    && pointer.position.y <= frame.y + SHEET_GRIP;
                if inside {
                    ctx.animator.cancel(Owner::SheetDrag(id), ctx.now_ms);
                    self.drag = Some(SheetDrag {
                        modal: id,
                        start_y: pointer.position.y,
                        last_y: pointer.position.y,
                        last_ms: ctx.now_ms,
                        velocity: 0.0,
                        moving: false,
                    });
                }
                None
            }
            PointerPhase::Move => {
                let drag = self.drag.as_mut()?;
                let elapsed = ctx.now_ms.saturating_sub(drag.last_ms);
                if elapsed > 0 {
                    let instant = (pointer.position.y - drag.last_y) * 1000.0 / elapsed as f32;
                    drag.velocity = drag.velocity * 0.4 + instant * 0.6;
                    drag.last_ms = ctx.now_ms;
                }
                drag.last_y = pointer.position.y;
                let travel = pointer.position.y - drag.start_y;
                let started = !drag.moving && travel.abs() >= SHEET_DRAG_SLOP;
                if !drag.moving && !started {
                    return None;
                }
                drag.moving = true;
                let id = drag.modal;
                // The sheet cannot grow while it is held: upward travel only hints.
                self.apply_sheet_offset(ctx, id, if travel > 0.0 { travel } else { travel * 0.25 });
                Some(started)
            }
            PointerPhase::Up | PointerPhase::Cancel => {
                let drag = self.drag.take().filter(|drag| drag.moving)?;
                let id = drag.modal;
                let travel = pointer.position.y - drag.start_y;
                let offset = if travel > 0.0 { travel } else { travel * 0.25 };
                let node = ctx.scene.node(id)?;
                let props = node.props();
                let surface = node.frame.height.max(1.0);
                let snaps = snap_points(props.bytes(PropKey::BottomSheetSnapPoints));
                let index = usize::try_from(props.integer_or(PropKey::BottomSheetIndex, 0))
                    .unwrap_or(0)
                    .min(snaps.len().saturating_sub(1));
                let dismissible = props.flag_or(PropKey::BottomSheetDismissible, true);
                let current = snaps.get(index).copied().unwrap_or(1.0);
                let projected = current - (travel + drag.velocity * 0.12) / surface;
                let cancelled = pointer.phase == PointerPhase::Cancel;
                if !cancelled && dismissible && index == 0 && projected < current * 0.55 {
                    ctx.emit_if(
                        id,
                        PropKey::OnBottomSheetDismiss,
                        event_kind::BOTTOM_SHEET_DISMISS,
                        Vec::new(),
                    );
                    // The exit starts from where the sheet was released.
                    self.apply_sheet_offset(ctx, id, 0.0);
                    self.request_close(ctx, id, false);
                    return Some(false);
                }
                if !cancelled && !snaps.is_empty() {
                    let nearest = snaps
                        .iter()
                        .enumerate()
                        .min_by(|a, b| (a.1 - projected).abs().total_cmp(&(b.1 - projected).abs()))
                        .map_or(index, |(nearest, _)| nearest);
                    let payload = WireMap::new()
                        .with("index", nearest as i64)
                        .with("position", snaps[nearest])
                        .encode();
                    ctx.emit_if(
                        id,
                        PropKey::OnBottomSheetChange,
                        event_kind::BOTTOM_SHEET_CHANGE,
                        payload,
                    );
                }
                if ctx.reduced_motion {
                    self.apply_sheet_offset(ctx, id, 0.0);
                } else {
                    ctx.animator.start(
                        Owner::SheetDrag(id),
                        offset,
                        0.0,
                        Curve::timing(SHEET_SETTLE_MS, Easing::EASE_OUT),
                        0,
                        ctx.now_ms,
                    );
                }
                Some(false)
            }
            _ => None,
        }
    }

    /// The user asked to close the top modal (Escape, system back). Returns
    /// whether a modal consumed the request.
    pub fn request_close_top(&mut self, ctx: &mut Ctx<'_>) -> bool {
        match self.top() {
            Some(id) => {
                self.request_close(ctx, id, false);
                true
            }
            None => false,
        }
    }

    /// The backdrop of `id` was clicked.
    pub fn backdrop_pressed(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        self.request_close(ctx, id, true);
    }

    fn request_close(&mut self, ctx: &mut Ctx<'_>, id: NodeId, from_backdrop: bool) {
        let Some(node) = ctx.scene.node(id) else {
            return;
        };
        let props = node.props();
        let presentation = props.integer_or(PropKey::ModalPresentation, 2);
        let sheet = presentation == PRESENTATION_SHEET;
        if sheet && !props.flag_or(PropKey::BottomSheetDismissible, true) {
            return;
        }
        if from_backdrop {
            if sheet && !props.flag_or(PropKey::BottomSheetBackdropDismiss, true) {
                return;
            }
            if presentation == PRESENTATION_FULL_SCREEN {
                return;
            }
        }
        let notifies_native = node.handles(PropKey::OnNativeEvent);
        // Close first, then tell PHP: the window never waits for a round trip.
        self.dismiss(ctx, id);
        ctx.emit_if(
            id,
            PropKey::OnModalRequestClose,
            event_kind::MODAL_REQUEST_CLOSE,
            Vec::new(),
        );
        if notifies_native {
            let payload = WireMap::new()
                .with("action", 1_i64)
                .with("dismissed", true)
                .encode();
            ctx.emit(id, event_kind::NATIVE, payload);
        }
    }
}

/// Duration and curves (backdrop, content) of an entrance.
fn entrance(animation: i64, sheet: bool) -> (u32, Easing, Easing) {
    if animation == ANIMATION_NONE {
        return (0, Easing::Linear, Easing::Linear);
    }
    if sheet {
        return (250, Easing::Linear, Easing::EASE_OUT);
    }
    match animation {
        ANIMATION_SLIDE_FADE => (320, Easing::Linear, Easing::OutCubic),
        _ => (225, Easing::EASE_OUT, Easing::EASE_OUT),
    }
}

fn exit(animation: i64, sheet: bool) -> (u32, Easing, Easing) {
    if animation == ANIMATION_NONE {
        return (0, Easing::Linear, Easing::Linear);
    }
    if sheet {
        return (200, Easing::Linear, Easing::EASE_IN);
    }
    match animation {
        ANIMATION_SLIDE_FADE => (220, Easing::Linear, Easing::InQuad),
        _ => (125, Easing::EASE_IN, Easing::EASE_IN),
    }
}
