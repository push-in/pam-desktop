//! Chromium's `backdrop-filter: blur()` and translucent fills, on 8-bit
//! pixels, for backends that composite a backdrop on the CPU.
//!
//! Measured on Electron 37 (Linux and macOS, identical): the blur is Skia's
//! three-box approximation of a Gaussian (box size
//! `d = ⌊σ·3·√(2π)/4 + ½⌋`, three boxes of `d`, or `d`, `d`, `d + 1` when `d`
//! is even), one axis at a time (x, then y), each rounded to 8 bits, with
//! transparent edges; the blurred result is drawn over the unblurred
//! backdrop, and a translucent fill is drawn over that. Every blend is
//! Skia's 8-bit one, see [`skia_over`].

use crate::color::Color;
use crate::style::BackdropBox;

/// Chromium composites a translucent fill in 8-bit integers (Skia: the
/// premultiplied source rounded, the destination scaled by `256 - alpha` and
/// shifted): `rgba(2,6,24,.7)` over `v` is `(1,4,17) + ⌊v·77/256⌋`.
#[must_use]
pub fn skia_over(source: Color, under: Color) -> Color {
    let alpha = u32::from(source.a);
    let channel = |s: u8, d: u8| {
        let premultiplied = (u32::from(s) * alpha + 127) / 255;
        pam_desktop_pixels::over_channel(premultiplied, alpha, d)
    };
    Color {
        r: channel(source.r, under.r),
        g: channel(source.g, under.g),
        b: channel(source.b, under.b),
        a: 255,
    }
}

pub use pam_desktop_pixels::box_sizes;

/// [`pam_desktop_pixels::composite`] on RGBA pixels with the view's fill
/// and the boxes' shadows in logical units (× `scale`, relative to the
/// image).
#[allow(clippy::too_many_arguments)]
pub fn composite(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    stride: usize,
    sigma: f32,
    fill: Option<Color>,
    boxes: &[BackdropBox],
    scale: f32,
) {
    let rgba = |color: Color| [color.r, color.g, color.b, color.a];
    let boxes: Vec<_> = boxes
        .iter()
        .map(|painted| pam_desktop_pixels::PaintedBox {
            rect: [
                painted.rect.x * scale,
                painted.rect.y * scale,
                painted.rect.right() * scale,
                painted.rect.bottom() * scale,
            ],
            radii: [
                painted.radii.top_left,
                painted.radii.top_right,
                painted.radii.bottom_right,
                painted.radii.bottom_left,
            ]
            .map(|radius| radius * scale),
            shadow: painted.shadow.filter(|shadow| !shadow.inset).map(|shadow| {
                pam_desktop_pixels::BoxShadow {
                    offset: [shadow.offset.x * scale, shadow.offset.y * scale],
                    blur: shadow.blur * scale,
                    spread: shadow.spread * scale,
                    color: rgba(shadow.color),
                }
            }),
            fill: painted.fill.map(rgba),
            clip: painted
                .clip
                .map(|clip| [clip.x, clip.y, clip.right(), clip.bottom()].map(|edge| edge * scale)),
        })
        .collect();
    pam_desktop_pixels::composite(pixels, width, height, stride, sigma, fill.map(rgba), &boxes);
}

