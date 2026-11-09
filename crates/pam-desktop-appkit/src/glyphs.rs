//! Glyphs drawn like Chromium on macOS.
//!
//! Chromium does not let Core Graphics draw text into the page: Skia's
//! `SkScalerContext_Mac` rasterizes each glyph once into an offscreen RGB
//! bitmap (black on white, Core Graphics' own subpixel quantization off) at
//! one of four horizontal quarter-pixel phases, turns it into an 8-bit
//! coverage mask, corrects it and blends the mask in the text colour at a
//! whole device pixel. Core Graphics drawing straight into a layer places
//! and antialiases glyphs differently (it keeps fewer positions above 11 px
//! and resamples the rest), so the same text differs from Chromium's in
//! almost every glyph edge.
//!
//! This module reproduces that pipeline for the layout manager
//! (`text::GlyphPlacer`): masks are rasterized with Skia's flags, corrected
//! with Skia's tables (`-webkit-font-smoothing: auto` only: Core Graphics'
//! smoothing gamma undone, then Skia's sRGB mask gamma for the text
//! luminance) and kept as 8-bit coverage. Each glyph run blends its masks
//! into one buffer drawn as a single image mask in the fill colour. Masks
//! live in one cache per process, shared by every view and window: least
//! recently used first above `BUDGET` bytes, released whole when no window
//! is visible.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr::NonNull;

use objc2::Message;
use objc2::rc::Retained;
use objc2_app_kit::{NSColor, NSColorSpace, NSFont};
use objc2_core_foundation::{
    CFData, CFRetained, CGAffineTransform, CGFloat, CGPoint, CGRect, CGSize,
};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextGetData, CGColorSpace, CGContext, CGDataProvider,
    CGGlyph, CGImage, CGImageAlphaInfo, CGImageByteOrderInfo, CGInterpolationQuality,
    CGTextDrawingMode,
};
use objc2_core_text::{CTFont, CTFontOrientation, CTFontSymbolicTraits};

/// Bytes of masks kept before the least recently used are dropped.
const BUDGET: usize = 1024 * 1024;
/// Bookkeeping per cached glyph (map entry, vector).
const ENTRY_OVERHEAD: usize = 64;
/// Glyphs larger than this (device pixels) are left to Core Graphics.
const MAX_SIDE: i32 = 256;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    font: usize,
    glyph: CGGlyph,
    /// Quarter-pixel phase, 0-3.
    phase: u8,
    /// Smoothed (`auto`) masks also depend on the text luminance level.
    smooth: bool,
    level: u8,
}

struct Mask {
    /// 8-bit coverage, `width` x `height`; empty for a glyph without ink
    /// (a space).
    coverage: Vec<u8>,
    /// Offset of the mask's top-left pixel from the glyph origin, in device
    /// pixels, y down.
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    used: u64,
}

impl Mask {
    fn bytes(&self) -> usize {
        ENTRY_OVERHEAD + self.coverage.capacity()
    }
}

