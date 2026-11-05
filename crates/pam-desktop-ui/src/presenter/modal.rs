//! `Modal` and bottom sheets, presented above the window content.
//!
//! A modal node covers the whole window. The host adds a backdrop behind its
//! content, animates the entrance and exit, and closes it optimistically on
//! Escape, a system back request or a backdrop click before telling PHP.

use pam_desktop_wire::{NodeKind, PropKey, PropValue};

use crate::animator::{Curve, Easing, Owner};
use crate::backend::NodeId;
use crate::context::{event_kind, Ctx};
use crate::geometry::{Affine, Rect};
use crate::props::PropMap;
use crate::scene::{Action, Change, IdMap, LayerOverride, LayerSlot, StyleLayer};
use crate::wiremap::WireMap;

const PRESENTATION_FULL_SCREEN: i64 = 1;
const PRESENTATION_SHEET: i64 = 3;

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
}

#[derive(Default)]
pub struct Modals {
    modals: IdMap<Modal>,
    /// Presented modals, oldest first (the last one is on top).
    stack: Vec<NodeId>,
    dirty: bool,
}

fn content_of(ctx: &Ctx<'_>, modal: NodeId) -> Option<NodeId> {
    let node = ctx.scene.node(modal)?;
    node.children.iter().copied().find(|child| ctx.scene.node(*child).is_some_and(|node| !node.is_synthetic()))
}

