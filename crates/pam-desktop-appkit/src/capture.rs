//! `PAM_DESKTOP_CAPTURE=<file.png>`: writes the content area of the first
//! visible window to a PNG file `PAM_DESKTOP_CAPTURE_AFTER` seconds (3 by
//! default) after it shows. A process may read its own windows without the
//! screen recording permission, also on a locked session, so this is how
//! visual checks capture an application on macOS.

use std::cell::Cell;
use std::ffi::OsString;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAppearanceCustomization, NSApplication, NSBitmapImageFileType, NSBitmapImageRep,
    NSGraphicsContext, NSView, NSWindow,
};
use objc2_core_graphics::CGContext;
use objc2_foundation::{NSDictionary, NSRunLoop, NSRunLoopCommonModes, NSTimer};

pub fn schedule(mtm: MainThreadMarker) {
    let Some(path) = std::env::var_os("PAM_DESKTOP_CAPTURE") else {
        return;
    };
    let delay = std::env::var("PAM_DESKTOP_CAPTURE_AFTER")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(3.0);
    after(delay, move || attempt(mtm, path, 60));
}

/// Captures once a window is visible, checking every second at most
/// `attempts` times.
fn attempt(mtm: MainThreadMarker, path: OsString, attempts: u32) {
    let windows = NSApplication::sharedApplication(mtm).windows();
    let Some(window) = windows.iter().find(|window| window.isVisible()) else {
        if attempts == 0 {
            eprintln!("pam-desktop: no visible window to capture");
        } else {
            after(1.0, move || attempt(mtm, path, attempts - 1));
        }
        return;
    };
    match capture(&window).map(|png| std::fs::write(&path, png)) {
        Some(Ok(())) => eprintln!("pam-desktop: captured {}", path.to_string_lossy()),
        Some(Err(error)) => eprintln!("pam-desktop: cannot write the capture: {error}"),
        None => eprintln!("pam-desktop: the window could not be captured"),
    }
}

fn after(seconds: f64, job: impl FnOnce() + 'static) {
    let job = Cell::new(Some(job));
    let block = RcBlock::new(move |_: NonNull<NSTimer>| {
        if let Some(job) = job.take() {
            job();
        }
    });
    // SAFETY: the block runs once on the main run loop; the mode is an
    // immutable string of Foundation.
    unsafe {
        let timer = NSTimer::timerWithTimeInterval_repeats_block(seconds, false, &block);
        NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
    }
}

fn capture(window: &NSWindow) -> Option<Vec<u8>> {
    let content = window.contentView()?;
    let bitmap = drawn(window, &content)?;
    // SAFETY: an empty dictionary is a valid set of PNG properties.
    let data = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }?;
    Some(data.to_vec())
}

/// The window background, then the layer tree of the content view.
/// `renderInContext:` leaves out what only the window server composites:
/// layer shadows, masks and Core Image filters.
fn drawn(window: &NSWindow, content: &NSView) -> Option<Retained<NSBitmapImageRep>> {
    let bounds = content.bounds();
    let bitmap = content.bitmapImageRepForCachingDisplayInRect(bounds)?;
    let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&bitmap)?;
    let previous = NSGraphicsContext::currentContext();
    NSGraphicsContext::setCurrentContext(Some(&context));
    let cg = context.CGContext();
    let background = window.backgroundColor();
    // The background is a dynamic color: resolve it in the window's appearance.
    window
        .effectiveAppearance()
        .performAsCurrentDrawingAppearance(&RcBlock::new(|| {
            CGContext::set_fill_color_with_color(Some(&cg), Some(&background.CGColor()));
            CGContext::fill_rect(Some(&cg), bounds);
        }));
    if let Some(layer) = content.layer() {
        // The layers of a flipped view are laid out from the top.
        if content.isFlipped() {
            CGContext::translate_ctm(Some(&cg), 0.0, bounds.size.height);
            CGContext::scale_ctm(Some(&cg), 1.0, -1.0);
        }
        layer.renderInContext(&cg);
    }
    context.flushGraphics();
    NSGraphicsContext::setCurrentContext(previous.as_deref());
    Some(bitmap)
}
