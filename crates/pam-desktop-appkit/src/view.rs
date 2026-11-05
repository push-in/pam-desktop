//! The views behind a scene node.
//!
//! `PamView` is the view of every node: it is placed at the frame the
//! layout engine computed and paints the node's decoration, image, drawing
//! and text. A box that is only a solid, uniformly rounded and bordered
//! rectangle is painted by its layer's own properties and never gets a
//! backing store; anything richer is drawn with Core Graphics in
//! `drawRect:`.
//!
//! `NodeView` adds what one view cannot do: outer shadows live in a sibling
//! view right below the node's (a layer never paints outside its backing
//! store), and `overflow: hidden` moves the node's content into an inner
//! view that clips to the rounded padding box.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, Sel};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly, Message};
use objc2_app_kit::{NSGraphicsContext, NSResponder, NSView, NSViewLayerContentsRedrawPolicy, NSWindowOrderingMode};
use objc2_core_foundation::{CFRetained, CGAffineTransform, CGFloat, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGContext, CGImage};
use objc2_core_image::{CIFilter, CIVector};
use objc2_foundation::{
    NSArray, NSDictionary, NSKeyValueObservingOptions, NSNumber, NSObjectNSKeyValueCoding, NSObjectNSKeyValueObserverRegistration, NSRect,
    NSString,
};
use objc2_quartz_core::{kCAFillRuleEvenOdd, CALayer, CAShapeLayer, CATransaction};
use pam_desktop_ui::backend::{BackendEvent, DrawCommand, ImageContent, NodeId, TextContent, VerticalAlign};
use pam_desktop_ui::geometry::{Affine, Insets, Point, Rect, Size};
use pam_desktop_ui::shell::Cursor;
use pam_desktop_ui::style::{BorderStyle, Decoration, LayerState, Shadow};

use crate::events::Events;
use crate::paint::{self, RoundedRect};
use crate::text::Paragraph;
use crate::util::{self, cg_color, cg_rect, ns_string};

pub struct TextState {
    pub content: TextContent,
    pub paragraph: Paragraph,
}

pub struct ImageState {
    pub content: ImageContent,
    pub image: CFRetained<CGImage>,
}

#[derive(Default)]
pub struct ViewState {
    pub decoration: Decoration,
    pub text: Option<TextState>,
    pub image: Option<ImageState>,
    /// Painted above the image and below the text and the children.
    pub drawing: Vec<DrawCommand>,
    /// Range of the text painted as selected (UTF-16 offsets).
    pub selection: Option<(u32, u32)>,
}

impl ViewState {
    /// Whether the box shows anything of its own.
    fn is_solid(&self) -> bool {
        let decoration = &self.decoration;
        self.text.is_some()
            || self.image.is_some()
            || !self.drawing.is_empty()
            || !decoration.background.is_transparent()
            || !decoration.gradients.is_empty()
            || decoration.border.is_visible()
            || (decoration.border_gradient.is_some() && decoration.border.has_width())
            || decoration.backdrop_blur > 0.0
    }

    /// Whether the box needs Core Graphics; otherwise its layer's own
    /// background, corner radius and border paint it.
    fn needs_drawing(&self) -> bool {
        let decoration = &self.decoration;
        let border = &decoration.border;
        self.text.is_some()
            || self.image.is_some()
            || !self.drawing.is_empty()
            || !decoration.gradients.is_empty()
            || (decoration.border_gradient.is_some() && border.has_width())
            || decoration.shadows.iter().any(|shadow| shadow.inset && !shadow.color.is_transparent())
            || !decoration.radii.is_uniform()
            || (border.is_visible() && (!border.is_uniform() || border.style != BorderStyle::Solid))
    }
}

/// Called with `true` when keyboard focus enters a view's subtree and
/// with `false` when it leaves.
pub type FocusListener = Rc<dyn Fn(bool)>;

pub struct ViewIvars {
    id: NodeId,
    events: Rc<Events>,
    state: RefCell<ViewState>,
    /// The mode chosen by `ViewState::needs_drawing`.
    draws: Cell<bool>,
    /// The box shows something, so it covers what is behind it.
    solid: Cell<bool>,
    /// What the layer adds to the frame: scale, rotation, skew.
    kept: KeptTransform,
    focusable: Cell<bool>,
    cursor: Cell<Cursor>,
    /// Assistive technology may press the view.
    pressable: Cell<bool>,
    /// Told when keyboard focus enters or leaves the view's subtree
    /// (the native control of an input).
    focus_listener: RefCell<Option<FocusListener>>,
    /// Secondary clicks do not reach the native control inside the view.
    context_menu_hidden: Cell<bool>,
}

/// Layer geometry inside a window's content view has its origin at the
/// top-left corner, like the views: AppKit flips the layer tree once, at
/// the first flipped view, and every view of this backend is flipped.
///
/// Everything handed to layers directly (transforms, shadow paths and
/// offsets, masks) goes through the helpers below, so this is the one
/// switch should a system ever show them mirrored vertically.
const LAYERS_Y_DOWN: bool = true;

