//! Scrolling viewports: an `NSScrollView` whose document view holds the
//! node's children at their content coordinates.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use objc2::rc::{Retained, Weak};
use objc2::runtime::NSObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAnimationContext, NSBorderType, NSEvent, NSResponder, NSScrollElasticity, NSScrollView,
    NSScrollerStyle, NSView, NSViewBoundsDidChangeNotification, NSWindowOrderingMode,
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
    /// The classic bars drawn in reserved gutters (`scrollbar-gutter:
    /// stable`, `scrollbar-width`/`scrollbar-color`), like Chromium's on
    /// macOS with a mouse: vertical, then horizontal (`overflow: auto`).
    bars: RefCell<Vec<Retained<PamScrollBar>>>,
    /// The gutters the shown classic bars take from the clip view: width of
    /// the vertical one, height of the horizontal one.
    gutters: Cell<(f64, f64)>,
    /// The content's size from the engine (the document may be larger).
    content: Cell<CGSize>,
    /// The track behind each classic bar (same order as `bars`).
    /// Fill, content-side line and outer line views of each track.
    tracks: RefCell<Vec<[Retained<PlainView>; 3]>>,
    /// How the tracks and thumbs look; `None` paints no track.
    track: Cell<Option<TrackStyle>>,
    /// Chromium's default bar (no `scrollbar-width`/`scrollbar-color`):
    /// its colours follow the colour scheme of the page behind it.
    default_bar: Cell<bool>,
}

/// The track of a classic bar: its fill and, for Chromium's default bar on
/// macOS, a 1 pt line on the content side and one on the outer edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackStyle {
    pub fill: Color,
    pub inner: Option<Color>,
    pub outer: Option<Color>,
}

/// Chromium's default classic bar on macOS (legacy scrollers, measured on
/// Electron 37): the regular 15 pt geometry; dark colour scheme: track
/// #2C2C2C between a #3D3D3D line on the content side and a #515151 line
/// on the outer edge, thumb #6B6B6B; light: track #FAFAFA, lines #E8E8E8
/// and #EDEDED, thumb #C1C1C1. Chromium picks them by CSS `color-scheme`;
/// natively the first opaque background behind the bar tells (like GTK).
fn default_bar_colors(dark: bool) -> (TrackStyle, Color) {
    let grey = |value: u8| Color::rgba(value, value, value, 255);
    if dark {
        (
            TrackStyle {
                fill: grey(44),
                inner: Some(grey(61)),
                outer: Some(grey(81)),
            },
            grey(107),
        )
    } else {
        (
            TrackStyle {
                fill: grey(LIGHT[0]),
                inner: Some(grey(LIGHT[1])),
                outer: Some(grey(LIGHT[2])),
            },
            grey(LIGHT[3]),
        )
    }
}

/// Light colour scheme: track, content-side line, outer line, thumb.
const LIGHT: [u8; 4] = [250, 232, 237, 193];

/// A classic bar's geometry across the bar: the gutter, the thumb inside it
/// and the thumb's gap to the outer edge (the rest is on the content side),
/// and the track's margin at both ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarGeometry {
    pub thickness: f64,
    pub thumb: f64,
    pub outer: f64,
    pub margin: f64,
}

impl BarGeometry {
    /// Chromium's classic bars on macOS (measured on Electron 37 with
    /// legacy scrollers): `scrollbar-color` alone is the regular 15 pt bar
    /// with an 8 pt thumb 3 pt from the edge, `scrollbar-width: thin` the
    /// small 11 pt bar with a 6 pt thumb 2 pt from the edge; the track ends
    /// 3 pt inside the viewport. An authored thickness
    /// (`::-webkit-scrollbar`) is a thumb `inset` inside its gutter.
    pub fn for_config(thickness: Option<f32>, inset: f32) -> Self {
        match thickness {
            None => Self {
                thickness: 15.0,
                thumb: 8.0,
                outer: 3.0,
                margin: 3.0,
            },
            Some(_) if inset > 0.0 => Self {
                thickness: f64::from(pam_desktop_ui::engine::SCROLLBAR_GUTTER_THIN),
                thumb: 6.0,
                outer: 2.0,
                margin: 3.0,
            },
            Some(thickness) => {
                let thickness = f64::from(thickness.clamp(0.0, 64.0));
                let inset = f64::from(inset).clamp(0.0, thickness / 4.0);
                Self {
                    thickness,
                    thumb: thickness - 2.0 * inset,
                    outer: inset,
                    margin: inset + 1.0,
                }
            }
        }
    }
}

