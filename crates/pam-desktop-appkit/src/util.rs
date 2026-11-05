//! Conversions between the core's geometry and colors and AppKit's.

use objc2::rc::Retained;
use objc2_app_kit::NSColor;
use objc2_core_foundation::{CFRetained, CGFloat, CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGColor;
use objc2_foundation::NSString;
use pam_desktop_ui::color::Color;
use pam_desktop_ui::geometry::{Point, Rect, Size};

/// Core Animation raises an exception on geometry that is not finite.
fn finite(value: f32) -> CGFloat {
    if value.is_finite() {
        CGFloat::from(value)
    } else {
        0.0
    }
}

pub fn cg_point(point: Point) -> CGPoint {
    CGPoint::new(finite(point.x), finite(point.y))
}

pub fn cg_size(size: Size) -> CGSize {
    CGSize::new(finite(size.width.max(0.0)), finite(size.height.max(0.0)))
}

pub fn cg_rect(rect: Rect) -> CGRect {
    CGRect::new(cg_point(rect.origin()), cg_size(rect.size()))
}

pub fn point(point: CGPoint) -> Point {
    Point::new(point.x as f32, point.y as f32)
}

pub fn size(size: CGSize) -> Size {
    Size::new(size.width as f32, size.height as f32)
}

/// Engine colors are sRGB; so are the colors handed to Core Graphics.
pub fn cg_color(color: Color) -> CFRetained<CGColor> {
    let [red, green, blue, alpha] = color.to_f32();
    CGColor::new_srgb(
        CGFloat::from(red),
        CGFloat::from(green),
        CGFloat::from(blue),
        CGFloat::from(alpha),
    )
}

pub fn ns_color(color: Color) -> Retained<NSColor> {
    let [red, green, blue, alpha] = color.to_f32();
    NSColor::colorWithSRGBRed_green_blue_alpha(
        CGFloat::from(red),
        CGFloat::from(green),
        CGFloat::from(blue),
        CGFloat::from(alpha),
    )
}

pub fn ns_string(text: &str) -> Retained<NSString> {
    NSString::from_str(text)
}

/// UTF-16 length of a Rust string (AppKit text offsets are UTF-16).
pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}
