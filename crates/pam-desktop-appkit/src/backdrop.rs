//! `backdrop-filter: blur()` without a Metal device.
//!
//! Core Animation runs a layer's background filters on the GPU; a Mac
//! without a Metal device (a virtual machine, some remote sessions) would
//! show the box unblurred. There the blur is done once on the CPU instead:
//! the window's layer tree is rendered without the box and whatever paints
//! after it, the part under the box goes through Chromium's pipeline
//! (`pam_desktop_ui::backdrop`: Skia's box blur over the unblurred backdrop,
//! then the box's translucent fill, all in 8 bits, in the encoded colour
//! space), and the result is the contents of a view placed just below the
//! box, whose own fill is then not painted. It is redone only when a view of
//! the window changed, at most ten times a second, and only when the pixels
//! under the box differ from the last ones blurred. Until the first result
//! the host shows the fill alone.

use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{AnyThread, Message};
use objc2_app_kit::{NSBitmapFormat, NSBitmapImageRep, NSImage, NSView};
use objc2_core_foundation::{CGFloat, CGRect, CGSize};
use objc2_core_graphics::CGImage;
use objc2_foundation::{NSRunLoop, NSRunLoopCommonModes, NSTimer};
use objc2_quartz_core::CATransaction;
use pam_desktop_ui::color::Color;
use pam_desktop_ui::style::BackdropBox;

/// Shortest time between two refreshes.
const INTERVAL: Duration = Duration::from_millis(100);

/// A box blurring what is behind it, and the view that shows the result.
pub struct Backdrop {
    /// Below the box (and its shadow), same frame.
    pub host: Retained<NSView>,
    /// The box and the views that paint with it (its shadow host).
    pub covers: Vec<Retained<NSView>>,
    /// Blur radius in points (CSS `blur()`).
    pub radius: f32,
    /// Corner radius of the box (uniform), in points.
    pub corner: f32,
    /// The box's translucent fill, composited here instead of by the box.
    pub fill: Option<Color>,
    /// Shadows and fills of the boxes over it (`Decoration::backdrop_boxes`).
    pub boxes: Vec<BackdropBox>,
    last: Cell<u64>,
}

