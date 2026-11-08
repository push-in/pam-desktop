//! Scrolling viewports: an `NSScrollView` whose document view holds the
//! node's children at their content coordinates.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use objc2::rc::{Retained, Weak};
use objc2::runtime::NSObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAnimationContext, NSBorderType, NSEvent, NSResponder, NSScrollElasticity, NSScrollView,
    NSScrollerStyle, NSView, NSViewBoundsDidChangeNotification,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSNotification, NSNotificationCenter};
use pam_desktop_ui::backend::{BackendEvent, NodeId, ScrollConfig};
use pam_desktop_ui::color::Color;
use pam_desktop_ui::geometry::{Point, Size};

use crate::events::Events;
use crate::util::{cg_color, cg_point, cg_size};
use crate::view::PlainView;

pub struct ScrollIvars {
    id: NodeId,
    events: Rc<Events>,
    /// User scrolling is allowed.
    enabled: Cell<bool>,
    /// The offset last reported, to report each one once.
    reported: Cell<Point>,
    /// The host itself is moving or resizing the viewport.
    applying: Cell<bool>,
    /// An offset asked for before the content was laid out that far.
    pending: Cell<Option<Point>>,
    /// The classic bar drawn in a reserved gutter (`scrollbar-gutter:
    /// stable` with `scrollbar-width`/`scrollbar-color`), like GTK.
    bar: RefCell<Option<Retained<PamScrollBar>>>,
}

pub struct BarIvars {
    scroll: Weak<PamScrollView>,
    thickness: Cell<f64>,
    /// Space between the thumb and the gutter's edges across the bar.
    inset: Cell<f64>,
    /// Pointer position along the track and the offset when the thumb was grabbed.
    grab: Cell<Option<(f64, f64)>>,
}

define_class!(
    // SAFETY: `NSView` allows subclassing; the overrides keep its
    // contracts. Main thread only.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = BarIvars]
    /// The thumb of a classic vertical scrollbar, painted by its layer.
    pub struct PamScrollBar;

    impl PamScrollBar {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool {
            false
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if let Some(scroll) = self.ivars().scroll.load() {
                let along = self.along(event);
                self.ivars().grab.set(Some((along, scroll.offset().y as f64)));
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            let (Some((start, offset)), Some(scroll)) = (self.ivars().grab.get(), self.ivars().scroll.load()) else {
                return;
            };
            let (viewport, content) = scroll.extent();
            let track = viewport - self.frame().size.height;
            if track > 0.0 {
                let moved = (self.along(event) - start) * (content - viewport) / track;
                scroll.user_scroll_to(offset + moved);
            }
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &NSEvent) {
            self.ivars().grab.set(None);
        }
    }
);

impl PamScrollBar {
    fn new(mtm: MainThreadMarker, scroll: &PamScrollView) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(BarIvars {
            scroll: Weak::from(scroll),
            thickness: Cell::new(10.0),
            inset: Cell::new(0.0),
            grab: Cell::new(None),
        });
        // SAFETY: `initWithFrame:` is the designated initializer of `NSView`.
        let bar: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        bar.setWantsLayer(true);
        bar
    }

    /// The pointer along the track, in the scroll view's coordinates.
    fn along(&self, event: &NSEvent) -> f64 {
        // SAFETY: the superview is only used to convert the location.
        match unsafe { self.superview() } {
            Some(parent) => {
                let point = parent.convertPoint_fromView(event.locationInWindow(), None);
                if parent.isFlipped() {
                    point.y
                } else {
                    -point.y
                }
            }
            None => event.locationInWindow().y,
        }
    }

    fn style(&self, thickness: f64, inset: f64, color: Color) {
        let inset = inset.clamp(0.0, thickness / 4.0);
        self.ivars().thickness.set(thickness);
        self.ivars().inset.set(inset);
        if let Some(layer) = self.layer() {
            layer.setBackgroundColor(Some(&cg_color(color)));
            layer.setCornerRadius((thickness - 2.0 * inset) / 2.0);
        }
    }
}