/// A shape of a view `height` tall, in its layer's coordinates.
fn layer_shape(shape: RoundedRect, height: f32) -> RoundedRect {
    if LAYERS_Y_DOWN {
        return shape;
    }
    let [top_left, top_right, bottom_right, bottom_left] = shape.corners;
    RoundedRect { rect: layer_rect(shape.rect, height), corners: [bottom_left, bottom_right, top_right, top_left] }
}

fn layer_rect(rect: Rect, height: f32) -> Rect {
    if LAYERS_Y_DOWN { rect } else { Rect::new(rect.x, height - rect.bottom(), rect.width, rect.height) }
}

fn layer_offset(offset: Point) -> CGSize {
    CGSize::new(CGFloat::from(offset.x), CGFloat::from(if LAYERS_Y_DOWN { offset.y } else { -offset.y }))
}

/// Sets the layer transform of a view so that `transform` acts around the
/// view's own origin, wherever AppKit put the layer's anchor point.
fn apply_layer_transform(view: &NSView, transform: &Affine) {
    let Some(layer) = view.layer() else { return };
    let anchor = layer.anchorPoint();
    let bounds = view.bounds();
    // The anchor, measured from the view's top-left corner.
    let anchor_y = if LAYERS_Y_DOWN { anchor.y } else { 1.0 - anchor.y };
    let pivot = Point::new(-(anchor.x * bounds.size.width) as f32, -(anchor_y * bounds.size.height) as f32);
    let around = if pivot == Point::ZERO { *transform } else { transform.around(pivot) };
    let native = if LAYERS_Y_DOWN {
        around
    } else {
        // The same transform with the y axis pointing up.
        Affine { a: around.a, b: -around.b, c: -around.c, d: around.d, tx: around.tx, ty: -around.ty }
    };
    layer.setAffineTransform(paint::cg_affine(&native));
}

const IDENTITY: CGAffineTransform = CGAffineTransform { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 };

/// The scale, rotation and skew of a view, kept on its layer.
///
/// AppKit owns the geometry of a view's layer: it sets the transform back
/// to identity whenever the view moves or resizes, and once more when the
/// view is first shown in a window. The translation of a node therefore
/// goes into the view's frame, where AppKit expects it, and only what a
/// frame cannot express is put on the layer, watched, and put back when
/// AppKit clears it.
pub struct KeptTransform {
    /// Without translation.
    linear: Cell<Affine>,
    /// The layer the keeper watches for this view.
    watched: RefCell<Option<Retained<CALayer>>>,
    /// The view itself is writing the layer's geometry.
    paused: Cell<bool>,
}

impl KeptTransform {
    fn new() -> Self {
        Self { linear: Cell::new(Affine::IDENTITY), watched: RefCell::new(None), paused: Cell::new(false) }
    }

    /// Moves `view` to `frame` and gives its layer the `linear` transform.
    fn place(&self, view: &NSView, frame: Rect, linear: Affine) {
        let previous = self.linear.replace(linear);
        self.paused.set(true);
        if !previous.is_identity()
            && let Some(layer) = view.layer()
        {
            // AppKit computes the layer's geometry from the frame: it must
            // not see a transform while it does.
            layer.setAffineTransform(IDENTITY);
        }
        view.setFrame(cg_rect(frame));
        self.paused.set(false);
        self.restore(view);
    }

    /// Puts the transform (back) on the view's layer and watches it for as
    /// long as there is one to keep.
    fn restore(&self, view: &NSView) {
        let linear = self.linear.get();
        let layer = view.layer();
        let watched = self.watched.borrow().clone();
        let wanted = layer.clone().filter(|_| !linear.is_identity());
        let same = match (&watched, &wanted) {
            (Some(watched), Some(wanted)) => Retained::as_ptr(watched) == Retained::as_ptr(wanted),
            (None, None) => true,
            _ => false,
        };
        if !same {
            // `try_with`: views may be released while the thread ends.
            let registered = KEEPER.try_with(|keeper| {
                let keeper = keeper.get_or_init(|| TransformKeeper::new(view.mtm()));
                // SAFETY: the keeper implements `observeValueForKeyPath:…`;
                // each layer is unregistered here before it is forgotten.
                unsafe {
                    if let Some(watched) = &watched {
                        watched.removeObserver_forKeyPath(keeper, &ns_string("transform"));
                    }
                    if let Some(wanted) = &wanted {
                        wanted.addObserver_forKeyPath_options_context(keeper, &ns_string("transform"), NSKeyValueObservingOptions::New, std::ptr::null_mut());
                    }
                }
            });
            if registered.is_ok() {
                *self.watched.borrow_mut() = wanted;
            }
        }
        if !linear.is_identity() {
            self.paused.set(true);
            apply_layer_transform(view, &linear);
            self.paused.set(false);
        }
    }

    /// AppKit wrote the layer's transform: when it cleared it, ours returns.
    fn layer_changed(&self, view: &NSView, layer: &CALayer) {
        if self.paused.get() || self.linear.get().is_identity() {
            return;
        }
        let current = layer.affineTransform();
        if current.a == 1.0 && current.b == 0.0 && current.c == 0.0 && current.d == 1.0 {
            self.paused.set(true);
            apply_layer_transform(view, &self.linear.get());
            self.paused.set(false);
        }
    }