pub struct BarIvars {
    scroll: Weak<PamScrollView>,
    geometry: Cell<BarGeometry>,
    /// Pointer position along the track and the offset when the thumb was grabbed.
    grab: Cell<Option<(f64, f64)>>,
    /// A horizontal bar along the bottom edge.
    horizontal: Cell<bool>,
}

define_class!(
    // SAFETY: `NSView` allows subclassing; the overrides keep its
    // contracts. Main thread only.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = BarIvars]
    /// The thumb of a classic scrollbar, painted by its layer.
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
                let offset = scroll.offset();
                let offset = if self.ivars().horizontal.get() { offset.x } else { offset.y };
                self.ivars().grab.set(Some((along, f64::from(offset))));
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            let (Some((start, offset)), Some(scroll)) = (self.ivars().grab.get(), self.ivars().scroll.load()) else {
                return;
            };
            let horizontal = self.ivars().horizontal.get();
            let (viewport, content) = scroll.extent(horizontal);
            let size = self.frame().size;
            let margin = self.ivars().geometry.get().margin;
            let track = viewport - 2.0 * margin - if horizontal { size.width } else { size.height };
            if track > 0.0 {
                let moved = (self.along(event) - start) * (content - viewport) / track;
                scroll.user_scroll_to(horizontal, offset + moved);
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
            geometry: Cell::new(BarGeometry::for_config(None, 0.0)),
            grab: Cell::new(None),
            horizontal: Cell::new(false),
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
                if self.ivars().horizontal.get() {
                    point.x
                } else if parent.isFlipped() {
                    point.y
                } else {
                    -point.y
                }
            }
            None if self.ivars().horizontal.get() => event.locationInWindow().x,
            None => event.locationInWindow().y,
        }
    }

    fn style(&self, geometry: BarGeometry, color: Color, horizontal: bool) {
        self.ivars().horizontal.set(horizontal);
        self.ivars().geometry.set(geometry);
        if let Some(layer) = self.layer() {
            layer.setBackgroundColor(Some(&cg_color(color)));
            layer.setCornerRadius(geometry.thumb / 2.0);
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

        /// The clip view leaves the classic bars' gutters to them, like a
        /// legacy scroller's.
        #[unsafe(method(tile))]
        fn tile(&self) {
            // SAFETY: forwards the same selector to `NSScrollView`.
            let _: () = unsafe { msg_send![super(self), tile] };
            let (width, height) = self.ivars().gutters.get();
            if width > 0.0 || height > 0.0 {
                let clip = self.contentView();
                let mut frame = clip.frame();
                frame.size.width = (frame.size.width - width).max(0.0);
                frame.size.height = (frame.size.height - height).max(0.0);
                if !self.isFlipped() {
                    frame.origin.y += height;
                }
                clip.setFrame(frame);
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
            self.update_bar();
        }
    }
);

impl PamScrollView {
    /// Lays the clip view out again for new gutters.
    fn retile(&self) {
        // SAFETY: `tile` takes no argument and returns nothing.
        let _: () = unsafe { msg_send![self, tile] };
    }

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

    /// Viewport and content lengths along the scrolling axis.
    fn extent(&self, horizontal: bool) -> (f64, f64) {
        let viewport = self.contentView().bounds().size;
        let content = self
            .documentView()
            .map_or(CGSize::ZERO, |document| document.frame().size);
        if horizontal {
            (viewport.width, content.width)
        } else {
            (viewport.height, content.height)
        }
    }

    /// Moves the viewport for the user (the classic bar's thumb).
    fn user_scroll_to(&self, horizontal: bool, value: f64) {
        let clip = self.contentView();
        let mut wanted = clip.bounds();
        if horizontal {
            wanted.origin.x = value;
        } else {
            wanted.origin.y = value;
        }
        let target = clip.constrainBoundsRect(wanted).origin;
        clip.scrollToPoint(target);
        self.reflectScrolledClipView(&clip);
    }

    /// Which classic bars show: the content overflows the clip view left
    /// by the other bar (CSS 2.1 §11.1.1). Returns whether the gutters
    /// changed.
    fn update_gutters(&self) -> bool {
        let bars = self.ivars().bars.borrow();
        let size = self.frame().size;
        let content = self.ivars().content.get();
        let thickness = |horizontal: bool| {
            bars.iter()
                .find(|bar| bar.ivars().horizontal.get() == horizontal)
                .map_or(0.0, |bar| bar.ivars().geometry.get().thickness)
        };
        let (across, along) = (thickness(false), thickness(true));
        let mut gutters = (0.0, 0.0);
        for _ in 0..3 {
            gutters = (
                if across > 0.0 && content.height > size.height - gutters.1 + 0.5 {
                    across
                } else {
                    0.0
                },
                if along > 0.0 && content.width > size.width - gutters.0 + 0.5 {
                    along
                } else {
                    0.0
                },
            );
        }
        self.ivars().gutters.replace(gutters) != gutters
    }

    /// Shows `track` along the whole gutter of a bar (the corner between
    /// two bars belongs to the vertical one), with its edge lines.
    fn paint_track(
        &self,
        track: &[Retained<PlainView>; 3],
        style: Option<TrackStyle>,
        geometry: BarGeometry,
        horizontal: bool,
        size: CGSize,
        gutters: (f64, f64),
    ) {
        let Some(style) = style else {
            for part in track {
                part.setHidden(true);
            }
            return;
        };
        let thickness = geometry.thickness;
        let frame = if horizontal {
            let y = if self.isFlipped() {
                size.height - thickness
            } else {
                0.0
            };
            CGRect::new(
                CGPoint::new(0.0, y),
                CGSize::new((size.width - gutters.0).max(0.0), thickness),
            )
        } else {
            CGRect::new(
                CGPoint::new(size.width - thickness, 0.0),
                CGSize::new(thickness, size.height),
            )
        };
        // Across the bar: the content side first, the outer edge last.
        let line = |across: f64| {
            if horizontal {
                let y = if self.isFlipped() {
                    frame.origin.y + across
                } else {
                    frame.origin.y + thickness - 1.0 - across
                };
                CGRect::new(
                    CGPoint::new(frame.origin.x, y),
                    CGSize::new(frame.size.width, 1.0),
                )
            } else {
                CGRect::new(
                    CGPoint::new(frame.origin.x + across, frame.origin.y),
                    CGSize::new(1.0, frame.size.height),
                )
            }
        };
        let parts = [
            (Some(style.fill), frame),
            (style.inner, line(0.0)),
            (style.outer, line(thickness - 1.0)),
        ];
        for (part, (color, rect)) in track.iter().zip(parts) {
            match (color, part.layer()) {
                (Some(color), Some(layer)) => {
                    part.setFrame(rect);
                    layer.setBackgroundColor(Some(&cg_color(color)));
                    part.setHidden(false);
                }
                _ => part.setHidden(true),
            }
        }
    }

    /// Places each classic bar's thumb in its gutter (proportional, at least
    /// 40 points long); hidden while everything fits, the gutter staying
    /// reserved by the layout.
    fn update_bar(&self) {
        let bars = self.ivars().bars.borrow().clone();
        let tracks = self.ivars().tracks.borrow().clone();
        let size = self.frame().size;
        let gutters = self.ivars().gutters.get();
        // The default bar takes the scheme of the page around it, which
        // may only be known once the scroll view is in place.
        if self.ivars().default_bar.get()
            && let Some(dark) = crate::view::backdrop_is_dark(self)
        {
            let (track, thumb) = default_bar_colors(dark);
            self.ivars().track.set(Some(track));
            for bar in &bars {
                let geometry = bar.ivars().geometry.get();
                bar.style(geometry, thumb, bar.ivars().horizontal.get());
            }
        }
        let track_style = self.ivars().track.get();
        for (index, bar) in bars.into_iter().enumerate() {
            let track = tracks.get(index);
            let horizontal = bar.ivars().horizontal.get();
            let (viewport, content) = self.extent(horizontal);
            let geometry = bar.ivars().geometry.get();
            let shown = if horizontal { gutters.1 } else { gutters.0 } > 0.0;
            if !shown || content <= viewport + 0.5 || viewport <= 0.0 {
                bar.setHidden(true);
                for part in track.into_iter().flatten() {
                    part.setHidden(true);
                }
                continue;
            }
            if let Some(track) = track {
                self.paint_track(track, track_style, geometry, horizontal, size, gutters);
            }
            let margin = geometry.margin;
            let track = (viewport - 2.0 * margin).max(0.0);
            let length = (track * viewport / content).max(40.0).min(track);
            let offset = self.offset();
            let offset = f64::from(if horizontal { offset.x } else { offset.y });
            let along = margin + (track - length) * offset / (content - viewport);
            let edge = geometry.thickness - geometry.outer - geometry.thumb;
            let frame = if horizontal {
                // Along the bottom edge of the gutter.
                let y = if self.isFlipped() {
                    size.height - geometry.thickness + edge
                } else {
                    geometry.outer
                };
                CGRect::new(CGPoint::new(along, y), CGSize::new(length, geometry.thumb))
            } else {
                let y = if self.isFlipped() {
                    along
                } else {
                    size.height - along - length
                };
                CGRect::new(
                    CGPoint::new(size.width - geometry.thickness + edge, y),
                    CGSize::new(geometry.thumb, length),
                )
            };
            bar.setFrame(frame);
            bar.setHidden(false);
        }
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
    /// `overflow: auto`: the document follows the content on both axes.
    both: bool,
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
            bars: RefCell::new(Vec::new()),
            gutters: Cell::new((0.0, 0.0)),
            content: Cell::new(CGSize::ZERO),
            tracks: RefCell::new(Vec::new()),
            track: Cell::new(None),
            default_bar: Cell::new(false),
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
            both: false,
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
    /// exactly as wide as the clip view across it, so it only scrolls one way.
    fn layout_document(&self) {
        self.scroll.ivars().content.set(cg_size(self.content));
        if self.scroll.update_gutters() {
            self.programmatic(|| self.scroll.retile());
        }
        let clip = self.scroll.contentView().frame().size;
        let viewport = Size::new(clip.width as f32, clip.height as f32);
        let size = if self.both {
            Size::new(
                self.content.width.max(viewport.width),
                self.content.height.max(viewport.height),
            )
        } else if self.horizontal {
            Size::new(self.content.width.max(viewport.width), viewport.height)
        } else {
            Size::new(viewport.width, self.content.height.max(viewport.height))
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
        self.both = config.both;
        self.scroll.ivars().enabled.set(config.enabled);
        let indicators = config.enabled
            && config.shows_indicators
            && config
                .scrollbar_thickness
                .is_none_or(|thickness| thickness > 0.0);
        // A bar in a reserved gutter is classic, like Chromium's with legacy
        // scrollers: drawn at the authored thickness and colours, or as
        // Chromium's default bar, on each scrolling axis. Without a gutter
        // the system's overlay scroller stays.
        let styled = config.scrollbar_thickness.is_some() || config.scrollbar_color.is_some();
        let classic = indicators && config.reserved_gutter;
        let default_bar = classic && !styled;
        self.scroll.ivars().default_bar.set(default_bar);
        let style = classic.then(|| {
            if default_bar {
                let dark = crate::view::backdrop_is_dark(&self.scroll).unwrap_or(false);
                (
                    BarGeometry::for_config(None, 0.0),
                    default_bar_colors(dark).1,
                )
            } else {
                (
                    BarGeometry::for_config(config.scrollbar_thickness, config.thumb_inset),
                    // Without `scrollbar-color`: a neutral grey that reads on
                    // light and dark pages alike.
                    config
                        .scrollbar_color
                        .unwrap_or(Color::rgba(128, 128, 128, 140)),
                )
            }
        });
        self.scroll.ivars().track.set(if default_bar {
            let dark = crate::view::backdrop_is_dark(&self.scroll).unwrap_or(false);
            Some(default_bar_colors(dark).0)
        } else if classic {
            config.scrollbar_track.map(|fill| TrackStyle {
                fill,
                inner: None,
                outer: None,
            })
        } else {
            None
        });
        let axes: &[bool] = if config.both {
            &[false, true]
        } else if config.horizontal {
            &[true]
        } else {
            &[false]
        };
        self.set_classic_bars(if classic { axes } else { &[] }, style);
        self.scroll
            .setHasVerticalScroller(indicators && (config.both || !config.horizontal) && !classic);
        self.scroll
            .setHasHorizontalScroller(indicators && (config.both || config.horizontal) && !classic);
        let (along, across) = (NSScrollElasticity::Automatic, NSScrollElasticity::None);
        self.scroll
            .setVerticalScrollElasticity(if config.horizontal && !config.both {
                across
            } else {
                along
            });
        self.scroll
            .setHorizontalScrollElasticity(if config.horizontal || config.both {
                along
            } else {
                across
            });
        self.layout_document();
    }

    fn set_classic_bars(&self, axes: &[bool], style: Option<(BarGeometry, Color)>) {
        let mut bars = self.scroll.ivars().bars.borrow_mut();
        while bars.len() > axes.len() {
            if let Some(bar) = bars.pop() {
                bar.removeFromSuperview();
            }
        }
        while bars.len() < axes.len() {
            let bar = PamScrollBar::new(self.scroll.mtm(), &self.scroll);
            self.scroll.addSubview(&bar);
            bars.push(bar);
        }
        let mut tracks = self.scroll.ivars().tracks.borrow_mut();
        while tracks.len() > axes.len() {
            for part in tracks.pop().into_iter().flatten() {
                part.removeFromSuperview();
            }
        }
        while tracks.len() < axes.len() {
            let mtm = self.scroll.mtm();
            let track = [
                PlainView::new(mtm),
                PlainView::new(mtm),
                PlainView::new(mtm),
            ];
            for part in &track {
                part.setHidden(true);
                self.scroll.addSubview_positioned_relativeTo(
                    part,
                    NSWindowOrderingMode::Below,
                    Some(&bars[tracks.len()]),
                );
            }
            tracks.push(track);
        }
        drop(tracks);
        if let Some((geometry, color)) = style {
            for (bar, &horizontal) in bars.iter().zip(axes) {
                bar.style(geometry, color, horizontal);
            }
        }
        drop(bars);
        if self.scroll.update_gutters() {
            self.programmatic(|| self.scroll.retile());
        }
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
        for bar in self.scroll.ivars().bars.borrow_mut().drain(..) {
            bar.removeFromSuperview();
        }
        for part in self.scroll.ivars().tracks.borrow_mut().drain(..).flatten() {
            part.removeFromSuperview();
        }
        self.scroll.removeFromSuperview();
    }
}