thread_local! {
    static BACKDROPS: RefCell<Vec<Backdrop>> = const { RefCell::new(Vec::new()) };
    static SCHEDULED: Cell<bool> = const { Cell::new(false) };
    static LAST: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Starts (or updates) the CPU backdrop of the box whose host is `host`.
pub fn register(
    host: &Retained<NSView>,
    covers: Vec<Retained<NSView>>,
    radius: f32,
    corner: f32,
    fill: Option<Color>,
    boxes: Vec<BackdropBox>,
) {
    if let Some(layer) = host.layer() {
        // Until the first composited backdrop: the fill over the sharp one.
        layer.setBackgroundColor(fill.map(crate::util::cg_color).as_deref());
        layer.setCornerRadius(CGFloat::from(corner.max(0.0)));
    }
    BACKDROPS.with(|backdrops| {
        let mut backdrops = backdrops.borrow_mut();
        if let Some(existing) = backdrops
            .iter_mut()
            .find(|backdrop| Retained::as_ptr(&backdrop.host) == Retained::as_ptr(host))
        {
            existing.covers = covers;
            existing.radius = radius;
            existing.corner = corner;
            existing.fill = fill;
            existing.boxes = boxes;
            existing.last.set(0);
        } else {
            backdrops.push(Backdrop {
                host: host.clone(),
                covers,
                radius,
                corner,
                fill,
                boxes,
                last: Cell::new(0),
            });
        }
    });
    invalidate();
}

/// Stops the CPU backdrop shown by `host`.
pub fn unregister(host: &NSView) {
    BACKDROPS.with(|backdrops| {
        backdrops
            .borrow_mut()
            .retain(|backdrop| !std::ptr::eq(&*backdrop.host, host));
    });
}

/// Something in a window changed: the backdrops are redone soon (no-op
/// without any).
pub fn invalidate() {
    if BACKDROPS.with(|backdrops| backdrops.borrow().is_empty())
        || SCHEDULED.with(|scheduled| scheduled.replace(true))
    {
        return;
    }
    let wait = LAST.with(Cell::get).map_or(Duration::ZERO, |last| {
        INTERVAL.saturating_sub(last.elapsed())
    });
    let block = RcBlock::new(move |_: NonNull<NSTimer>| {
        SCHEDULED.with(|scheduled| scheduled.set(false));
        LAST.with(|last| last.set(Some(Instant::now())));
        refresh();
    });
    // SAFETY: the block runs once on the main run loop; the mode is an
    // immutable string of Foundation.
    unsafe {
        let timer = NSTimer::timerWithTimeInterval_repeats_block(wait.as_secs_f64(), false, &block);
        NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
    }
}

fn refresh() {
    let backdrops = BACKDROPS.with(|backdrops| {
        backdrops
            .borrow()
            .iter()
            .map(|backdrop| {
                (
                    backdrop.host.clone(),
                    backdrop.covers.clone(),
                    backdrop.radius,
                    backdrop.corner,
                    backdrop.fill,
                    backdrop.boxes.clone(),
                    backdrop.last.get(),
                )
            })
            .collect::<Vec<_>>()
    });
    for (host, covers, radius, corner, fill, boxes, last) in backdrops {
        let paint = (fill, boxes.as_slice());
        if let Some(hash) = refresh_one(&host, &covers, radius, corner, paint, last) {
            BACKDROPS.with(|backdrops| {
                if let Some(backdrop) = backdrops
                    .borrow()
                    .iter()
                    .find(|backdrop| Retained::as_ptr(&backdrop.host) == Retained::as_ptr(&host))
                {
                    backdrop.last.set(hash);
                }
            });
        }
    }
}

/// Hides `view` and everything painted after it in the window, returning
/// the layers hidden and their previous state.
fn hide_from(view: &NSView, hidden: &mut Vec<(Retained<objc2_quartz_core::CALayer>, bool)>) {
    let mut current = view.retain();
    // SAFETY: the superviews are only read.
    while let Some(parent) = unsafe { current.superview() } {
        let siblings = parent.subviews();
        let mut after = false;
        for sibling in &siblings {
            if after && let Some(layer) = sibling.layer() {
                hidden.push((layer.clone(), layer.isHidden()));
                layer.setHidden(true);
            }
            if std::ptr::eq(&*sibling, &*current) {
                after = true;
            }
        }
        current = parent;
    }
}

fn refresh_one(
    host: &NSView,
    covers: &[Retained<NSView>],
    radius: f32,
    corner: f32,
    (fill, boxes): (Option<Color>, &[BackdropBox]),
    last: u64,
) -> Option<u64> {
    let window = host.window()?;
    if !window.isVisible() {
        return None;
    }
    let content = window.contentView()?;
    // Views draw their layer contents lazily: a backdrop registered with
    // the views of the same update would read empty layers.
    window.displayIfNeeded();
    CATransaction::flush();
    let mut hidden = Vec::new();
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    for view in std::iter::once(host).chain(covers.iter().map(|view| &**view)) {
        if let Some(layer) = view.layer() {
            hidden.push((layer.clone(), layer.isHidden()));
            layer.setHidden(true);
        }
    }
    if let Some(front) = covers.last() {
        hide_from(front, &mut hidden);
    }
    let bitmap = crate::capture::drawn(&window, &content);
    for (layer, was) in hidden.into_iter().rev() {
        layer.setHidden(was);
    }
    CATransaction::commit();
    let bitmap = bitmap?;
    let bounds = content.bounds();
    let scale = bitmap.pixelsWide() as CGFloat / bounds.size.width.max(1.0);
    let rect = host.convertRect_toView(host.bounds(), Some(&content));
    let top = if content.isFlipped() {
        rect.origin.y
    } else {
        bounds.size.height - rect.origin.y - rect.size.height
    };
    let pixels = CGRect::new(
        objc2_core_foundation::CGPoint::new((rect.origin.x * scale).round(), (top * scale).round()),
        CGSize::new(
            (rect.size.width * scale).round(),
            (rect.size.height * scale).round(),
        ),
    );
    if pixels.size.width < 1.0 || pixels.size.height < 1.0 {
        return None;
    }
    let hash = digest(&bitmap, pixels);
    if hash == last {
        return None;
    }
    // Chromium's blur and fill, written over the region in the bitmap.
    composite(
        &bitmap,
        pixels,
        radius * scale as f32,
        fill,
        boxes,
        scale as f32,
    )?;
    let whole = bitmap.CGImage()?;
    let blurred = CGImage::with_image_in_rect(Some(&whole), pixels)?;
    let image = NSImage::initWithCGImage_size(NSImage::alloc(), &blurred, rect.size);
    let root = host.layer()?;
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    // The view's own layer is AppKit's: the image goes in a sublayer.
    // SAFETY: the sublayers are only read.
    let sublayers = unsafe { root.sublayers() };
    let layer = match sublayers.as_ref().and_then(|layers| layers.firstObject()) {
        Some(layer) => layer,
        None => {
            let layer = objc2_quartz_core::CALayer::new();
            root.addSublayer(&layer);
            layer
        }
    };
    layer.setFrame(host.bounds());
    // SAFETY: a layer's contents may be an `NSImage`.
    unsafe { layer.setContents(Some(&image)) };
    layer.setCornerRadius(CGFloat::from(corner.max(0.0)));
    layer.setMasksToBounds(corner > 0.0);
    CATransaction::commit();
    Some(hash)
}

/// Runs `pam_desktop_ui::backdrop::composite` on the pixels of `rect`
/// (device pixels, top-left origin) of an 8-bit RGBA bitmap.
fn composite(
    bitmap: &NSBitmapImageRep,
    rect: CGRect,
    sigma: f32,
    fill: Option<Color>,
    boxes: &[BackdropBox],
    scale: f32,
) -> Option<()> {
    let format = bitmap.bitmapFormat();
    if bitmap.bitsPerSample() != 8
        || bitmap.samplesPerPixel() != 4
        || bitmap.bitsPerPixel() != 32
        || bitmap.isPlanar()
        || format.contains(NSBitmapFormat::AlphaFirst)
        || format.contains(NSBitmapFormat::FloatingPointSamples)
    {
        return None;
    }
    let data = bitmap.bitmapData();
    if data.is_null() {
        return None;
    }
    let row = bitmap.bytesPerRow() as usize;
    let (x, y) = (rect.origin.x as usize, rect.origin.y as usize);
    let width = (rect.size.width as usize).min((bitmap.pixelsWide() as usize).saturating_sub(x));
    let height = (rect.size.height as usize).min((bitmap.pixelsHigh() as usize).saturating_sub(y));
    if width == 0 || height == 0 {
        return None;
    }
    // SAFETY: the rows y..y+height, bytes x·4..(x+width)·4 lie inside the
    // bitmap's buffer, which nothing else touches meanwhile.
    let region = unsafe {
        std::slice::from_raw_parts_mut(data.add(y * row + x * 4), row * (height - 1) + width * 4)
    };
    pam_desktop_ui::backdrop::composite(region, width, height, row, sigma, fill, boxes, scale);
    Some(())
}

/// A digest of the pixels of `rect` (device pixels, top-left origin).
fn digest(bitmap: &objc2_app_kit::NSBitmapImageRep, rect: CGRect) -> u64 {
    let mut hasher = DefaultHasher::new();
    let data = bitmap.bitmapData();
    let row = bitmap.bytesPerRow() as usize;
    let pixel = (bitmap.bitsPerPixel() / 8).max(1) as usize;
    let height = bitmap.pixelsHigh() as usize;
    if data.is_null() {
        return 0;
    }
    let (x, y) = (rect.origin.x as usize, rect.origin.y as usize);
    let (w, h) = (rect.size.width as usize, rect.size.height as usize);
    for line in y..(y + h).min(height) {
        // SAFETY: the row lies inside the bitmap's buffer.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                data.add(line * row + x * pixel),
                (w * pixel).min(row - x * pixel),
            )
        };
        bytes.hash(&mut hasher);
    }
    hasher.finish().max(1)
}