    /// Stops watching (the view is going away).
    fn forget(&self, view: &NSView) {
        self.linear.set(Affine::IDENTITY);
        self.restore(view);
    }
}

thread_local! {
    /// The one observer of every kept transform.
    static KEEPER: std::cell::OnceCell<Retained<TransformKeeper>> = const { std::cell::OnceCell::new() };
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements. Main thread
    // only: it is registered on the layers of views.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct TransformKeeper;

    impl TransformKeeper {
        /// The `transform` of a watched layer was written.
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_value(
            &self,
            _key_path: Option<&NSString>,
            object: Option<&AnyObject>,
            _change: Option<&NSDictionary<NSString, AnyObject>>,
            _context: *mut c_void,
        ) {
            let Some(layer) = object.and_then(|object| object.downcast_ref::<CALayer>()) else { return };
            // The delegate of a view's layer is the view.
            let Some(delegate) = layer.delegate() else { return };
            let owner: &AnyObject = (*delegate).as_ref();
            if let Some(view) = owner.downcast_ref::<PamView>() {
                view.ivars().kept.layer_changed(view, layer);
            } else if let Some(view) = owner.downcast_ref::<PlainView>() {
                view.ivars().layer_changed(view, layer);
            }
        }
    }
);

impl TransformKeeper {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: `init` is the designated initializer of `NSObject`.
        unsafe { msg_send![super(this), init] }
    }
}

/// Views draw past their bounds unless told otherwise (macOS 14 made it
/// the default; before, the property does not exist and the layer decides).
fn set_clips(view: &NSView, clips: bool) {
    if view.respondsToSelector(sel!(setClipsToBounds:)) {
        view.setClipsToBounds(clips);
    }
    if let Some(layer) = view.layer() {
        layer.setMasksToBounds(clips);
    }
}

define_class!(
    // SAFETY: `NSView` allows subclassing; the overrides below keep its
    // contracts (they never retain `self` and call `super` where AppKit
    // requires it). The class is only used on the main thread.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ViewIvars]
    pub struct PamView;

    impl PamView {
        /// The engine's frames have their origin at the top-left corner.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        /// Layer-painted boxes never get a backing store.
        #[unsafe(method(wantsUpdateLayer))]
        fn wants_update_layer(&self) -> bool {
            !self.ivars().draws.get()
        }

        /// Where AppKit lets a view set its layer's own properties.
        #[unsafe(method(updateLayer))]
        fn update_layer(&self) {
            if let Some(layer) = self.layer() {
                // SAFETY: nil is a valid value: it drops pixels drawn while
                // the view still needed Core Graphics.
                unsafe { layer.setContents(None) };
            }
            self.apply_layer_paint();
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let Some(graphics) = NSGraphicsContext::currentContext() else { return };
            self.paint(&graphics.CGContext());
        }

        /// A box that shows nothing is only a container: native controls
        /// behind it (a scroll view under an overlay) stay reachable. The
        /// core does its own hit testing of pointer events.
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, point: CGPoint) -> Option<Retained<NSView>> {
            // SAFETY: forwards the same selector and argument to `NSView`.
            let hit: Option<Retained<NSView>> = unsafe { msg_send![super(self), hitTest: point] };
            let ivars = self.ivars();
            let own = hit.as_deref().is_some_and(|view| std::ptr::eq(view, &**self));
            hit.filter(|_| !own || ivars.solid.get() || ivars.focusable.get())
        }

        /// A press focuses native controls only; boxes are reached with Tab.
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            let ivars = self.ivars();
            ivars.focusable.get() && !ivars.events.dispatching_mouse_down.get()
        }

        /// The window is moved through `begin_window_drag` only.
        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool {
            false
        }

        /// The focus ring follows the rounded outline of the box.
        #[unsafe(method(drawFocusRingMask))]
        fn draw_focus_ring_mask(&self) {
            let Some(graphics) = NSGraphicsContext::currentContext() else { return };
            let context = graphics.CGContext();
            let size = util::size(self.bounds().size);
            let radii = self.ivars().state.borrow().decoration.radii;
            CGContext::begin_path(Some(&context));
            RoundedRect::new(size, radii, Insets::ZERO).add_to_context(&context);
            CGContext::fill_path(Some(&context));
        }

        #[unsafe(method(focusRingMaskBounds))]
        fn focus_ring_mask_bounds(&self) -> NSRect {
            self.bounds()
        }

        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            if let Some(cursor) = crate::shell::native_cursor(self.ivars().cursor.get()) {
                self.addCursorRect_cursor(self.bounds(), &cursor);
            }
        }

        /// AppKit may rebuild the layer's geometry when the view moves in
        /// the hierarchy: what was set on the layer is set again.
        #[unsafe(method(viewDidMoveToWindow))]
        fn view_did_move_to_window(&self) {
            self.restore_layer();
        }

        #[unsafe(method(viewDidMoveToSuperview))]
        fn view_did_move_to_superview(&self) {
            self.restore_layer();
        }

        /// The window moved to a display with another pixel density.
        #[unsafe(method(viewDidChangeBackingProperties))]
        fn view_did_change_backing_properties(&self) {
            // SAFETY: forwards the same selector to `NSView`.
            let _: () = unsafe { msg_send![super(self), viewDidChangeBackingProperties] };
            self.repaint();
        }

        /// Light or dark changed: text without a color follows the system.
        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn view_did_change_effective_appearance(&self) {
            // SAFETY: forwards the same selector to `NSView`.
            let _: () = unsafe { msg_send![super(self), viewDidChangeEffectiveAppearance] };
            self.repaint();
        }

        /// The default action of assistive technology.
        #[unsafe(method(accessibilityPerformPress))]
        fn accessibility_perform_press(&self) -> bool {
            let ivars = self.ivars();
            let pressable = ivars.pressable.get();
            if pressable {
                ivars.events.push(BackendEvent::AccessibilityAction { id: ivars.id, name: String::from("activate") });
            }
            pressable
        }

        /// Only pressable roles offer the press action.
        #[unsafe(method(isAccessibilitySelectorAllowed:))]
        fn is_accessibility_selector_allowed(&self, selector: Sel) -> bool {
            if selector == sel!(accessibilityPerformPress) {
                self.ivars().pressable.get()
            } else {
                // SAFETY: forwards the same selector with the same argument to `NSView`.
                unsafe { msg_send![super(self), isAccessibilitySelectorAllowed: selector] }
            }
        }
    }
);

