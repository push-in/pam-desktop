//! Small conversions between the core's types and Win32 / Direct2D ones.

use pam_desktop_ui::color::Color;
use pam_desktop_ui::geometry::{Affine, Point, Rect};
use windows::Win32::Foundation::{COLORREF, LPARAM, RECT};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D_RECT_F};
use windows_numerics::{Matrix3x2, Vector2};

/// A NUL-terminated UTF-16 copy of `text`, for `PCWSTR` parameters.
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The text of a UTF-16 buffer up to its first NUL.
pub fn from_wide(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|unit| *unit == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

pub const fn loword(value: usize) -> u16 {
    (value & 0xFFFF) as u16
}

pub const fn hiword(value: usize) -> u16 {
    ((value >> 16) & 0xFFFF) as u16
}

/// Signed client coordinates packed in an `LPARAM` (`GET_X_LPARAM`/`GET_Y_LPARAM`).
pub const fn lparam_point(lparam: LPARAM) -> (i32, i32) {
    let value = lparam.0 as usize;
    (loword(value) as i16 as i32, hiword(value) as i16 as i32)
}

/// Straight-alpha Direct2D color.
pub fn color_f(color: Color) -> D2D1_COLOR_F {
    let [r, g, b, a] = color.to_f32();
    D2D1_COLOR_F { r, g, b, a }
}

/// GDI color (`0x00BBGGRR`); alpha is dropped.
pub const fn colorref(color: Color) -> COLORREF {
    COLORREF((color.r as u32) | ((color.g as u32) << 8) | ((color.b as u32) << 16))
}

/// An opaque color from a GDI `COLORREF`.
pub const fn from_colorref(value: u32) -> Color {
    Color::rgba((value & 0xFF) as u8, ((value >> 8) & 0xFF) as u8, ((value >> 16) & 0xFF) as u8, 255)
}

pub const fn rect_f(rect: Rect) -> D2D_RECT_F {
    D2D_RECT_F { left: rect.x, top: rect.y, right: rect.x + rect.width, bottom: rect.y + rect.height }
}

pub const fn vec2(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

pub const fn point_vec(point: Point) -> Vector2 {
    Vector2 { X: point.x, Y: point.y }
}

/// Direct2D uses row vectors: `[a b; c d; tx ty]`.
pub const fn matrix(affine: &Affine) -> Matrix3x2 {
    Matrix3x2 { M11: affine.a, M12: affine.b, M21: affine.c, M22: affine.d, M31: affine.tx, M32: affine.ty }
}

/// Logical rectangle to device pixels, rounded outwards so nothing is cut.
pub fn pixel_rect(rect: Rect, scale: f32) -> RECT {
    RECT {
        left: (rect.x * scale).floor() as i32,
        top: (rect.y * scale).floor() as i32,
        right: (rect.right() * scale).ceil() as i32,
        bottom: (rect.bottom() * scale).ceil() as i32,
    }
}

/// Logical rectangle to the nearest device pixels (child window placement).
pub fn pixel_rect_nearest(rect: Rect, scale: f32) -> RECT {
    RECT {
        left: (rect.x * scale).round() as i32,
        top: (rect.y * scale).round() as i32,
        right: (rect.right() * scale).round() as i32,
        bottom: (rect.bottom() * scale).round() as i32,
    }
}

/// Number of UTF-16 code units of `text`.
pub fn utf16_len(text: &str) -> u32 {
    text.encode_utf16().count() as u32
}