/// The plain translucent background of a backdrop-filter box, which a
/// backend composites into the blurred backdrop.
#[must_use]
pub fn backdrop_fill(decoration: &crate::style::Decoration) -> Option<Color> {
    let fill = decoration.background;
    (decoration.backdrop_blur > 0.0
        && fill.a > 0
        && fill.a < 255
        && decoration.gradients.is_empty())
    .then_some(fill)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gray(width: usize, height: usize, value: impl Fn(usize, usize) -> u8) -> Vec<u8> {
        let mut pixels = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                let v = value(x, y);
                pixels.extend_from_slice(&[v, v, v, 255]);
            }
        }
        pixels
    }

    /// The dialogs' `box-shadow: 0 24px 64px rgba(0,0,0,.68)` on an 8px
    /// rounded box at (300, 200), 600 × 300.
    fn dialog_shadow() -> BackdropBox {
        BackdropBox {
            rect: crate::geometry::Rect::new(300.0, 200.0, 600.0, 300.0),
            radii: crate::style::Radii {
                top_left: 8.0,
                top_right: 8.0,
                bottom_right: 8.0,
                bottom_left: 8.0,
            },
            shadow: Some(crate::style::Shadow {
                offset: crate::geometry::Point::new(0.0, 24.0),
                blur: 64.0,
                spread: 0.0,
                color: Color::rgba(0, 0, 0, 173),
                inset: false,
            }),
            fill: None,
            clip: None,
        }
    }

    #[test]
    fn shadows_match_electron() {
        // Electron 37: the shadow over white, left of the box (row 374) and
        // below it (column 600).
        let (width, height) = (1200, 800);
        let mut pixels = gray(width, height, |_, _| 255);
        composite(
            &mut pixels,
            width,
            height,
            width * 4,
            0.0,
            None,
            &[dialog_shadow()],
            1.0,
        );
        let row: Vec<u8> = (270..300).map(|x| pixels[(374 * width + x) * 4]).collect();
        assert_eq!(
            row,
            [
                225, 224, 222, 220, 219, 217, 216, 214, 212, 210, 208, 207, 205, 203, 201, 199,
                197, 195, 193, 191, 189, 187, 185, 183, 180, 178, 176, 174, 172, 170
            ]
        );
        let column: Vec<u8> = (524..540).map(|y| pixels[(y * width + 600) * 4]).collect();
        assert_eq!(
            column,
            [
                170, 172, 174, 176, 178, 180, 183, 185, 187, 189, 191, 193, 195, 197, 199, 201
            ]
        );
    }

    #[test]
    fn box_sizes_follow_skia() {
        assert_eq!(box_sizes(8.0), vec![15, 15, 15]);
        assert_eq!(box_sizes(3.0), vec![6, 6, 7]);
        assert!(box_sizes(0.5).is_empty());
    }

    #[test]
    fn blur_matches_electron() {
        // Electron 37 captures, backdrop-filter: blur(8px) over a window
        // white left of x = 600 and black right of it (1200×800).
        let (width, height) = (1200, 800);
        let mut pixels = gray(width, height, |x, _| if x < 600 { 255 } else { 0 });
        composite(&mut pixels, width, height, width * 4, 8.0, None, &[], 1.0);
        let row: Vec<u8> = (590..610).map(|x| pixels[(150 * width + x) * 4]).collect();
        assert_eq!(
            row,
            [
                227, 221, 213, 204, 194, 183, 171, 159, 147, 134, 121, 108, 96, 84, 72, 61, 51, 42,
                34, 28
            ]
        );
        // At the top edge half of the blur is the unblurred backdrop.
        let top: Vec<u8> = (0..6).map(|y| pixels[(y * width + 600) * 4]).collect();
        assert_eq!(top, [64, 70, 75, 81, 87, 92]);
        let left_column = (0..6)
            .map(|y| pixels[(y * width + 590) * 4])
            .collect::<Vec<_>>();
        assert_eq!(left_column, [240, 238, 238, 236, 235, 233]);
    }

    /// `cargo test --release -p pam-desktop-ui --lib backdrop_cost -- --ignored --nocapture`
    #[test]
    #[ignore = "timing"]
    fn backdrop_cost() {
        let (width, height) = (1200, 800);
        let mut pixels = gray(width, height, |x, y| ((x * 7 + y * 13) % 256) as u8);
        let start = std::time::Instant::now();
        for _ in 0..5 {
            composite(
                &mut pixels,
                width,
                height,
                width * 4,
                8.0,
                Some(Color::rgba(2, 6, 24, 179)),
                &[dialog_shadow()],
                1.0,
            );
        }
        println!("composite 1200x800 sigma 8: {:?} each", start.elapsed() / 5);
    }

    #[test]
    fn stacked_masks_match_electron() {
        // Three slate-950/70 masks with blur(8px) over black: (1,5,23).
        let fill = Color::rgba(2, 6, 24, 179);
        let mut pixels = gray(64, 64, |_, _| 0);
        for _ in 0..3 {
            composite(&mut pixels, 64, 64, 64 * 4, 8.0, Some(fill), &[], 1.0);
        }
        assert_eq!(
            &pixels[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 3],
            &[1, 5, 23]
        );
        assert_eq!(
            skia_over(fill, Color::rgba(255, 255, 255, 255)),
            Color::rgba(77, 80, 93, 255)
        );
    }
}