impl PamView {
    pub fn new(mtm: MainThreadMarker, id: NodeId, events: &Rc<Events>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ViewIvars {
            id,
            events: Rc::clone(events),
            state: RefCell::new(ViewState::default()),
            draws: Cell::new(false),
            solid: Cell::new(false),
            kept: KeptTransform::new(),
            focusable: Cell::new(false),
            cursor: Cell::new(Cursor::Default),
            pressable: Cell::new(false),
            focus_listener: RefCell::new(None),
            context_menu_hidden: Cell::new(false),
        });
        // SAFETY: `initWithFrame:` is the designated initializer of `NSView`.
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        view.setWantsLayer(true);
        // Pixels are redrawn when the view says so, not on every move.
        view.setLayerContentsRedrawPolicy(NSViewLayerContentsRedrawPolicy::OnSetNeedsDisplay);
        set_clips(&view, false);
        view
    }

    /// A strong reference to the view.
    pub fn retain_view(&self) -> Retained<Self> {
        self.retain()
    }

    /// The node this view belongs to.
    pub fn node_id(&self) -> NodeId {
        self.ivars().id
    }

    pub fn with_state<T>(&self, call: impl FnOnce(&ViewState) -> T) -> T {
        call(&self.ivars().state.borrow())
    }

    /// Changes what the view paints and schedules the repaint.
    fn update(&self, change: impl FnOnce(&mut ViewState)) {
        change(&mut self.ivars().state.borrow_mut());
        self.refresh();
    }

    /// Redraws a box that is painted with Core Graphics.
    fn repaint(&self) {
        if self.ivars().draws.get() {
            self.setNeedsDisplay(true);
        }
    }

    /// Picks the paint mode for the current state and applies it.
    fn refresh(&self) {
        let ivars = self.ivars();
        let (draws, solid) = {
            let state = ivars.state.borrow();
            (state.needs_drawing(), state.is_solid())
        };
        ivars.solid.set(solid);
        let changed = ivars.draws.replace(draws) != draws;
        self.apply_layer_paint();
        if draws || changed {
            self.setNeedsDisplay(true);
        }
    }

    /// Background, corner radius and border of a layer-painted box.
    fn apply_layer_paint(&self) {
        let Some(layer) = self.layer() else { return };
        let ivars = self.ivars();
        let state = ivars.state.borrow();
        let decoration = &state.decoration;
        if ivars.draws.get() {
            layer.setBackgroundColor(None);
            layer.setBorderWidth(0.0);
            layer.setCornerRadius(0.0);
            return;
        }
        let size = util::size(self.bounds().size);
        if decoration.background.is_transparent() {
            layer.setBackgroundColor(None);
        } else {
            layer.setBackgroundColor(Some(&cg_color(decoration.background)));
        }
        layer.setCornerRadius(CGFloat::from(decoration.radii.clamped(size).top_left));
        if decoration.border.is_visible() {
            layer.setBorderWidth(CGFloat::from(decoration.border.widths[0]));
            layer.setBorderColor(Some(&cg_color(decoration.border.colors[0])));
        } else {
            layer.setBorderWidth(0.0);
        }
    }

    fn restore_layer(&self) {
        self.apply_layer_paint();
        self.ivars().kept.restore(self);
    }

    pub fn set_decoration(&self, decoration: &Decoration) {
        self.update(|state| state.decoration = decoration.clone());
        // The focus ring follows the outline.
        if self.ivars().focusable.get() {
            self.noteFocusRingMaskChanged();
        }
    }

    pub fn set_text(&self, content: Option<&TextContent>) {
        let text = content.map(|content| TextState { content: content.clone(), paragraph: Paragraph::new(&content.block) });
        self.update(|state| {
            state.text = text;
            state.selection = None;
        });
    }

    pub fn set_image(&self, image: Option<ImageState>) {
        self.update(|state| state.image = image);
    }

    pub fn set_drawing(&self, commands: &[DrawCommand]) {
        self.update(|state| {
            state.drawing.clear();
            state.drawing.extend_from_slice(commands);
        });
    }

    pub fn set_text_selection(&self, range: Option<(u32, u32)>) {
        let changed = {
            let mut state = self.ivars().state.borrow_mut();
            let changed = state.selection != range;
            state.selection = range;
            changed
        };
        if changed {
            self.setNeedsDisplay(true);
        }
    }

    /// Called after the frame changed size.
    pub fn size_changed(&self) {
        // Corner radii are clamped to the box, and the pixels are stale.
        self.refresh();
        if self.ivars().cursor.get() != Cursor::Default {
            self.invalidate_cursor();
        }
    }

    /// Moves the view to `frame` and gives its layer the `linear` transform.
    pub fn place(&self, frame: Rect, linear: Affine) {
        self.ivars().kept.place(self, frame, linear);
    }

    /// Releases what the view registered outside itself.
    pub fn retire(&self) {
        self.ivars().kept.forget(self);
    }

    pub fn set_focusable(&self, focusable: bool) {
        self.ivars().focusable.set(focusable);
    }

    pub fn set_cursor(&self, cursor: Cursor) {
        if self.ivars().cursor.replace(cursor) != cursor {
            self.invalidate_cursor();
        }
    }

    fn invalidate_cursor(&self) {
        if let Some(window) = self.window() {
            window.invalidateCursorRectsForView(self);
        }
    }

    pub fn set_pressable(&self, pressable: bool) {
        self.ivars().pressable.set(pressable);
    }

    pub fn set_focus_listener(&self, listener: Option<FocusListener>) {
        *self.ivars().focus_listener.borrow_mut() = listener;
    }

    /// Tells the native control inside the view about keyboard focus.
    pub fn notify_focus(&self, focused: bool) {
        let listener = self.ivars().focus_listener.borrow().clone();
        if let Some(listener) = listener {
            listener(focused);
        }
    }

    pub fn set_context_menu_hidden(&self, hidden: bool) {
        self.ivars().context_menu_hidden.set(hidden);
    }

    pub fn context_menu_hidden(&self) -> bool {
        self.ivars().context_menu_hidden.get()
    }

    /// Where the text is painted: its top-left corner and wrapping width.
    fn text_placement(&self, text: &TextState, size: Size) -> (Point, f32) {
        let insets = text.content.insets;
        let available = (size.width - insets.horizontal()).max(0.0);
        let free = size.height - insets.vertical() - text.paragraph.measure(available).height;
        let top = insets.top
            + match text.content.vertical {
                VerticalAlign::Top => 0.0,
                VerticalAlign::Center => (free / 2.0).max(0.0),
                VerticalAlign::Bottom => free.max(0.0),
            };
        // Whole device pixels keep the glyphs crisp.
        let scale = self.window().map_or(1.0, |window| window.backingScaleFactor() as f32).max(1.0);
        (Point::new(insets.left, (top * scale).round() / scale), available)
    }

    /// The UTF-16 offset of the character boundary nearest to `point`.
    pub fn text_index_at(&self, point: Point) -> Option<u32> {
        let state = self.ivars().state.borrow();
        let text = state.text.as_ref()?;
        let (origin, width) = self.text_placement(text, util::size(self.bounds().size));
        Some(text.paragraph.index_at(width, CGPoint::new(CGFloat::from(point.x - origin.x), CGFloat::from(point.y - origin.y))))
    }

    fn paint(&self, context: &CGContext) {
        let state = self.ivars().state.borrow();
        let size = util::size(self.bounds().size);
        if size.is_empty() {
            return;
        }
        let decoration = &state.decoration;
        paint::paint_background(context, size, decoration);
        paint::paint_border(context, size, decoration);
        if decoration.clip {
            CGContext::save_g_state(Some(context));
            paint::clip_to_padding_box(context, size, decoration);
        }
        if let Some(image) = &state.image {
            paint::paint_image(context, size, decoration, &image.content, &image.image);
        }
        if !state.drawing.is_empty() {
            let scale = self.window().map_or(1.0, |window| window.backingScaleFactor() as f32).max(1.0);
            CGContext::save_g_state(Some(context));
            CGContext::clip_to_rect(Some(context), cg_rect(Rect::from_size(size)));
            paint::paint_drawing(context, &state.drawing, 1.0 / scale);
            CGContext::restore_g_state(Some(context));
        }
        if let Some(text) = &state.text {
            let (origin, width) = self.text_placement(text, size);
            text.paragraph.draw(context, util::cg_point(origin), width, state.selection);
        }
        if decoration.clip {
            CGContext::restore_g_state(Some(context));
        }
    }
}

