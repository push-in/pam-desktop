//! Raster cache of the rows of a scroll view, for GTK's software renderer.
//!
//! GTK keeps the render node of every widget between frames, but its Cairo
//! renderer (no GPU: virtual machines, containers, remote sessions) draws
//! every node of the visible rows again at each frame of a scroll: text,
//! rounded borders and clips are rasterized on the main thread every time.
//! Chromium scrolls from raster tiles it already has instead.
//!
//! While such a renderer draws a scroll view that scrolls, each child of its
//! content that is on screen is rasterized once into an image, and the
//! frames of the scroll only place those images (a copy per row instead of
//! drawing it); children off screen are left out of the frame instead of
//! being clipped away by the renderer after their paths were built. A row is
//! rasterized again when its render node changes (GTK gives it a new one
//! when anything inside it changed), and a row that keeps changing (an
//! animation, a video, a caret) is drawn directly for a while. Rows more than a screen
//! away from the visible area release their image, so the cache never
//! holds much more than three screens of pixels, and a second after the
//! scroll ends every image goes and rows are drawn directly again.
//!
//! Images are placed on whole device pixels (an image placed between
//! pixels would be resampled, blurring text): a row may move by less than
//! half a pixel compared with drawing it directly. GPU renderers composite
//! on their own and never use this cache.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;
use gtk::{cairo, gdk, glib, graphene, gsk};

/// Rows larger than this many device pixels are always drawn directly.
const MAX_PIXELS: i64 = 4096 * 1024;

/// A row whose render node changed this many frames in a row is drawn
/// directly for [`VOLATILE_FRAMES`] frames.
const MAX_MISSES: u8 = 3;
const VOLATILE_FRAMES: u16 = 60;

struct Entry {
    /// The render node the image was rasterized from.
    source: gsk::RenderNode,
    /// A node painting the image, in the row's own coordinates.
    image: Option<gsk::RenderNode>,
    misses: u8,
    volatile: u16,
}

/// How long rows stay cached after the last scroll.
pub const ROWS_KEPT: Duration = Duration::from_secs(1);

#[derive(Default)]
pub struct RowCache {
    entries: HashMap<usize, Entry>,
    /// When the view last scrolled; none: not scrolling, nothing cached.
    scrolled_at: Option<Instant>,
}

/// Whether `widget` is drawn by GTK's Cairo (software) renderer.
pub fn software_rendered(widget: &impl IsA<gtk::Widget>) -> bool {
    widget
        .native()
        .and_then(|native| native.renderer())
        .is_some_and(|renderer| renderer.is::<gsk::CairoRenderer>())
}

fn node_ptr(node: &gsk::RenderNode) -> usize {
    let pointer: *mut gsk::ffi::GskRenderNode = node.to_glib_none().0;
    pointer as usize
}

fn intersects(a: &graphene::Rect, b: &graphene::Rect) -> bool {
    a.x() < b.x() + b.width()
        && b.x() < a.x() + a.width()
        && a.y() < b.y() + b.height()
        && b.y() < a.y() + a.height()
}

fn grown(rect: &graphene::Rect, x: f32, y: f32) -> graphene::Rect {
    graphene::Rect::new(
        rect.x() - x,
        rect.y() - y,
        rect.width() + 2.0 * x,
        rect.height() + 2.0 * y,
    )
}

impl RowCache {
    /// Whether rows are drawn from images (a scroll is in progress).
    pub fn active(&self) -> bool {
        self.scrolled_at.is_some()
    }

    /// Notes a scroll; true when it starts one (the caller arranges
    /// [`Self::release`]).
    pub fn scrolled(&mut self) -> bool {
        self.scrolled_at.replace(Instant::now()).is_none()
    }

    /// Lets the images go once the scroll has been over for [`ROWS_KEPT`];
    /// otherwise returns how much longer to wait.
    pub fn release(&mut self) -> Option<Duration> {
        let remaining = ROWS_KEPT.saturating_sub(self.scrolled_at?.elapsed());
        if !remaining.is_zero() {
            return Some(remaining);
        }
        self.scrolled_at = None;
        self.entries.clear();
        None
    }