define_class!(
    // SAFETY: `NSScrollView` allows subclassing; the override forwards to
    // `super` or to the next responder. Main thread only.
    #[unsafe(super(NSScrollView, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ScrollIvars]
    pub struct PamScrollView;

    impl PamScrollView {
        /// A scroll view that is not user-scrollable lets the gesture
        /// through to whatever is behind it.
        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            if self.ivars().enabled.get() {
                // SAFETY: forwards the same event to `NSScrollView`.
                let _: () = unsafe { msg_send![super(self), scrollWheel: event] };
            // SAFETY: the next responder is only messaged, never kept.
            } else if let Some(next) = unsafe { self.nextResponder() } {
                next.scrollWheel(event);
            }
        }

        /// Overlay scrollers always: AppKit switches scroll views to the
        /// system's preferred style (legacy with a mouse), whose scroller
        /// takes room the engine gave to the content.
        #[unsafe(method(scrollerStyle))]
        fn scroller_style(&self) -> NSScrollerStyle {
            NSScrollerStyle::Overlay
        }

        #[unsafe(method(setScrollerStyle:))]
        fn set_scroller_style(&self, _style: NSScrollerStyle) {
            // SAFETY: forwards to `NSScrollView` with the overlay style.
            let _: () = unsafe { msg_send![super(self), setScrollerStyle: NSScrollerStyle::Overlay] };
        }

        /// The clip view moved over the document (user or programmatic).
        #[unsafe(method(clipBoundsChanged:))]
        fn clip_bounds_changed(&self, _notification: &NSNotification) {
            let ivars = self.ivars();
            if !ivars.applying.get() {
                // The user scrolled: an offset still waiting for its
                // content is no longer wanted.
                ivars.pending.set(None);
            }
            self.report_offset();
            self.update_bar();
        }
    }
);

impl PamScrollView {
    /// The offset inside the scrollable range: the rubber band of an
    /// overscroll is a visual effect, not a position.
    fn offset(&self) -> Point {
        let clip = self.contentView();
        let bounds = clip.bounds();
        let document = self
            .documentView()
            .map_or(CGSize::ZERO, |document| document.frame().size);
        let limit = |value: f64, content: f64, viewport: f64| {
            value.clamp(0.0, (content - viewport).max(0.0)) as f32
        };
        Point::new(
            limit(bounds.origin.x, document.width, bounds.size.width),
            limit(bounds.origin.y, document.height, bounds.size.height),
        )
    }

    /// Viewport and content lengths along the vertical axis.
    fn extent(&self) -> (f64, f64) {
        let viewport = self.contentView().bounds().size.height;
        let content = self
            .documentView()
            .map_or(0.0, |document| document.frame().size.height);
        (viewport, content)
    }

    /// Moves the viewport for the user (the classic bar's thumb).
    fn user_scroll_to(&self, y: f64) {
        let clip = self.contentView();
        let mut wanted = clip.bounds();
        wanted.origin.y = y;
        let target = clip.constrainBoundsRect(wanted).origin;
        clip.scrollToPoint(target);
        self.reflectScrolledClipView(&clip);
    }

    /// Places the classic bar's thumb in the gutter at the right edge, sized
    /// like GTK's slider (proportional, at least 40 points long, `inset`
    /// inside the gutter across the bar); hidden while everything fits, the
    /// gutter staying reserved.
    fn update_bar(&self) {
        let Some(bar) = self.ivars().bar.borrow().clone() else {
            return;
        };
        let (viewport, content) = self.extent();
        let (thickness, inset) = (bar.ivars().thickness.get(), bar.ivars().inset.get());
        if content <= viewport + 0.5 || viewport <= 0.0 || thickness <= 0.0 {
            bar.setHidden(true);
            return;
        }
        let length = (viewport * viewport / content).max(40.0).min(viewport);
        let offset = f64::from(self.offset().y);
        let along = (viewport - length) * offset / (content - viewport);
        let width = self.frame().size.width;
        let y = if self.isFlipped() {
            along
        } else {
            viewport - along - length
        };
        bar.setFrame(CGRect::new(
            CGPoint::new(width - thickness + inset, y),
            CGSize::new(thickness - 2.0 * inset, length),
        ));
        bar.setHidden(false);
    }