define_class!(
    // SAFETY: `NSView` allows subclassing and the overrides keep its
    // contracts. The class is only used on the main thread.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = KeptTransform]
    /// A flipped view that paints nothing: the clip of a node's content,
    /// the host of its shadow layers, the placement of a native control.
    pub struct PlainView;

    impl PlainView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool {
            false
        }

        /// Never a target itself: only what it contains.
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, point: CGPoint) -> Option<Retained<NSView>> {
            // SAFETY: forwards the same selector and argument to `NSView`.
            let hit: Option<Retained<NSView>> = unsafe { msg_send![super(self), hitTest: point] };
            hit.filter(|view| !std::ptr::eq(&**view, &**self))
        }

        #[unsafe(method(viewDidMoveToWindow))]
        fn view_did_move_to_window(&self) {
            self.ivars().restore(self);
        }
    }
);

impl PlainView {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(KeptTransform::new());
        // SAFETY: `initWithFrame:` is the designated initializer of `NSView`.
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        view.setWantsLayer(true);
        set_clips(&view, false);
        view
    }

    fn place(&self, frame: Rect, linear: Affine) {
        self.ivars().place(self, frame, linear);
    }

    fn retire(&self) {
        self.ivars().forget(self);
    }
}

/// Runs layer changes without the implicit animations of standalone layers.
fn without_animation(change: impl FnOnce()) {
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    change();
    CATransaction::commit();
}

