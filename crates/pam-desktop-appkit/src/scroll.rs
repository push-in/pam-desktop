//! Scrolling viewports: an `NSScrollView` whose document view holds the
//! node's children at their content coordinates.

use std::cell::Cell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAnimationContext, NSBorderType, NSEvent, NSResponder, NSScrollElasticity, NSScrollView,
    NSScrollerStyle, NSView, NSViewBoundsDidChangeNotification,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSNotification, NSNotificationCenter};
use pam_desktop_ui::backend::{BackendEvent, NodeId, ScrollConfig};
use pam_desktop_ui::geometry::{Point, Size};

use crate::events::Events;
use crate::util::{cg_point, cg_size};
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
        let indicators = config.enabled && config.shows_indicators;
        self.scroll
            .setHasVerticalScroller(indicators && !config.horizontal);
        self.scroll
            .setHasHorizontalScroller(indicators && config.horizontal);
        let (along, across) = (NSScrollElasticity::Automatic, NSScrollElasticity::None);
        self.scroll
            .setVerticalScrollElasticity(if config.horizontal { across } else { along });
        self.scroll
            .setHorizontalScrollElasticity(if config.horizontal { along } else { across });
        self.layout_document();
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
        self.scroll.removeFromSuperview();
    }
}