    fn report_offset(&self) {
        let ivars = self.ivars();
        let offset = self.offset();
        if ivars.reported.replace(offset) != offset {
            ivars.events.push(BackendEvent::ScrollChanged {
                id: ivars.id,
                offset,
            });
        }
    }
}

/// The scroll view of a `ViewClass::Scroll` node and its document.
pub struct ScrollHost {
    scroll: Retained<PamScrollView>,
    /// Where the node's children live, in content coordinates.
    pub document: Retained<PlainView>,
    horizontal: bool,
    viewport: Size,
    content: Size,
}

impl ScrollHost {
    pub fn new(mtm: MainThreadMarker, id: NodeId, events: &Rc<Events>, parent: &NSView) -> Self {
        let this = PamScrollView::alloc(mtm).set_ivars(ScrollIvars {
            id,
            events: Rc::clone(events),
            enabled: Cell::new(true),
            reported: Cell::new(Point::ZERO),
            applying: Cell::new(false),
            pending: Cell::new(None),
            bar: RefCell::new(None),
        });
        // SAFETY: `initWithFrame:` is the designated initializer of `NSScrollView`.
        let scroll: Retained<PamScrollView> =
            unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        // The node's own view paints the background and the border.
        scroll.setDrawsBackground(false);
        scroll.setBorderType(NSBorderType::NoBorder);
        scroll.setAutohidesScrollers(true);
        // Overlay scrollers take no room: the engine laid the content out
        // for the whole viewport.
        scroll.setScrollerStyle(NSScrollerStyle::Overlay);
        // The engine already accounts for the title bar.
        scroll.setAutomaticallyAdjustsContentInsets(false);
        let document = PlainView::new(mtm);
        scroll.setDocumentView(Some(&document));
        let clip = scroll.contentView();
        clip.setDrawsBackground(false);
        clip.setPostsBoundsChangedNotifications(true);
        // SAFETY: `clipBoundsChanged:` is implemented above and takes the
        // notification; the observer is removed in `destroy`.
        unsafe {
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                &scroll,
                sel!(clipBoundsChanged:),
                Some(NSViewBoundsDidChangeNotification),
                Some(&clip),
            );
        }
        parent.addSubview(&scroll);
        let mut host = Self {
            scroll,
            document,
            horizontal: false,
            viewport: Size::ZERO,
            content: Size::ZERO,
        };
        host.configure(&ScrollConfig::default());
        host
    }

    /// The view placed inside the node's view.
    pub fn view(&self) -> Retained<NSView> {
        Retained::into_super(Retained::into_super(self.scroll.clone()))
    }

    /// Runs a change of the host's own: the bounds changes it causes are
    /// not the user's.
    fn programmatic<T>(&self, change: impl FnOnce() -> T) -> T {
        let applying = &self.scroll.ivars().applying;
        let nested = applying.replace(true);
        let result = change();
        applying.set(nested);
        result
    }

    /// The document is as long as the content along the scrolling axis and
    /// exactly as wide as the viewport across it, so it only scrolls one way.
    fn layout_document(&self) {
        let size = if self.horizontal {
            Size::new(
                self.content.width.max(self.viewport.width),
                self.viewport.height,
            )
        } else {
            Size::new(
                self.viewport.width,
                self.content.height.max(self.viewport.height),
            )
        };
        self.programmatic(|| self.document.setFrameSize(cg_size(size)));
        self.scroll.update_bar();
        // An offset asked for earlier may be reachable now.
        let pending = &self.scroll.ivars().pending;
        if let Some(target) = pending.get()
            && self.move_to(target, false)
        {
            pending.set(None);
        }
    }

    /// The viewport covers the node's box.
    pub fn set_viewport(&mut self, size: Size) {
        self.viewport = size;
        self.programmatic(|| {
            self.scroll
                .setFrame(CGRect::new(CGPoint::ZERO, cg_size(size)))
        });
        self.layout_document();
    }

    pub fn set_content(&mut self, size: Size) {
        self.content = size;
        self.layout_document();
    }

    pub fn configure(&mut self, config: &ScrollConfig) {
        self.horizontal = config.horizontal;
        self.scroll.ivars().enabled.set(config.enabled);
        let indicators = config.enabled
            && config.shows_indicators
            && config
                .scrollbar_thickness
                .is_none_or(|thickness| thickness > 0.0);
        // A styled bar in a reserved gutter is classic, like GTK's (and
        // Chromium's): always shown at the authored thickness and colour.
        // Without a gutter the system's overlay scroller stays.
        let styled = config.scrollbar_thickness.is_some() || config.scrollbar_color.is_some();
        let classic = indicators && !config.horizontal && config.reserved_gutter && styled;
        self.set_classic_bar(classic.then(|| {
            (
                f64::from(config.scrollbar_thickness.unwrap_or(10.0).clamp(0.0, 64.0)),
                f64::from(config.thumb_inset),
                // Without `scrollbar-color`: a neutral grey that reads on
                // light and dark pages alike.
                config
                    .scrollbar_color
                    .unwrap_or(Color::rgba(128, 128, 128, 140)),
            )
        }));
        self.scroll
            .setHasVerticalScroller(indicators && !config.horizontal && !classic);
        self.scroll
            .setHasHorizontalScroller(indicators && config.horizontal);
        let (along, across) = (NSScrollElasticity::Automatic, NSScrollElasticity::None);
        self.scroll
            .setVerticalScrollElasticity(if config.horizontal { across } else { along });
        self.scroll
            .setHorizontalScrollElasticity(if config.horizontal { along } else { across });
        self.layout_document();
    }

    fn set_classic_bar(&self, style: Option<(f64, f64, Color)>) {
        let mut slot = self.scroll.ivars().bar.borrow_mut();
        match style {
            Some((thickness, inset, color)) => {
                let bar = slot.get_or_insert_with(|| {
                    let bar = PamScrollBar::new(self.scroll.mtm(), &self.scroll);
                    self.scroll.addSubview(&bar);
                    bar
                });
                bar.style(thickness, inset, color);
            }
            None => {
                if let Some(bar) = slot.take() {
                    bar.removeFromSuperview();
                }
            }
        }
        drop(slot);
        self.scroll.update_bar();
    }

    /// Moves the viewport as close to `offset` as the content allows.
    /// Returns whether it got there.
    fn move_to(&self, offset: Point, animated: bool) -> bool {
        let clip = self.scroll.contentView();
        let mut wanted = clip.bounds();
        wanted.origin = cg_point(offset);
        let target = clip.constrainBoundsRect(wanted).origin;
        self.programmatic(|| {
            if animated {
                // Implicit animation of the layer-backed clip view. No
                // animator proxy: it is an instance of a private class, and
                // objc2 checks the receiver's class in debug builds.
                NSAnimationContext::beginGrouping();
                let context = NSAnimationContext::currentContext();
                context.setDuration(0.25);
                context.setAllowsImplicitAnimation(true);
                clip.setBoundsOrigin(target);
                NSAnimationContext::endGrouping();
            } else {
                clip.scrollToPoint(target);
            }
            self.scroll.reflectScrolledClipView(&clip);
        });
        (target.x - wanted.origin.x).abs() <= 0.5 && (target.y - wanted.origin.y).abs() <= 0.5
    }

    /// An offset beyond the content laid out so far is remembered and
    /// lands as soon as the content reaches it.
    pub fn scroll_to(&self, offset: Point, animated: bool) {
        let offset = Point::new(offset.x.max(0.0), offset.y.max(0.0));
        let reached = self.move_to(offset, animated);
        self.scroll
            .ivars()
            .pending
            .set((!reached).then_some(offset));
    }

    pub fn destroy(&self) {
        // SAFETY: removes the registration made in `new`.
        unsafe { NSNotificationCenter::defaultCenter().removeObserver(&self.scroll) };
        if let Some(bar) = self.scroll.ivars().bar.borrow_mut().take() {
            bar.removeFromSuperview();
        }
        self.scroll.removeFromSuperview();
    }
}