/// The outer shadows of a node, composited by Core Animation.
struct ShadowHost {
    view: Retained<PlainView>,
    layers: Vec<Retained<CALayer>>,
}

/// Blur, backdrop blur and color matrix last applied to a node's view.
#[derive(Clone, Copy, Default, PartialEq)]
struct Filters {
    blur: f32,
    backdrop_blur: f32,
    color_matrix: Option<[f32; 20]>,
}

fn filter(name: &str, values: &[(&str, &AnyObject)]) -> Option<Retained<CIFilter>> {
    // SAFETY: the names and keys are those Core Image documents for its
    // built-in filters, and every value has the class the key expects.
    unsafe {
        let filter = CIFilter::filterWithName(&ns_string(name))?;
        filter.setDefaults();
        for (key, value) in values {
            filter.setValue_forKey(Some(value), &ns_string(key));
        }
        Some(filter)
    }
}

fn gaussian_blur(radius: f32) -> Option<Retained<CIFilter>> {
    let radius = NSNumber::new_f32(radius);
    filter("CIGaussianBlur", &[("inputRadius", radius.as_ref())])
}

/// Rows of R, G, B and A with an offset column, as a Core Image filter.
fn color_matrix(matrix: &[f32; 20]) -> Option<Retained<CIFilter>> {
    let row = |index: usize| {
        let at = |column: usize| CGFloat::from(matrix[index * 5 + column]);
        // SAFETY: plain numbers; the class method only builds a vector.
        unsafe { CIVector::vectorWithX_Y_Z_W(at(0), at(1), at(2), at(3)) }
    };
    let offset = |index: usize| CGFloat::from(matrix[index * 5 + 4]);
    // SAFETY: as above.
    let bias = unsafe { CIVector::vectorWithX_Y_Z_W(offset(0), offset(1), offset(2), offset(3)) };
    let (red, green, blue, alpha) = (row(0), row(1), row(2), row(3));
    filter(
        "CIColorMatrix",
        &[
            ("inputRVector", red.as_ref()),
            ("inputGVector", green.as_ref()),
            ("inputBVector", blue.as_ref()),
            ("inputAVector", alpha.as_ref()),
            ("inputBiasVector", bias.as_ref()),
        ],
    )
}

/// Everything native a node owns besides its specialised hosts.
pub struct NodeView {
    /// The view placed in the parent.
    pub view: Retained<PamView>,
    mtm: MainThreadMarker,
    shadow: Option<ShadowHost>,
    /// Inner view clipping the content to the rounded padding box.
    clip: Option<Retained<PlainView>>,
    /// `Decoration::clip` of the current decoration.
    clipping: bool,
    frame: Rect,
    layer: LayerState,
    filters: Filters,
}

impl NodeView {
    pub fn new(mtm: MainThreadMarker, id: NodeId, events: &Rc<Events>) -> Self {
        Self {
            view: PamView::new(mtm, id, events),
            mtm,
            shadow: None,
            clip: None,
            clipping: false,
            frame: Rect::ZERO,
            layer: LayerState::default(),
            filters: Filters::default(),
        }
    }

    pub fn frame(&self) -> Rect {
        self.frame
    }

    /// The views this node contributes to its parent, back to front.
    pub fn stack(&self, into: &mut Vec<Retained<NSView>>) {
        if let Some(shadow) = &self.shadow {
            into.push(Retained::into_super(shadow.view.clone()));
        }
        into.push(Retained::into_super(self.view.clone()));
    }

    /// Where native controls and child nodes are, without creating the clip.
    pub fn content_view(&self) -> Retained<NSView> {
        match &self.clip {
            Some(clip) => Retained::into_super(clip.clone()),
            None => Retained::into_super(self.view.clone()),
        }
    }