#[derive(Default)]
struct Cache {
    masks: HashMap<Key, Mask>,
    /// Fonts by id, kept alive so that an address seen again is the same
    /// font.
    fonts: HashMap<usize, Retained<NSFont>>,
    next_font: usize,
    bytes: usize,
    clock: u64,
    /// The offscreen RGB bitmap glyphs are drawn into, grown as needed.
    offscreen: Option<(CFRetained<CGContext>, usize, usize)>,
    /// Skia's sRGB correcting tables, one per luminance level.
    tables: Option<Box<[[u8; 256]; 8]>>,
    smooth_behavior: Option<Smoothing>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Smoothing {
    /// Smoothing changes nothing.
    None,
    /// Smoothing dilates glyphs (macOS 10.14+).
    Some,
    /// Smoothing draws subpixel (LCD) coverage.
    Subpixel,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

/// Whether glyphs are drawn as Chromium's masks: opt-in with
/// `PAM_DESKTOP_TEXT_GAMMA=1` (see docs/text-rendering.md for the cost).
pub fn enabled() -> bool {
    thread_local! {
        static ENABLED: bool = std::env::var("PAM_DESKTOP_TEXT_GAMMA").is_ok_and(|value| value == "1");
    }
    ENABLED.with(|enabled| *enabled)
}

/// Drops every cached mask, font and buffer (no window is visible).
pub fn release() {
    CACHE.with(|cache| {
        if let Ok(mut cache) = cache.try_borrow_mut() {
            let behavior = cache.smooth_behavior;
            *cache = Cache {
                smooth_behavior: behavior,
                ..Cache::default()
            };
        }
    });
}

/// Draws `glyphs` of `font` at `positions` (user space of `context`, a
/// flipped context, y down, whose device space is an axis-aligned scale of
/// it) like Chromium does. Returns `false`, having drawn nothing, when the
/// font or the context is not one the pipeline handles (colour glyphs,
/// rotated or skewed spaces, glyphs larger than `MAX_SIDE`): the caller
/// then lets Core Graphics draw them.
pub fn draw(
    context: &CGContext,
    font: &NSFont,
    glyphs: &[CGGlyph],
    positions: &[CGPoint],
    color: Option<&NSColor>,
    smooth: bool,
) -> bool {
    let device = CGContext::user_space_to_device_space_transform(Some(context));
    // Device pixels must be a plain scale of user space (either y
    // direction: layer contexts flip in the CTM or in the device transform).
    if device.b != 0.0 || device.c != 0.0 || device.a <= 0.0 || device.d == 0.0 {
        return false;
    }
    if (device.a - device.d.abs()).abs() > f64::EPSILON {
        return false;
    }
    // SAFETY: `NSFont` is toll-free bridged with `CTFont`.
    let ct: &CTFont = unsafe { &*(font as *const NSFont).cast::<CTFont>() };
    // SAFETY: plain property read.
    if unsafe { ct.symbolic_traits() }.contains(CTFontSymbolicTraits::TraitColorGlyphs) {
        return false;
    }
    let scale = device.a;
    CACHE.with(|cache| {
        let Ok(mut cache) = cache.try_borrow_mut() else {
            return false;
        };
        let smooth = smooth && cache.smoothing() != Smoothing::None;
        let level = if smooth {
            luminance_level(color, cache.smoothing())
        } else {
            0
        };
        let font_id = cache.font_id(font);
        let mut placed = Vec::with_capacity(glyphs.len());
        // Device pixels counted downwards, like Skia's.
        let flip = device.d.signum();
        let (scale_y, offset_y) = (device.d.abs(), flip * device.ty);
        for (&glyph, point) in glyphs.iter().zip(positions) {
            let x = device.a * point.x + device.tx;
            let y = scale_y * point.y + offset_y;
            // Skia's subpixel rounding: x to the nearest quarter, y to the
            // nearest pixel.
            let quarters = ((x + 0.125) * 4.0).floor();
            let column = (quarters / 4.0).floor();
            let phase = (quarters - column * 4.0) as u8;
            let row = (y + 0.5).floor();
            let key = Key {
                font: font_id,
                glyph,
                phase,
                smooth,
                level,
            };
            if !cache.ensure(key, ct, scale) {
                return false;
            }
            placed.push((key, column, row));
        }
        cache.clock += 1;
        let clock = cache.clock;
        // The run's glyphs are blended into one coverage buffer (two
        // overlapping edges cover `a + b - ab`, as two separate blends in
        // one colour would) and drawn with a single image mask.
        let (mut x0, mut y0, mut x1, mut y1) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
        for &(key, column, row) in &placed {
            let Some(mask) = cache.masks.get_mut(&key) else {
                continue;
            };
            mask.used = clock;
            if mask.coverage.is_empty() {
                continue;
            }
            let (left, top) = (
                column as i64 + i64::from(mask.left),
                row as i64 + i64::from(mask.top),
            );
            x0 = x0.min(left);
            y0 = y0.min(top);
            x1 = x1.max(left + i64::from(mask.width));
            y1 = y1.max(top + i64::from(mask.height));
        }
        if x0 >= x1 || y0 >= y1 {
            cache.trim();
            return true;
        }
        let (width, height) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let mut coverage = vec![0u8; width * height];
        for &(key, column, row) in &placed {
            let Some(mask) = cache.masks.get(&key) else {
                continue;
            };
            if mask.coverage.is_empty() {
                continue;
            }
            let left = (column as i64 + i64::from(mask.left) - x0) as usize;
            let top = (row as i64 + i64::from(mask.top) - y0) as usize;
            for (y, source) in mask.coverage.chunks_exact(mask.width as usize).enumerate() {
                let start = (top + y) * width + left;
                for (target, &value) in coverage[start..start + source.len()].iter_mut().zip(source)
                {
                    let (a, b) = (u32::from(*target), u32::from(value));
                    *target = (a + b - (a * b + 127) / 255) as u8;
                }
            }
        }
        // Image-mask samples: 0 paints, 255 leaves the destination.
        for value in &mut coverage {
            *value = 0xFF - *value;
        }
        // SAFETY: `coverage` is a valid byte buffer of its length.
        let data = unsafe { CFData::new(None, coverage.as_ptr(), coverage.len() as isize) };
        drop(coverage);
        let provider = data
            .as_deref()
            .and_then(|data| CGDataProvider::with_cf_data(Some(data)));
        // SAFETY: an 8-bit mask of `width` x `height` samples, rows packed.
        let image = provider.and_then(|provider| unsafe {
            CGImage::mask_create(
                width,
                height,
                8,
                8,
                width,
                Some(&provider),
                std::ptr::null(),
                false,
            )
        });
        if let Some(image) = image {
            CGContext::save_g_state(Some(context));
            if let Some(color) = color {
                CGContext::set_fill_color_with_color(Some(context), Some(&color.CGColor()));
            }
            CGContext::set_interpolation_quality(Some(context), CGInterpolationQuality::None);
            // Images are drawn bottom row first from the rect's origin: in
            // a y-down space they would be upside down.
            CGContext::scale_ctm(Some(context), 1.0, -1.0);
            let left = (x0 as f64 - device.tx) / device.a;
            let top = (y0 as f64 - offset_y) / scale_y;
            let size = CGSize::new(width as f64 / device.a, height as f64 / scale_y);
            CGContext::draw_image(
                Some(context),
                CGRect::new(CGPoint::new(left, -(top + size.height)), size),
                Some(&image),
            );
            CGContext::restore_g_state(Some(context));
        }
        cache.trim();
        true
    })
}

impl Cache {
    /// The cache's id for `font`.
    fn font_id(&mut self, font: &NSFont) -> usize {
        if let Some((&id, _)) = self
            .fonts
            .iter()
            .find(|(_, kept)| std::ptr::eq(&***kept, font))
        {
            return id;
        }
        let id = self.next_font;
        self.next_font += 1;
        self.fonts.insert(id, font.retain());
        id
    }

    fn smoothing(&mut self) -> Smoothing {
        *self.smooth_behavior.get_or_insert_with(smoothing_behavior)
    }

    /// Rasterizes `key` unless cached. `false` when the glyph is too large.
    fn ensure(&mut self, key: Key, font: &CTFont, scale: f64) -> bool {
        if let Some(mask) = self.masks.get(&key) {
            return mask.width <= MAX_SIDE && mask.height <= MAX_SIDE;
        }
        let mask = self.rasterize(key, font, scale);
        let fits = mask.width <= MAX_SIDE && mask.height <= MAX_SIDE;
        self.bytes += mask.bytes();
        self.masks.insert(key, mask);
        fits
    }

    /// `SkScalerContext_Mac::generateMetrics` + `generateImage`.
    /// Fonts keep their size (and optical size); a device scale goes into
    /// the text matrix, as Skia's `fTransform`.
    fn rasterize(&mut self, key: Key, font: &CTFont, scale: f64) -> Mask {
        let mut glyph = key.glyph;
        let mut bounds = CGRect::default();
        // SAFETY: one glyph, one rect.
        unsafe {
            font.bounding_rects_for_glyphs(
                CTFontOrientation::Horizontal,
                NonNull::from(&mut glyph),
                &mut bounds,
                1,
            )
        };
        let empty = Mask {
            coverage: Vec::new(),
            left: 0,
            top: 0,
            width: 0,
            height: 0,
            used: 0,
        };
        if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
            return empty;
        }
        let sub_x = f64::from(key.phase) / 4.0;
        // Device pixels, y down from the baseline; the right edge grows by
        // the phase, the box is rounded out and outset by one pixel.
        let left = (bounds.origin.x * scale).floor() as i32 - 1;
        let top = (-(bounds.origin.y + bounds.size.height) * scale).floor() as i32 - 1;
        let right = ((bounds.origin.x + bounds.size.width) * scale + sub_x).ceil() as i32 + 1;
        let bottom = (-bounds.origin.y * scale).ceil() as i32 + 1;
        let (width, height) = (right - left, bottom - top);
        if width > MAX_SIDE || height > MAX_SIDE {
            return Mask {
                width,
                height,
                ..empty
            };
        }
        let table = if key.smooth {
            Some(self.tables()[usize::from(key.level)])
        } else {
            None
        };
        let Some((pixels, stride)) =
            self.draw_offscreen(font, glyph, key, (left, top, width, height), scale)
        else {
            return empty;
        };
        let mut coverage = Vec::with_capacity((width * height) as usize);
        for row in pixels.chunks_exact(stride).take(height as usize) {
            for &pixel in &row[..width as usize] {
                let (mut r, mut g, mut b) =
                    ((pixel >> 16) & 0xFF, (pixel >> 8) & 0xFF, pixel & 0xFF);
                if key.smooth {
                    // Core Graphics smooths with a 2.0 coverage gamma.
                    (r, g, b) = (pow2(r), pow2(g), pow2(b));
                }
                let mut value = luminance(0xFF - r, 0xFF - g, 0xFF - b);
                if let Some(table) = &table {
                    value = u32::from(table[value as usize]);
                }
                coverage.push(value as u8);
            }
        }
        if coverage.iter().all(|&value| value == 0) {
            return empty;
        }
        Mask {
            coverage,
            left,
            top,
            width,
            height,
            used: 0,
        }
    }

    /// `SkScalerContext_Mac::Offscreen::getCG`: the glyph drawn black on
    /// white into a device RGB bitmap; returns its pixels and row stride.
    fn draw_offscreen(
        &mut self,
        font: &CTFont,
        glyph: CGGlyph,
        key: Key,
        (left, top, width, height): (i32, i32, i32, i32),
        scale: f64,
    ) -> Option<(&[u32], usize)> {
        let (width, height) = (width as usize, height as usize);
        let grow = self
            .offscreen
            .as_ref()
            .is_none_or(|(_, w, h)| *w < width || *h < height);
        if grow {
            let (w, h) = self.offscreen.as_ref().map_or((0, 0), |(_, w, h)| (*w, *h));
            let (w, h) = (
                w.max(width).next_power_of_two(),
                h.max(height).next_power_of_two(),
            );
            let space = CGColorSpace::new_device_rgb()?;
            let info = CGImageByteOrderInfo::Order32Little.0 | CGImageAlphaInfo::NoneSkipFirst.0;
            // SAFETY: Core Graphics allocates and owns the pixels.
            let context = unsafe {
                CGBitmapContextCreate(std::ptr::null_mut(), w, h, 8, w * 4, Some(&space), info)
            }?;
            // Skia does quantization and subpixel positioning itself.
            CGContext::set_allows_font_subpixel_quantization(Some(&context), false);
            CGContext::set_should_subpixel_quantize_fonts(Some(&context), false);
            CGContext::set_allows_font_subpixel_positioning(Some(&context), true);
            CGContext::set_should_subpixel_position_fonts(Some(&context), true);
            CGContext::set_text_drawing_mode(Some(&context), CGTextDrawingMode::Fill);
            CGContext::set_gray_fill_color(Some(&context), 0.0, 1.0);
            CGContext::set_should_antialias(Some(&context), true);
            self.offscreen = Some((context, w, h));
        }
        let (context, stride, rows) = self.offscreen.as_ref()?;
        let (stride, rows) = (*stride, *rows);
        CGContext::set_should_smooth_fonts(Some(context), key.smooth);
        CGContext::set_text_matrix(
            Some(context),
            CGAffineTransform {
                a: scale,
                b: 0.0,
                c: 0.0,
                d: scale,
                tx: 0.0,
                ty: 0.0,
            },
        );
        let data = CGBitmapContextGetData(Some(context)).cast::<u32>();
        if data.is_null() {
            return None;
        }
        // SAFETY: the bitmap holds `stride * rows` pixels.
        let pixels = unsafe { std::slice::from_raw_parts_mut(data, stride * rows) };
        // White, only where the glyph goes (rows from the top of the bitmap).
        for row in pixels.chunks_exact_mut(stride).take(height) {
            row[..width].fill(0xFFFF_FFFF);
        }
        // The bitmap's y axis points up from its bottom row: the glyph's
        // box occupies its top `height` rows.
        // Text space: positions are scaled by the text matrix.
        let point = CGPoint::new(
            (f64::from(-left) + f64::from(key.phase) / 4.0) / scale,
            (rows as f64 + f64::from(top)) / scale,
        );
        let mut glyph = glyph;
        // SAFETY: one glyph at one position, into a live bitmap context.
        unsafe { font.draw_glyphs(NonNull::from(&mut glyph), NonNull::from(&point), 1, context) };
        CGContext::flush(Some(context));
        Some((&pixels[..], stride))
    }

    /// Skia's `SkTMaskGamma<3,3,3>` correcting tables for gamma sRGB,
    /// contrast 0 (`SK_GAMMA_SRGB`, `SK_GAMMA_CONTRAST=0.0` on macOS).
    fn tables(&mut self) -> &[[u8; 256]; 8] {
        self.tables.get_or_insert_with(|| {
            let mut tables = Box::new([[0u8; 256]; 8]);
            for (level, table) in tables.iter_mut().enumerate() {
                // sk_t_scale255<3>
                let level = level as u32;
                let source = (level << 5) | (level << 2) | (level >> 1);
                correcting_table(table, source);
            }
            tables
        })
    }

    /// Drops the least recently used masks while over budget.
    fn trim(&mut self) {
        if self.bytes <= BUDGET {
            return;
        }
        let mut ages: Vec<(u64, Key)> = self
            .masks
            .iter()
            .map(|(key, mask)| (mask.used, *key))
            .collect();
        ages.sort_unstable_by_key(|(used, _)| *used);
        let target = BUDGET * 3 / 4;
        for (_, key) in ages {
            if self.bytes <= target {
                break;
            }
            if let Some(mask) = self.masks.remove(&key) {
                self.bytes -= mask.bytes();
            }
        }
        let fonts: std::collections::HashSet<usize> =
            self.masks.keys().map(|key| key.font).collect();
        self.fonts.retain(|id, _| fonts.contains(id));
    }
}

/// `sk_pow2_table`: undoes Core Graphics' 2.0 smoothing gamma.
fn pow2(value: u32) -> u32 {
    (value * value + 128) / 255
}

/// `SkComputeLuminance`.
fn luminance(r: u32, g: u32, b: u32) -> u32 {
    (r * 54 + g * 183 + b * 19) >> 8
}

/// The luminance level (3 bits) of the text colour Skia picks the mask
/// gamma with, after `SkTypeface_Mac::onFilterRec` scales the colour for
/// Core Graphics' smoothing.
fn luminance_level(color: Option<&NSColor>, smoothing: Smoothing) -> u8 {
    let (r, g, b) = color
        .and_then(|color| color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace()))
        .map_or((0, 0, 0), |color| {
            let channel = |value: CGFloat| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
            (
                channel(color.redComponent()),
                channel(color.greenComponent()),
                channel(color.blueComponent()),
            )
        });
    let (r, g, b) = match smoothing {
        Smoothing::Subpixel => (r * 3 / 4, g * 3 / 4, b * 3 / 4),
        _ => (r / 2, g / 2, b / 2),
    };
    (luminance(r, g, b) >> 5) as u8
}

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f32) -> f32 {
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

/// `SkTMaskGamma_build_correcting_lut` with contrast 0 and sRGB gamma.
fn correcting_table(table: &mut [u8; 256], source: u32) {
    let src = source as f32 / 255.0;
    let lin_src = srgb_to_linear(src);
    let dst = 1.0 - src;
    let lin_dst = srgb_to_linear(dst);
    if (src - dst).abs() < 1.0 / 256.0 {
        for (i, entry) in table.iter_mut().enumerate() {
            *entry = (255.0 * (i as f32 / 255.0)).round() as u8;
        }
        return;
    }
    for (i, entry) in table.iter_mut().enumerate() {
        let srca = i as f32 / 255.0;
        let lin_out = lin_src * srca + (1.0 - srca) * lin_dst;
        let out = linear_to_srgb(lin_out);
        let result = (out - dst) / (src - dst);
        *entry = (255.0 * result).round().clamp(0.0, 255.0) as u8;
    }
}

/// `SkCTFontGetSmoothBehavior`: draws a glyph with and without smoothing.
fn smoothing_behavior() -> Smoothing {
    let Some(space) = CGColorSpace::new_device_rgb() else {
        return Smoothing::None;
    };
    let font = NSFont::systemFontOfSize(16.0);
    // SAFETY: `NSFont` is toll-free bridged with `CTFont`.
    let ct: &CTFont = unsafe { &*(&*font as *const NSFont).cast::<CTFont>() };
    let mut glyph: CGGlyph = 0;
    let mut character: u16 = u16::from(b'W');
    // SAFETY: one character, one glyph.
    unsafe {
        ct.glyphs_for_characters(NonNull::from(&mut character), NonNull::from(&mut glyph), 1)
    };
    let draw = |smooth: bool| -> Option<Vec<u32>> {
        let info = CGImageByteOrderInfo::Order32Little.0 | CGImageAlphaInfo::NoneSkipFirst.0;
        // SAFETY: Core Graphics allocates and owns the pixels.
        let context = unsafe {
            CGBitmapContextCreate(std::ptr::null_mut(), 24, 24, 8, 24 * 4, Some(&space), info)
        }?;
        CGContext::set_should_smooth_fonts(Some(&context), smooth);
        CGContext::set_should_antialias(Some(&context), true);
        CGContext::set_text_drawing_mode(Some(&context), CGTextDrawingMode::Fill);
        CGContext::set_gray_fill_color(Some(&context), 1.0, 1.0);
        let point = CGPoint::new(2.0, 5.0);
        let mut glyph = glyph;
        // SAFETY: one glyph into a live bitmap context.
        unsafe {
            ct.draw_glyphs(
                NonNull::from(&mut glyph),
                NonNull::from(&point),
                1,
                &context,
            )
        };
        CGContext::flush(Some(&context));
        let data = CGBitmapContextGetData(Some(&context)).cast::<u32>();
        // SAFETY: the bitmap holds 24 x 24 pixels.
        (!data.is_null()).then(|| unsafe { std::slice::from_raw_parts(data, 24 * 24) }.to_vec())
    };
    let (Some(plain), Some(smooth)) = (draw(false), draw(true)) else {
        return Smoothing::None;
    };
    let mut behavior = Smoothing::None;
    for (&a, &b) in plain.iter().zip(&smooth) {
        let (r, g, bl) = ((b >> 16) & 0xFF, (b >> 8) & 0xFF, b & 0xFF);
        if r != g || r != bl {
            return Smoothing::Subpixel;
        }
        if a != b {
            behavior = Smoothing::Some;
        }
    }
    behavior
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correcting_tables_keep_the_ends() {
        let mut table = [0u8; 256];
        for level in 0..8u32 {
            correcting_table(&mut table, (level << 5) | (level << 2) | (level >> 1));
            assert_eq!(table[0], 0);
            assert_eq!(table[255], 255);
        }
        // Dark text (level 0) thins the mask, light text thickens it.
        correcting_table(&mut table, 0);
        assert!(table[128] < 128);
        correcting_table(&mut table, 255);
        assert!(table[128] > 128);
    }

    #[test]
    fn pow2_undoes_the_smoothing_gamma() {
        assert_eq!(pow2(0), 0);
        assert_eq!(pow2(255), 255);
        assert_eq!(pow2(128), 64);
    }
}
