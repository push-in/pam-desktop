//! `PAM_DESKTOP_CAPTURE=<file.png>`: writes the content area of the first
//! visible window to a PNG file `PAM_DESKTOP_CAPTURE_AFTER` seconds (3 by
//! default) after it shows. A process may read its own windows without the
//! screen recording permission, also on a locked session, so this is how
//! visual checks capture an application on macOS.
//!
//! `PAM_DESKTOP_CAPTURE_INPUT="click:x,y;sleep:0.7;move:x,y;scroll:x,y,dy"`: input that
//! visual checks replay inside the process once the first window shows
//! (after `PAM_DESKTOP_CAPTURE_AFTER`; the capture waits for it), in content points from the top left
//! like an xdotool recipe (`scroll` turns the wheel by `dy` pixels, positive
//! towards the top). Synthesized events need no accessibility permission and
//! do not move the system pointer.

use std::cell::Cell;
use std::ffi::OsString;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAppearanceCustomization, NSApplication, NSBitmapImageFileType, NSBitmapImageRep, NSEvent,
    NSEventModifierFlags, NSEventType, NSGraphicsContext, NSScreen, NSView, NSWindow,
};
use objc2_core_graphics::{CGContext, CGEvent, CGEventField, CGScrollEventUnit};
use objc2_foundation::{
    NSDictionary, NSPoint, NSProcessInfo, NSRunLoop, NSRunLoopCommonModes, NSTimer,
};

pub fn schedule(mtm: MainThreadMarker) {
    let delay = std::env::var("PAM_DESKTOP_CAPTURE_AFTER")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(3.0);
    let mut replayed = 0.0;
    if let Ok(script) = std::env::var("PAM_DESKTOP_CAPTURE_INPUT") {
        let steps = parse_input(&script);
        // The capture waits for the input and one more second.
        replayed = steps
            .iter()
            .map(|step| match step {
                Step::Sleep(seconds) => *seconds,
                _ => 0.0,
            })
            .sum::<f64>()
            + 1.0;
        after(delay, move || replay(mtm, steps, 0, 60));
    }
    let Some(path) = std::env::var_os("PAM_DESKTOP_CAPTURE") else {
        return;
    };
    after(delay + replayed, move || attempt(mtm, path, 60));
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    Click(f64, f64),
    Move(f64, f64),
    Scroll(f64, f64, f64),
    Sleep(f64),
}

fn parse_input(script: &str) -> Vec<Step> {
    let point = |value: &str| {
        let (x, y) = value.split_once(',')?;
        Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
    };
    script
        .split(';')
        .filter_map(|step| {
            let (kind, value) = step.trim().split_once(':')?;
            match kind {
                "click" => point(value).map(|(x, y)| Step::Click(x, y)),
                "move" => point(value).map(|(x, y)| Step::Move(x, y)),
                "scroll" => {
                    let (at, delta) = value.rsplit_once(',')?;
                    let (x, y) = point(at)?;
                    Some(Step::Scroll(x, y, delta.trim().parse().ok()?))
                }
                "sleep" => value.trim().parse().ok().map(Step::Sleep),
                _ => None,
            }
        })
        .collect()
}

/// Replays `steps[index..]` on the first visible window, waiting for one
/// every second at most `attempts` times.
fn replay(mtm: MainThreadMarker, steps: Vec<Step>, index: usize, attempts: u32) {
    let windows = NSApplication::sharedApplication(mtm).windows();
    let Some(window) = windows.iter().find(|window| window.isVisible()) else {
        if attempts > 0 {
            after(1.0, move || replay(mtm, steps, index, attempts - 1));
        }
        return;
    };
    let mut index = index;
    while let Some(&step) = steps.get(index) {
        index += 1;
        match step {
            Step::Sleep(seconds) => {
                after(seconds, move || replay(mtm, steps, index, attempts));
                return;
            }
            Step::Click(x, y) => {
                for kind in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp] {
                    if let Some(event) = mouse_event(&window, kind, x, y) {
                        window.sendEvent(&event);
                    }
                }
            }
            Step::Scroll(x, y, delta) => {
                if let Some(event) = scroll_event(mtm, &window, x, y, delta) {
                    window.sendEvent(&event);
                }
            }
            Step::Move(x, y) => {
                if let (Some(content), Some(event)) = (
                    window.contentView(),
                    mouse_event(&window, NSEventType::MouseMoved, x, y),
                ) {
                    content.mouseMoved(&event);
                }
            }
        }
    }
}

/// A pixel wheel event over content point `(x, y)` of `window`.
fn scroll_event(
    mtm: MainThreadMarker,
    window: &NSWindow,
    x: f64,
    y: f64,
    delta: f64,
) -> Option<Retained<NSEvent>> {
    let content = window.contentView()?;
    let y = if content.isFlipped() {
        y
    } else {
        content.bounds().size.height - y
    };
    let screen = window.convertPointToScreen(content.convertPoint_toView(NSPoint::new(x, y), None));
    // Quartz places events from the top left of the main display.
    let height = NSScreen::screens(mtm).firstObject()?.frame().size.height;
    let event =
        CGEvent::new_scroll_wheel_event2(None, CGScrollEventUnit::Pixel, 1, delta as i32, 0, 0)?;
    CGEvent::set_location(Some(&event), NSPoint::new(screen.x, height - screen.y));
    for field in [
        CGEventField::MouseEventWindowUnderMousePointer,
        CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent,
    ] {
        CGEvent::set_integer_value_field(Some(&event), field, window.windowNumber() as i64);
    }
    NSEvent::eventWithCGEvent(&event)
}

fn mouse_event(window: &NSWindow, kind: NSEventType, x: f64, y: f64) -> Option<Retained<NSEvent>> {
    let content = window.contentView()?;
    // Content points from the top left, whatever the view's flipping.
    let y = if content.isFlipped() {
        y
    } else {
        content.bounds().size.height - y
    };
    let location = content.convertPoint_toView(NSPoint::new(x, y), None);
    let pressure = if kind == NSEventType::LeftMouseDown {
        1.0
    } else {
        0.0
    };
    NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        kind,
        location,
        NSEventModifierFlags::empty(),
        NSProcessInfo::processInfo().systemUptime(),
        window.windowNumber(),
        None,
        0,
        1,
        pressure,
    )
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
pub(crate) fn drawn(window: &NSWindow, content: &NSView) -> Option<Retained<NSBitmapImageRep>> {
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

#[cfg(test)]
mod tests {
    use super::{Step, parse_input};

    #[test]
    fn input_scripts_read_like_xdotool_recipes() {
        assert_eq!(
            parse_input("click:307,294; sleep:0.7;move:1199, 799;bogus;click:1;scroll:600,400,-40"),
            [
                Step::Click(307.0, 294.0),
                Step::Sleep(0.7),
                Step::Move(1199.0, 799.0),
                Step::Scroll(600.0, 400.0, -40.0)
            ]
        );
    }
}