    /// Where native controls and child nodes go. A clipping node gets its
    /// inner view when the first content arrives.
    pub fn content_parent(&mut self) -> Retained<NSView> {
        if self.clipping && self.clip.is_none() {
            let clip = PlainView::new(self.mtm);
            // Whatever the view already holds moves inside, in order.
            clip.setSubviews(&self.view.subviews());
            self.view.addSubview(&clip);
            self.clip = Some(clip);
            self.layout_clip();
        }
        self.content_view()
    }

    fn dissolve_clip(&mut self) {
        if let Some(clip) = self.clip.take() {
            let content = clip.subviews();
            clip.removeFromSuperview();
            self.view.setSubviews(&content);
        }
    }

    /// Places the clip on the padding box. Its bounds keep the origin of
    /// the border box, so content frames mean the same with and without it.
    fn layout_clip(&self) {
        let Some(clip) = &self.clip else { return };
        let (decoration_radii, [left, top, right, bottom]) = self.view.with_state(|state| (state.decoration.radii, state.decoration.border.widths));
        let inner = RoundedRect::new(self.frame.size(), decoration_radii, Insets::new(left, top, right, bottom));
        clip.setFrame(cg_rect(inner.rect));
        clip.setBoundsOrigin(CGPoint::new(CGFloat::from(left), CGFloat::from(top)));
        set_clips(clip, true);
        let Some(layer) = clip.layer() else { return };
        without_animation(|| {
            let radius = if inner.is_rect() { Some(0.0) } else { inner.uniform_radius() };
            match radius {
                Some(radius) => {
                    layer.setCornerRadius(CGFloat::from(radius));
                    // SAFETY: nil removes the mask.
                    unsafe { layer.setMask(None) };
                }
                None => {
                    // Corners that differ need a shape.
                    let mask = CAShapeLayer::new();
                    mask.setFrame(cg_rect(inner.rect));
                    mask.setPath(Some(&layer_shape(inner.translate(-left, -top), inner.rect.height).path()));
                    layer.setCornerRadius(0.0);
                    // SAFETY: the mask is a fresh layer without a superlayer.
                    unsafe { layer.setMask(Some(&mask)) };
                }
            }
        });
    }

    fn outer_shadows(decoration: &Decoration) -> impl Iterator<Item = &Shadow> {
        decoration.shadows.iter().filter(|shadow| !shadow.inset && !shadow.color.is_transparent())
    }

    /// Builds the shadow layers for the current decoration and size.
    fn layout_shadows(&mut self) {
        let size = self.frame.size();
        let view = &self.view;
        let (count, opaque) = view.with_state(|state| (Self::outer_shadows(&state.decoration).count(), state.decoration.background.is_opaque()));
        if count == 0 {
            if let Some(shadow) = self.shadow.take() {
                shadow.view.retire();
                shadow.view.removeFromSuperview();
            }
            return;
        }
        if self.shadow.is_none() {
            let host = PlainView::new(self.mtm);
            let (frame, linear) = self.placement();
            host.place(frame, linear);
            host.setAlphaValue(CGFloat::from(self.layer.opacity));
            host.setHidden(!self.layer.visible);
            // SAFETY: the superview is only used to insert the sibling.
            if let Some(parent) = unsafe { view.superview() } {
                parent.addSubview_positioned_relativeTo(&host, NSWindowOrderingMode::Below, Some(view));
            }
            self.shadow = Some(ShadowHost { view: host, layers: Vec::new() });
        }
        let Some(host) = self.shadow.as_mut() else { return };
        let Some(root) = host.view.layer() else { return };
        without_animation(|| {
            while host.layers.len() > count {
                if let Some(layer) = host.layers.pop() {
                    layer.removeFromSuperlayer();
                }
            }
            while host.layers.len() < count {
                let layer = CALayer::new();
                root.addSublayer(&layer);
                host.layers.push(layer);
            }
            let bounds = Rect::from_size(size);
            view.with_state(|state| {
                let outline = RoundedRect::new(size, state.decoration.radii, Insets::ZERO);
                // The first shadow of the list is the topmost one.
                for (layer, shadow) in host.layers.iter().rev().zip(Self::outer_shadows(&state.decoration)) {
                    layer.setFrame(cg_rect(bounds));
                    layer.setShadowPath(Some(&layer_shape(outline.outset(shadow.spread), size.height).path()));
                    layer.setShadowColor(Some(&cg_color(shadow.color)));
                    layer.setShadowOpacity(1.0);
                    layer.setShadowOffset(layer_offset(shadow.offset));
                    // A CSS blur radius is two standard deviations.
                    layer.setShadowRadius(CGFloat::from(shadow.blur.max(0.0) / 2.0));
                    if opaque {
                        // SAFETY: nil removes the mask.
                        unsafe { layer.setMask(None) };
                    } else {
                        // A shadow never shows through a translucent box.
                        let mask = CAShapeLayer::new();
                        mask.setFrame(cg_rect(bounds));
                        let around = layer_rect(shadow.bounds(bounds).outset(8.0), size.height);
                        mask.setPath(Some(&layer_shape(outline, size.height).cutout_path(around)));
                        // SAFETY: the fill rule is an immutable string exported by Core Animation.
                        mask.setFillRule(unsafe { kCAFillRuleEvenOdd });
                        // SAFETY: the mask is a fresh layer without a superlayer.
                        unsafe { layer.setMask(Some(&mask)) };
                    }
                }
            });
        });
    }