impl Modals {
    pub fn on_change(&mut self, ctx: &mut Ctx<'_>, change: Change) {
        match change {
            // A modal marked as a window is shown by the window controller.
            Change::Created(id, NodeKind::Modal) if ctx.scene.node(id).is_some_and(|node| node.handles(PropKey::WindowSurface)) => {}
            Change::Created(id, NodeKind::Modal) => {
                self.modals.insert(
                    id,
                    Modal { phase: Phase::Hidden, backdrop: None, handle: None, sync: true, restyle: false, animation: ANIMATION_NONE, sheet: false },
                );
                self.dirty = true;
            }
            Change::Removed(id, NodeKind::Modal) => {
                // Unmounting a visible modal closes it without a dismiss event.
                self.modals.remove(&id);
                self.stack.retain(|presented| *presented != id);
                ctx.animator.cancel(Owner::ModalBackdrop(id), ctx.now_ms);
                ctx.animator.cancel(Owner::ModalContent(id), ctx.now_ms);
            }
            Change::Updated(id, key) => {
                let Some(modal) = self.modals.get_mut(&id) else { return };
                match key {
                    PropKey::Visible => modal.sync = true,
                    PropKey::ModalBackdropColor
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
                } else {
                    ctx.scene.node(id).map(|node| node.parent).filter(|parent| self.modals.contains_key(parent))
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
        for id in ids {
            let Some(modal) = self.modals.get_mut(&id) else { continue };
            let (sync, restyle) = (std::mem::take(&mut modal.sync), std::mem::take(&mut modal.restyle));
            if sync {
                let visible = ctx.scene.node(id).is_some_and(|node| node.props().flag_or(PropKey::Visible, true));
                let presented = matches!(modal.phase, Phase::Entering | Phase::Shown);
                if visible && !presented {
                    self.present(ctx, id);
                } else if !visible && presented {
                    self.dismiss(ctx, id);
                }
            } else if restyle && self.modals.get(&id).is_some_and(|modal| modal.phase != Phase::Hidden) {
                self.style(ctx, id);
            }
        }
    }

    /// The topmost presented modal.
    #[must_use]
    pub fn top(&self) -> Option<NodeId> {
        self.stack.last().copied()
    }

    #[must_use]
    pub fn is_presented(&self, id: NodeId) -> bool {
        self.stack.contains(&id)
    }

    fn present(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(node) = ctx.scene.node(id) else { return };
        let props = node.props();
        let sheet = props.integer_or(PropKey::ModalPresentation, 2) == PRESENTATION_SHEET;
        let animation = props.integer_or(PropKey::ModalAnimationType, ANIMATION_NONE);
        let Some(modal) = self.modals.get_mut(&id) else { return };
        ctx.animator.cancel(Owner::ModalBackdrop(id), ctx.now_ms);
        ctx.animator.cancel(Owner::ModalContent(id), ctx.now_ms);
        modal.sheet = sheet;
        modal.animation = animation;
        modal.phase = Phase::Entering;
        if modal.backdrop.is_none() {
            let mut backdrop = PropMap::new();
            // Below the content, whatever its index.
            backdrop.insert(PropKey::ZIndex, PropValue::Integer(-1));
            modal.backdrop =
                Some(ctx.scene.add_synthetic(id, NodeKind::View, backdrop, Rect::ZERO, 0, Some(Action::ModalBackdrop { modal: id })));
        }
        if !self.stack.contains(&id) {
            self.stack.push(id);
        }
        ctx.scene.set_overlays(self.stack.clone());
        ctx.scene.set_layer_override(id, LayerSlot::Presenter, LayerOverride::NONE);
        self.style(ctx, id);

        let (duration, backdrop_curve, content_curve) = entrance(animation, sheet);
        if duration == 0 || ctx.reduced_motion {
            self.apply(ctx, id, 1.0, 1.0);
            if let Some(modal) = self.modals.get_mut(&id) {
                modal.phase = Phase::Shown;
            }
        } else {
            self.apply(ctx, id, 0.0, 0.0);
            ctx.animator.start(Owner::ModalBackdrop(id), 0.0, 1.0, Curve::timing(duration, backdrop_curve), 0, ctx.now_ms);
            ctx.animator.start(Owner::ModalContent(id), 0.0, 1.0, Curve::timing(duration, content_curve), 0, ctx.now_ms);
        }
        ctx.emit_if(id, PropKey::OnModalShow, event_kind::MODAL_SHOW, Vec::new());
    }

    /// Starts the exit; the modal leaves the window when it ends.
    fn dismiss(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(modal) = self.modals.get_mut(&id) else { return };
        if !matches!(modal.phase, Phase::Entering | Phase::Shown) {
            return;
        }
        modal.phase = Phase::Exiting;
        let (animation, sheet) = (modal.animation, modal.sheet);
        let backdrop_from = ctx.animator.cancel(Owner::ModalBackdrop(id), ctx.now_ms).unwrap_or(1.0);
        let content_from = ctx.animator.cancel(Owner::ModalContent(id), ctx.now_ms).unwrap_or(1.0);
        let (duration, backdrop_curve, content_curve) = exit(animation, sheet);
        if duration == 0 || ctx.reduced_motion {
            self.finish_exit(ctx, id);
        } else {
            ctx.animator.start(Owner::ModalBackdrop(id), backdrop_from, 0.0, Curve::timing(duration, backdrop_curve), 0, ctx.now_ms);
            ctx.animator.start(Owner::ModalContent(id), content_from, 0.0, Curve::timing(duration, content_curve), 0, ctx.now_ms);
        }
    }

    fn finish_exit(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(modal) = self.modals.get_mut(&id) else { return };
        modal.phase = Phase::Hidden;
        let chrome = [modal.backdrop.take(), modal.handle.take()];
        for node in chrome.into_iter().flatten() {
            ctx.scene.remove_synthetic(node);
        }
        self.stack.retain(|presented| *presented != id);
        ctx.scene.set_overlays(self.stack.clone());
        if let Some(content) = content_of(ctx, id) {
            ctx.scene.set_layer_override(content, LayerSlot::Presenter, LayerOverride::NONE);
            ctx.scene.set_style_overlay(content, StyleLayer::Presenter, None);
        }
        ctx.emit_if(id, PropKey::OnModalDismiss, event_kind::MODAL_DISMISS, Vec::new());
    }

    /// Backdrop colour, sheet corners and grabber, and chrome geometry.
    fn style(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(node) = ctx.scene.node(id) else { return };
        let props = node.props();
        let frame = node.frame;
        let transparent = props.flag_or(PropKey::ModalTransparent, false);
        let color = if transparent { 0 } else { props.integer_or(PropKey::ModalBackdropColor, DEFAULT_BACKDROP) };
        let sheet = props.integer_or(PropKey::ModalPresentation, 2) == PRESENTATION_SHEET;
        let radius = props.number_or(PropKey::BottomSheetCornerRadius, 20.0).clamp(0.0, 128.0);
        let shows_handle = sheet && props.flag_or(PropKey::BottomSheetHandleVisible, true);
        let content = content_of(ctx, id);
        let content_frame = content.and_then(|content| ctx.scene.node(content)).map(|node| node.frame);
        let Some(modal) = self.modals.get_mut(&id) else { return };
        modal.sheet = sheet;
        if let Some(backdrop) = modal.backdrop {
            ctx.scene.set_synthetic_frame(backdrop, frame);
            ctx.scene.set_prop(backdrop, PropKey::BackgroundColor, Some(PropValue::Integer(color)));
        }
        if let Some(content) = content {
            // A sheet rounds its top corners and clips its content to them.
            let overlay = sheet.then(|| {
                let mut overlay = PropMap::new();
                overlay.insert(PropKey::BorderTopLeftRadius, PropValue::Float(f64::from(radius)));
                overlay.insert(PropKey::BorderTopRightRadius, PropValue::Float(f64::from(radius)));
                overlay.insert(PropKey::Overflow, PropValue::Integer(2));
                overlay
            });
            ctx.scene.set_style_overlay(content, StyleLayer::Presenter, overlay);
        }
        match (shows_handle, content_frame, modal.handle) {
            (true, Some(sheet_frame), existing) => {
                let handle_frame = Rect::new(sheet_frame.x + (sheet_frame.width - 36.0) / 2.0, sheet_frame.y + 10.0, 36.0, 4.0);
                let handle = existing.unwrap_or_else(|| {
                    let mut handle = PropMap::new();
                    handle.insert(PropKey::BackgroundColor, PropValue::Integer(HANDLE_COLOR));
                    handle.insert(PropKey::BorderRadius, PropValue::Integer(2));
                    handle.insert(PropKey::PointerEvents, PropValue::Integer(2));
                    ctx.scene.add_synthetic(id, NodeKind::View, handle, handle_frame, 1, None)
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
    }

    /// Frame-clock sample of a backdrop or content tween.
    pub fn on_sample(&mut self, ctx: &mut Ctx<'_>, owner: Owner, value: f32, finished: bool) {
        let id = owner.node();
        let Some(modal) = self.modals.get(&id) else { return };
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
        let Some(modal) = self.modals.get(&id) else { return };
        let layer = LayerOverride { opacity: progress.clamp(0.0, 1.0), ..LayerOverride::NONE };
        if let Some(backdrop) = modal.backdrop {
            ctx.scene.set_layer_override(backdrop, LayerSlot::Presenter, layer);
        }
    }

    /// `progress` 0 = fully out, 1 = resting position.
    fn apply_content(&self, ctx: &mut Ctx<'_>, id: NodeId, progress: f32) {
        let Some(modal) = self.modals.get(&id) else { return };
        let Some(content) = content_of(ctx, id) else { return };
        let (Some(surface), Some(frame)) = (ctx.scene.node(id).map(|node| node.frame), ctx.scene.node(content).map(|node| node.frame))
        else {
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
        ctx.scene.set_layer_override(content, LayerSlot::Presenter, layer);
        if let Some(handle) = modal.handle {
            ctx.scene.set_layer_override(handle, LayerSlot::Presenter, LayerOverride { transform: layer.transform, ..LayerOverride::NONE });
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
        let Some(node) = ctx.scene.node(id) else { return };
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
        ctx.emit_if(id, PropKey::OnModalRequestClose, event_kind::MODAL_REQUEST_CLOSE, Vec::new());
        if notifies_native {
            let payload = WireMap::new().with("action", 1_i64).with("dismissed", true).encode();
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