    /// Snapshots the children of `view`, the content of a scroll view: the
    /// ones on screen from their images. False when the cache does not
    /// apply (another renderer, not inside a viewport): the caller
    /// snapshots the children itself.
    pub fn snapshot_children(&mut self, view: &gtk::Widget, snapshot: &gtk::Snapshot) -> bool {
        if self.scrolled_at.is_none() || !software_rendered(view) {
            self.entries.clear();
            return false;
        }
        let Some(viewport) = view.parent().filter(|parent| parent.is::<gtk::Viewport>()) else {
            self.entries.clear();
            return false;
        };
        let (Some(visible), Some(native)) = (viewport.compute_bounds(view), view.native()) else {
            return false;
        };
        let Some(origin) = view.compute_point(&native, &graphene::Point::new(0.0, 0.0)) else {
            return false;
        };
        let scale = view.scale_factor().max(1) as f32;
        // Shadows and outlines paint a little outside their widget.
        let visible = grown(&visible, 32.0, 32.0);
        let keep = grown(&visible, visible.width(), visible.height());
        let mut entries = HashMap::with_capacity(self.entries.len());
        let mut child = view.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            let key = current.as_ptr() as usize;
            let Some(bounds) = current.compute_bounds(view) else {
                view.snapshot_child(&current, snapshot);
                continue;
            };
            if !intersects(&bounds, &visible) {
                // Off screen: not in this frame. The content is snapshotted
                // again whenever the viewport moves or changes size.
                if intersects(&bounds, &keep)
                    && let Some(entry) = self.entries.remove(&key)
                {
                    entries.insert(key, entry);
                }
                continue;
            }
            let recorder = gtk::Snapshot::new();
            view.snapshot_child(&current, &recorder);
            let Some(node) = recorder.to_node() else {
                continue;
            };
            let (inner, dx, dy) = match node.downcast_ref::<gsk::TransformNode>() {
                Some(transformed) => {
                    let transform = transformed.transform();
                    if transform.category() < gsk::TransformCategory::_2dTranslate {
                        snapshot.append_node(&node);
                        continue;
                    }
                    let (dx, dy) = transform.to_translate();
                    (transformed.child(), dx, dy)
                }
                None => (node.clone(), 0.0, 0.0),
            };
            let mut entry = match self.entries.remove(&key) {
                Some(entry) if node_ptr(&entry.source) == node_ptr(&inner) => {
                    Entry { misses: 0, ..entry }
                }
                Some(entry) => Entry {
                    source: inner.clone(),
                    image: None,
                    misses: entry.misses.saturating_add(1),
                    volatile: entry.volatile,
                },
                None => Entry {
                    source: inner.clone(),
                    image: None,
                    misses: 0,
                    volatile: 0,
                },
            };
            if entry.misses >= MAX_MISSES {
                entry.volatile = VOLATILE_FRAMES;
                entry.misses = 0;
            }
            if entry.volatile > 0 {
                entry.volatile -= 1;
                entry.image = None;
            } else if entry.image.is_none() {
                entry.image = rasterize(&inner, scale);
            }
            match &entry.image {
                Some(image) => {
                    // Whole device pixels: the image is copied, not resampled.
                    let snap = |position: f32| ((position * scale).round() / scale) - position;
                    snapshot.save();
                    snapshot.translate(&graphene::Point::new(
                        dx + snap(origin.x() + dx),
                        dy + snap(origin.y() + dy),
                    ));
                    snapshot.append_node(image);
                    snapshot.restore();
                }
                None => snapshot.append_node(&node),
            }
            entries.insert(key, entry);
        }
        self.entries = entries;
        true
    }
}

/// Draws `node` into an image of whole device pixels and returns a node
/// that paints it, in the coordinates of `node`.
fn rasterize(node: &gsk::RenderNode, scale: f32) -> Option<gsk::RenderNode> {
    let bounds = node.bounds();
    let left = bounds.x().floor();
    let top = bounds.y().floor();
    let width = (bounds.x() + bounds.width()).ceil() - left;
    let height = (bounds.y() + bounds.height()).ceil() - top;
    let pixel_width = (width * scale).ceil() as i32;
    let pixel_height = (height * scale).ceil() as i32;
    if pixel_width <= 0
        || pixel_height <= 0
        || i64::from(pixel_width) * i64::from(pixel_height) > MAX_PIXELS
    {
        return None;
    }
    let surface =
        cairo::ImageSurface::create(cairo::Format::ARgb32, pixel_width, pixel_height).ok()?;
    surface.set_device_scale(f64::from(scale), f64::from(scale));
    {
        let context = cairo::Context::new(&surface).ok()?;
        context.translate(-f64::from(left), -f64::from(top));
        node.draw(&context);
    }
    surface.flush();
    // A Cairo node keeps the image surface itself: painting it copies the
    // pixels (the X11 backend keeps an uploaded copy of it), where a texture
    // would be converted to a new surface at every frame.
    let recorder = gtk::Snapshot::new();
    let rect = graphene::Rect::new(
        left,
        top,
        pixel_width as f32 / scale,
        pixel_height as f32 / scale,
    );
    let context = recorder.append_cairo(&rect);
    context
        .set_source_surface(&surface, f64::from(left), f64::from(top))
        .ok()?;
    context.paint().ok()?;
    drop(context);
    recorder.to_node()
}

/// How long the frame clock keeps a steady cadence after the last scroll.
const SCROLL_PACE: Duration = Duration::from_millis(150);

thread_local! {
    /// The frame clock kept updating by a scroll, and when it last moved.
    static PACED: RefCell<Option<(gdk::FrameClock, Instant)>> = const { RefCell::new(None) };
}

/// Keeps the frame clock of `view` updating while it scrolls.
///
/// Without a compositor reporting presented frames (X11 without one, a
/// virtual machine, a remote session) GTK times each frame that input asks
/// for from when the previous one actually started, so a scroll ran at
/// 18-19 ms per frame (about 53 per second) instead of 16.7. While the
/// clock is "updating" (as for an animation) frames stay on a fixed
/// cadence. It stops updating shortly after the scroll ends.
pub fn keep_pace(view: &gtk::Widget) {
    let Some(clock) = view.frame_clock() else {
        return;
    };
    let start = PACED.with(|paced| {
        let mut paced = paced.borrow_mut();
        match paced.as_mut() {
            Some((current, last)) if *current == clock => {
                *last = Instant::now();
                false
            }
            Some(_) => false,
            None => {
                *paced = Some((clock.clone(), Instant::now()));
                true
            }
        }
    });
    if start {
        clock.begin_updating();
        arm_pace(SCROLL_PACE);
    }
}

fn arm_pace(delay: Duration) {
    glib::timeout_add_local_once(delay, || {
        let remaining = PACED.with(|paced| {
            paced
                .borrow()
                .as_ref()
                .map(|(_, last)| SCROLL_PACE.saturating_sub(last.elapsed()))
        });
        match remaining {
            Some(remaining) if !remaining.is_zero() => arm_pace(remaining),
            _ => {
                if let Some((clock, _)) = PACED.with(|paced| paced.borrow_mut().take()) {
                    clock.end_updating();
                }
            }
        }
    });
}