    fn apply_filters(&mut self, decoration: &Decoration) {
        let filters = Filters { blur: decoration.blur, backdrop_blur: decoration.backdrop_blur, color_matrix: decoration.color_matrix };
        if filters == self.filters {
            return;
        }
        self.filters = filters;
        let content: Vec<Retained<CIFilter>> = [
            filters.color_matrix.as_ref().and_then(color_matrix),
            (filters.blur > 0.0).then(|| gaussian_blur(filters.blur)).flatten(),
        ]
        .into_iter()
        .flatten()
        .collect();
        let backdrop: Vec<Retained<CIFilter>> = (filters.backdrop_blur > 0.0).then(|| gaussian_blur(filters.backdrop_blur)).flatten().into_iter().collect();
        self.view.setLayerUsesCoreImageFilters(!content.is_empty() || !backdrop.is_empty());
        self.view.setContentFilters(&NSArray::from_retained_slice(&content));
        self.view.setBackgroundFilters(&NSArray::from_retained_slice(&backdrop));
    }

    pub fn set_decoration(&mut self, decoration: &Decoration) {
        self.view.set_decoration(decoration);
        self.clipping = decoration.clip;
        if !self.clipping {
            self.dissolve_clip();
        } else if self.clip.is_none() && !self.view.subviews().is_empty() {
            self.content_parent();
        } else {
            self.layout_clip();
        }
        self.layout_shadows();
        self.apply_filters(decoration);
    }

    /// Where the view goes and what its layer adds: a node's translation
    /// moves the frame, the rest of its transform is the layer's.
    fn placement(&self) -> (Rect, Affine) {
        let transform = self.layer.transform;
        (self.frame.translate(transform.tx, transform.ty), Affine { tx: 0.0, ty: 0.0, ..transform })
    }

    fn place(&self) {
        let (frame, linear) = self.placement();
        self.view.place(frame, linear);
        if let Some(shadow) = &self.shadow {
            shadow.view.place(frame, linear);
        }
    }

    pub fn set_frame(&mut self, frame: Rect) {
        let resized = frame.size() != self.frame.size();
        self.frame = frame;
        self.place();
        if resized {
            self.view.size_changed();
            self.layout_clip();
            self.layout_shadows();
        }
    }

    pub fn set_layer(&mut self, layer: &LayerState) {
        let previous = std::mem::replace(&mut self.layer, *layer);
        if previous.opacity != layer.opacity {
            self.view.setAlphaValue(CGFloat::from(layer.opacity));
        }
        if previous.visible != layer.visible {
            self.view.setHidden(!layer.visible);
        }
        if let Some(shadow) = &self.shadow {
            shadow.view.setAlphaValue(CGFloat::from(layer.opacity));
            shadow.view.setHidden(!layer.visible);
        }
        if previous.transform != layer.transform {
            self.place();
        }
    }

    /// Takes the node out of its parent.
    pub fn detach(&self) {
        if let Some(shadow) = &self.shadow {
            shadow.view.retire();
            shadow.view.removeFromSuperview();
        }
        if let Some(clip) = &self.clip {
            clip.retire();
        }
        self.view.retire();
        self.view.removeFromSuperview();
    }
}

impl Drop for NodeView {
    /// A layer must not be released while its transform is still watched
    /// (a node that was never destroyed, when the application ends).
    fn drop(&mut self) {
        if let Some(shadow) = &self.shadow {
            shadow.view.retire();
        }
        if let Some(clip) = &self.clip {
            clip.retire();
        }
        self.view.retire();
    }
}

/// The node view that contains `view` (or is it), walking up the hierarchy.
pub fn node_view_of(view: &NSView) -> Option<Retained<PamView>> {
    let mut current = Some(view.retain());
    while let Some(candidate) = current {
        match candidate.downcast::<PamView>() {
            Ok(node) => return Some(node),
            // SAFETY: the superview is only read to keep walking.
            Err(other) => current = unsafe { other.superview() },
        }
    }
    None
}

/// Replaces the subviews of `parent` when they differ from `wanted`.
pub fn arrange(parent: &NSView, wanted: &[Retained<NSView>]) {
    let current = parent.subviews();
    let same = current.len() == wanted.len() && current.iter().zip(wanted).all(|(left, right)| Retained::as_ptr(&left) == Retained::as_ptr(right));
    if !same {
        parent.setSubviews(&NSArray::from_retained_slice(wanted));
    }
}

/// The responder's view, when it is one.
pub fn responder_view(responder: &NSResponder) -> Option<Retained<NSView>> {
    responder.retain().downcast::<NSView>().ok()
}
