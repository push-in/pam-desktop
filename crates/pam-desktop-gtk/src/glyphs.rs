//! Text drawn like Chromium on Linux, from one glyph atlas shared by every
//! view of the process.
//!
//! Skia (Chromium builds it with `SK_GAMMA_APPLY_TO_A8`, gamma 1.2 and
//! contrast 0.2 on Linux) corrects each glyph mask for the luminance of the
//! text colour: light text on dark gains weight, dark text on light loses a
//! little. GSK blends the raw coverage, so the same glyphs look lighter there.
//!
//! A paragraph is walked by a Pango renderer (so backgrounds, selection,
//! underline, strikethrough, letter-spacing, ellipsis and inline boxes come
//! from Pango exactly as for GSK's own renderer); each glyph is rasterized
//! once by cairo with the font's own options (hinting, antialiasing) at the
//! quarter-pixel phase cairo itself would use, its coverage goes through
//! Skia's correcting table for the text colour, and the result is kept in
//! the atlas. Drawing a paragraph then only places texture nodes of cached
//! glyphs on whole device pixels: no texture per paragraph. Colour glyphs
//! (emoji), unknown glyphs and huge glyphs are left to GSK's text node.
//!
//! The atlas is a set of pages (one CPU buffer per batch of new glyphs; each
//! glyph's texture is a view of its page), least recently used pages go
//! first above [`BUDGET_BYTES`], and everything is released when no window
//! of the process is mapped.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::rc::Rc;
use std::sync::OnceLock;

use gtk::glib::translate::FromGlibPtrNone;
use gtk::prelude::*;
use gtk::{cairo, gdk, glib, graphene, gsk, pango};

use glib::gobject_ffi;
use pango::ffi as pango_ffi;

#[link(name = "pangocairo-1.0")]
unsafe extern "C" {
    fn pango_cairo_font_get_scaled_font(
        font: *mut pango_ffi::PangoFont,
    ) -> *mut cairo::ffi::cairo_scaled_font_t;
}

const GAMMA: f32 = 1.2;
const CONTRAST: f32 = 0.2;
/// Luminance levels of the tables (Skia: 3 bits).
const LEVELS: usize = 8;

/// Skia `SkTMaskGamma_build_correcting_lut` for one source luminance.
fn correcting_table(source: u8) -> [u8; 256] {
    let src = f32::from(source) / 255.0;
    let lin_src = src.powf(GAMMA);
    let dst = 1.0 - src;
    let lin_dst = dst.powf(GAMMA);
    // The contrast boost fades out as the text becomes white.
    let contrast = CONTRAST * lin_dst;
    let mut table = [0u8; 256];
    for (index, entry) in table.iter_mut().enumerate() {
        let raw = index as f32 / 255.0;
        let coverage = raw + (1.0 - raw) * contrast * raw;
        let value = if (src - dst).abs() < 1.0 / 256.0 {
            coverage
        } else {
            let linear = lin_src * coverage + (1.0 - coverage) * lin_dst;
            (linear.powf(1.0 / GAMMA) - dst) / (src - dst)
        };
        *entry = (value * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    table
}

fn tables() -> &'static [[u8; 256]; LEVELS] {
    static TABLES: OnceLock<[[u8; 256]; LEVELS]> = OnceLock::new();
    TABLES.get_or_init(|| {
        std::array::from_fn(|level| {
            // sk_t_scale255<3>: the level spread back over 0..255.
            let level = level as u32;
            correcting_table(((level << 5) | (level << 2) | (level >> 1)) as u8)
        })
    })
}

/// Skia's luminance of a text colour (paint gamma), as a table index.
fn level(red: u32, green: u32, blue: u32) -> usize {
    let luma = |channel: u32| (channel as f32 / 255.0).powf(GAMMA);
    let linear = 0.2126 * luma(red) + 0.7152 * luma(green) + 0.0722 * luma(blue);
    let luminance = (linear.powf(1.0 / GAMMA) * 255.0).round() as u32;
    (luminance.min(255) >> 5) as usize
}

/// Whether text views use the atlas. `PAM_DESKTOP_TEXT_GAMMA=1` turns it on,
/// `=0` off; unset keeps [`DEFAULT_ENABLED`].
fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(
        || match std::env::var("PAM_DESKTOP_TEXT_GAMMA").as_deref() {
            Ok("1") => true,
            Ok("0") => false,
            _ => DEFAULT_ENABLED,
        },
    )
}

/// The atlas stays opt-in until it costs no more than GSK's own glyph cache
/// (see docs/text-rendering.md for the measurements).
const DEFAULT_ENABLED: bool = false;

/// Upper bound of all atlas pages together; least recently drawn pages are
/// dropped first (their glyphs are rasterized again if they come back).
const BUDGET_BYTES: usize = 4 * 1024 * 1024;
/// Glyphs wider or taller than this (device pixels) stay GSK text nodes.
const MAX_GLYPH: i32 = 256;
/// Widest page row (device pixels).
const PAGE_WIDTH: usize = 512;

const GLYPH_EMPTY: u32 = 0x0FFF_FFFF;
const GLYPH_UNKNOWN_FLAG: u32 = 0x1000_0000;

/// Cairo's image compositor places a glyph at `floor(x + 1/8)` with a
/// quarter-pixel phase `floor(4 (x + 1/8)) - 4 floor(x + 1/8)`; the atlas
/// buckets device positions the same way so its glyphs land where cairo's
/// would.
fn position(device: f64) -> (i32, u8) {
    let shifted = device + 0.125;
    let whole = shifted.floor();
    let phase = ((shifted * 4.0).floor() - whole * 4.0) as i64;
    (whole as i32, phase.clamp(0, 3) as u8)
}

/// What a cached glyph was drawn for.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct Key {
    /// Atlas-local id of the Pango font (face, size, options).
    font: u32,
    glyph: u32,
    scale: u8,
    /// Quarter-pixel phases: x in bits 0-1, y in bits 2-3.
    phase: u8,
    /// Text colour, RGBA 8-bit: it picks the correcting table (luminance
    /// level) and tints the mask.
    color: u32,
}

impl Key {
    fn new(font: u32, glyph: u32, scale: u8, phase: (u8, u8), color: [u8; 4]) -> Self {
        Key {
            font,
            glyph,
            scale,
            phase: phase.0 | (phase.1 << 2),
            color: u32::from_be_bytes(color),
        }
    }

    fn phase(&self) -> (u8, u8) {
        (self.phase & 3, self.phase >> 2)
    }
}

/// FxHash: keys are small integers hashed once per drawn glyph.
#[derive(Default)]
struct Fx(u64);

impl Hasher for Fx {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.write_u64(u64::from(*byte));
        }
    }

    fn write_u8(&mut self, value: u8) {
        self.write_u64(u64::from(value));
    }

    fn write_u32(&mut self, value: u32) {
        self.write_u64(u64::from(value));
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(5) ^ value).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }
}

type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<Fx>>;

/// One page of glyphs: the batch rasterized for one paragraph draw.
struct Page {
    stamp: Cell<u64>,
    bytes: usize,
    keys: Vec<Key>,
    /// The page as a cairo image, for GTK's Cairo renderer (GPU renderers
    /// get a texture per glyph instead, which they pack into their own
    /// atlas).
    surface: Option<cairo::ImageSurface>,
}

/// A glyph of the atlas: its texture (a view of its page) or its place in
/// the page's cairo image, and where its top-left corner sits from the
/// glyph origin, in device pixels.
#[derive(Clone)]
struct Glyph {
    texture: Option<gdk::Texture>,
    page_x: i32,
    page_y: i32,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    page: Rc<Page>,
}

/// Glyphs that never become textures, whatever the colour or phase.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Special {
    /// No ink (spaces).
    Blank,
    /// Colour, unknown or huge glyph: drawn by GSK.
    Native,
}

#[derive(Default)]
struct Atlas {
    clock: u64,
    glyphs: FxMap<Key, Glyph>,
    special: FxMap<(u32, u32, u8), Special>,
    pages: Vec<Rc<Page>>,
    bytes: usize,
    /// Fonts by pointer, kept alive so a pointer is never reused for another.
    fonts: FxMap<usize, (pango::Font, u32)>,
    /// Whether the pages are cairo images (GTK's Cairo renderer).
    software: bool,
}

thread_local! {
    static ATLAS: RefCell<Atlas> = RefCell::default();
    static FRAME: RefCell<Option<Frame>> = const { RefCell::new(None) };
}

impl Atlas {
    fn font_id(&mut self, font: &pango::Font) -> u32 {
        let next = self.fonts.len() as u32;
        let pointer = font.as_ptr() as usize;
        self.fonts
            .entry(pointer)
            .or_insert_with(|| (font.clone(), next))
            .1
    }

    /// Looks a glyph up and marks its page as drawn now.
    fn hit(&self, key: &Key) -> Option<&Glyph> {
        let glyph = self.glyphs.get(key)?;
        glyph.page.stamp.set(self.clock);
        Some(glyph)
    }

    /// Adds a page holding `glyphs` (already views of its `bytes`).
    fn add_page(&mut self, packed: Packed) {
        let Packed {
            bytes,
            surface,
            glyphs,
        } = packed;
        let page = Rc::new(Page {
            stamp: Cell::new(self.clock),
            bytes,
            keys: glyphs.iter().map(|(key, _)| *key).collect(),
            surface,
        });
        for (key, parts) in glyphs {
            self.glyphs.insert(
                key,
                Glyph {
                    texture: parts.texture,
                    page_x: parts.page_x,
                    page_y: parts.page_y,
                    left: parts.left,
                    top: parts.top,
                    width: parts.width,
                    height: parts.height,
                    page: Rc::clone(&page),
                },
            );
        }
        self.bytes += bytes;
        self.pages.push(page);
    }

    /// Drops least recently drawn pages until the atlas fits the budget
    /// (pages drawn in the current frame stay).
    fn enforce_budget(&mut self) {
        if self.bytes <= BUDGET_BYTES {
            return;
        }
        // Down to 7/8 of the budget, so a full atlas is not trimmed at
        // every new glyph.
        let target = BUDGET_BYTES / 8 * 7;
        self.pages.sort_by_key(|page| page.stamp.get());
        let mut dropped = 0;
        for page in &self.pages {
            if self.bytes <= target || page.stamp.get() == self.clock {
                break;
            }
            for key in &page.keys {
                if self
                    .glyphs
                    .get(key)
                    .is_some_and(|glyph| Rc::ptr_eq(&glyph.page, page))
                {
                    self.glyphs.remove(key);
                }
            }
            self.bytes -= page.bytes;
            dropped += 1;
        }
        self.pages.drain(..dropped);
    }
}

/// A rasterized glyph waiting for its page.
struct Pending {
    key: Key,
    pixels: Vec<u8>,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
}

struct GlyphParts {
    texture: Option<gdk::Texture>,
    page_x: i32,
    page_y: i32,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
}

enum Op {
    /// A glyph whose origin is at this device pixel.
    Glyph(Key, i32, i32),
    /// A background, underline or strikethrough (snapshot coordinates).
    Rect(gdk::RGBA, graphene::Rect),
    /// A run left to GSK.
    Node(gsk::RenderNode),
}

/// The paragraph being drawn: its ops and the glyphs it rasterized.
struct Frame {
    /// Layout origin in window logical pixels.
    origin: (f64, f64),
    /// Exact minus snapped top of the first line (see `TextState::line_phase`).
    phase: f64,
    /// Pango y of the first baseline drawn (the first line's).
    first_baseline: Option<i32>,
    scale: f64,
    color: gdk::RGBA,
    ops: Vec<Op>,
    pending: Vec<Pending>,
    pending_index: FxMap<Key, usize>,
}

fn rgba8(color: &gdk::RGBA) -> [u8; 4] {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    [
        channel(color.red()),
        channel(color.green()),
        channel(color.blue()),
        channel(color.alpha()),
    ]
}

/// The colour Pango gives a part of the current run, like GSK's renderer.
unsafe fn part_color(
    renderer: *mut pango_ffi::PangoRenderer,
    part: pango_ffi::PangoRenderPart,
    default: &gdk::RGBA,
) -> gdk::RGBA {
    // SAFETY: called from a renderer callback with that renderer.
    let (color, alpha) = unsafe {
        (
            pango_ffi::pango_renderer_get_color(renderer, part),
            pango_ffi::pango_renderer_get_alpha(renderer, part),
        )
    };
    // SAFETY: Pango returns null or a colour of the renderer.
    match unsafe { color.as_ref() } {
        Some(color) => gdk::RGBA::new(
            f32::from(color.red) / 65535.0,
            f32::from(color.green) / 65535.0,
            f32::from(color.blue) / 65535.0,
            if alpha != 0 {
                f32::from(alpha) / 65535.0
            } else {
                default.alpha()
            },
        ),
        None if alpha != 0 => default.with_alpha(f32::from(alpha) / 65535.0),
        None => *default,
    }
}

/// Rasterizes one glyph with black ink at `phase`: the coverage, or `None`
/// when the glyph has no ink, or `Err` when it must stay a GSK glyph.
#[allow(clippy::type_complexity)]
fn rasterize(
    font: &pango::Font,
    scaled: *mut cairo::ffi::cairo_scaled_font_t,
    glyph: u32,
    scale: f64,
    phase: (u8, u8),
) -> Result<Option<(Vec<u8>, i32, i32, i32, i32)>, ()> {
    let (ink, _) = font.glyph_extents(glyph);
    if ink.width() <= 0 || ink.height() <= 0 {
        return Ok(None);
    }
    let units = f64::from(pango::SCALE);
    let device = |value: i32| f64::from(value) / units * scale;
    // Two pixels of margin for hinting, the phase and antialiasing.
    let left = device(ink.x()).floor() as i32 - 2;
    let top = device(ink.y()).floor() as i32 - 2;
    let right = device(ink.x() + ink.width()).ceil() as i32 + 3;
    let bottom = device(ink.y() + ink.height()).ceil() as i32 + 3;
    let (width, height) = (right - left, bottom - top);
    if width > MAX_GLYPH || height > MAX_GLYPH {
        return Err(());
    }
    let mut surface =
        cairo::ImageSurface::create(cairo::Format::ARgb32, width, height).map_err(|_| ())?;
    {
        let context = cairo::Context::new(&surface).map_err(|_| ())?;
        context.scale(scale, scale);
        // SAFETY: the scaled font belongs to `font`, alive for this call.
        let scaled = unsafe { cairo::ScaledFont::from_raw_none(scaled) };
        context.set_scaled_font(&scaled);
        context.set_source_rgb(0.0, 0.0, 0.0);
        let x = f64::from(-left) + f64::from(phase.0) / 4.0;
        let y = f64::from(-top) + f64::from(phase.1) / 4.0;
        context
            .show_glyphs(&[cairo::Glyph::new(glyph.into(), x / scale, y / scale)])
            .map_err(|_| ())?;
    }
    surface.flush();
    let stride = surface.stride() as usize;
    let data = surface.data().map_err(|_| ())?;
    let row = width as usize * 4;
    let mut coverage = Vec::with_capacity(width as usize * height as usize);
    for line in data.chunks(stride).take(height as usize) {
        for pixel in line[..row].chunks_exact(4) {
            // Black ink leaves the colour channels empty: anything else is a
            // colour glyph (or subpixel antialiasing), drawn by GSK.
            if pixel[0] | pixel[1] | pixel[2] != 0 {
                return Err(());
            }
            coverage.push(pixel[3]);
        }
    }
    Ok(Some((coverage, left, top, width, height)))
}

/// Turns coverage into premultiplied BGRA of `color`, through Skia's
/// correcting table for the colour's luminance.
fn tint(coverage: &[u8], color: [u8; 4]) -> Vec<u8> {
    let table = &tables()[level(
        u32::from(color[0]),
        u32::from(color[1]),
        u32::from(color[2]),
    )];
    let alpha = u32::from(color[3]);
    let mut pixels = Vec::with_capacity(coverage.len() * 4);
    for &value in coverage {
        let corrected = (u32::from(table[usize::from(value)]) * alpha + 127) / 255;
        let premultiply = |channel: u8| ((u32::from(channel) * corrected + 127) / 255) as u8;
        pixels.extend_from_slice(&[
            premultiply(color[2]),
            premultiply(color[1]),
            premultiply(color[0]),
            corrected as u8,
        ]);
    }
    pixels
}

impl Frame {
    /// One run of glyphs at pango position (`x`, `y`) of its baseline.
    fn glyphs(
        &mut self,
        atlas: &mut Atlas,
        font: &pango::Font,
        string: &pango_ffi::PangoGlyphString,
        (x, y): (i32, i32),
        color: gdk::RGBA,
    ) -> bool {
        let color8 = rgba8(&color);
        if color8[3] == 0 {
            return true;
        }
        // SAFETY: the font comes from a Pango cairo font map.
        let scaled = unsafe { pango_cairo_font_get_scaled_font(font.as_ptr()) };
        if scaled.is_null() {
            return false;
        }
        let font_id = atlas.font_id(font);
        let scale = self.scale;
        let scale8 = scale as u8;
        let units = f64::from(pango::SCALE);
        // Each baseline is rounded to the device pixel grid, as Skia does
        // for unhinted horizontal positions. The origin puts the first
        // baseline on its pixel; a later line moves by its distance from the
        // first rounded from the exact (unsnapped) top, as Blink snaps each
        // line box: lines 19.5px apart from a top at .6 are 19px apart.
        let first = *self.first_baseline.get_or_insert(y);
        let distance = f64::from(y - first) / units;
        let shift = (self.phase + distance + 0.5).floor();
        let baseline = ((self.origin.1 + f64::from(first) / units + shift) * scale).round();
        let infos: &[pango_ffi::PangoGlyphInfo] = if string.num_glyphs > 0 {
            // SAFETY: Pango's glyph array has `num_glyphs` entries.
            unsafe { std::slice::from_raw_parts(string.glyphs, string.num_glyphs as usize) }
        } else {
            &[]
        };
        let start = self.ops.len();
        let mut advance = x;
        for info in infos {
            let glyph = info.glyph;
            let geometry = info.geometry;
            let glyph_x = (self.origin.0 + f64::from(advance + geometry.x_offset) / units) * scale;
            let glyph_y = baseline + f64::from(geometry.y_offset) / units * scale;
            advance += geometry.width;
            if glyph == GLYPH_EMPTY || glyph == u32::MAX {
                continue;
            }
            if glyph & GLYPH_UNKNOWN_FLAG != 0 {
                self.ops.truncate(start);
                return false;
            }
            let (pixel_x, phase_x) = position(glyph_x);
            let (pixel_y, phase_y) = position(glyph_y);
            let key = Key::new(font_id, glyph, scale8, (phase_x, phase_y), color8);
            if atlas.hit(&key).is_some() || self.pending_index.contains_key(&key) {
                self.ops.push(Op::Glyph(key, pixel_x, pixel_y));
                continue;
            }
            match atlas.special.get(&(font_id, glyph, scale8)) {
                Some(Special::Blank) => continue,
                Some(Special::Native) => {
                    self.ops.truncate(start);
                    return false;
                }
                None => {}
            }
            match rasterize(font, scaled, glyph, scale, key.phase()) {
                Ok(Some((coverage, left, top, width, height))) => {
                    self.pending_index.insert(key, self.pending.len());
                    self.pending.push(Pending {
                        key,
                        pixels: tint(&coverage, color8),
                        left,
                        top,
                        width,
                        height,
                    });
                    self.ops.push(Op::Glyph(key, pixel_x, pixel_y));
                }
                Ok(None) => {
                    atlas
                        .special
                        .insert((font_id, glyph, scale8), Special::Blank);
                }
                Err(()) => {
                    atlas
                        .special
                        .insert((font_id, glyph, scale8), Special::Native);
                    self.ops.truncate(start);
                    return false;
                }
            }
        }
        true
    }
}

/// A frame's new glyphs packed into one page.
struct Packed {
    bytes: usize,
    surface: Option<cairo::ImageSurface>,
    glyphs: Vec<(Key, GlyphParts)>,
}

/// Packs the frame's new glyphs into one page (rows of up to
/// [`PAGE_WIDTH`] pixels): a cairo image for the Cairo renderer, else a
/// texture per glyph viewing the page's bytes.
fn pack(pending: Vec<Pending>, software: bool) -> Option<Packed> {
    if pending.is_empty() {
        return None;
    }
    let widest = pending.iter().map(|glyph| glyph.width as usize).max()?;
    let total: usize = pending.iter().map(|glyph| glyph.width as usize).sum();
    let page_width = total.min(PAGE_WIDTH).max(widest);
    let mut places = Vec::with_capacity(pending.len());
    let (mut x, mut y, mut shelf) = (0usize, 0usize, 0usize);
    for glyph in &pending {
        let (width, height) = (glyph.width as usize, glyph.height as usize);
        if x + width > page_width {
            y += shelf;
            x = 0;
            shelf = 0;
        }
        places.push((x, y));
        x += width;
        shelf = shelf.max(height);
    }
    let page_height = y + shelf;
    let stride = page_width * 4;
    let mut data = vec![0u8; stride * page_height];
    for (glyph, &(x, y)) in pending.iter().zip(&places) {
        let row = glyph.width as usize * 4;
        for (line, source) in glyph.pixels.chunks_exact(row).enumerate() {
            let offset = (y + line) * stride + x * 4;
            data[offset..offset + row].copy_from_slice(source);
        }
    }
    let bytes = data.len();
    let parts = |glyph: &Pending, (x, y): (usize, usize), texture| GlyphParts {
        texture,
        page_x: x as i32,
        page_y: y as i32,
        left: glyph.left,
        top: glyph.top,
        width: glyph.width,
        height: glyph.height,
    };
    if software {
        // Premultiplied BGRA is cairo's ARGB32 on little-endian machines.
        let surface = cairo::ImageSurface::create_for_data(
            data,
            cairo::Format::ARgb32,
            page_width as i32,
            page_height as i32,
            stride as i32,
        )
        .ok()?;
        let glyphs = pending
            .iter()
            .zip(places)
            .map(|(glyph, place)| (glyph.key, parts(glyph, place, None)))
            .collect();
        return Some(Packed {
            bytes,
            surface: Some(surface),
            glyphs,
        });
    }
    let page = glib::Bytes::from_owned(data);
    let glyphs = pending
        .iter()
        .zip(places)
        .map(|(glyph, (x, y))| {
            let start = y * stride + x * 4;
            let end = start + (glyph.height as usize - 1) * stride + glyph.width as usize * 4;
            let view = glib::Bytes::from_bytes(&page, start..end);
            let texture = gdk::MemoryTexture::new(
                glyph.width,
                glyph.height,
                gdk::MemoryFormat::B8g8r8a8Premultiplied,
                &view,
                stride,
            );
            (glyph.key, parts(glyph, (x, y), Some(texture.upcast())))
        })
        .collect();
    Some(Packed {
        bytes,
        surface: None,
        glyphs,
    })
}

unsafe extern "C" fn draw_glyphs(
    renderer: *mut pango_ffi::PangoRenderer,
    font: *mut pango_ffi::PangoFont,
    glyphs: *mut pango_ffi::PangoGlyphString,
    x: i32,
    y: i32,
) {
    FRAME.with(|frame| {
        let mut frame = frame.borrow_mut();
        let Some(frame) = frame.as_mut() else {
            return;
        };
        // SAFETY: Pango passes a live renderer, font and glyph string.
        let color = unsafe {
            part_color(
                renderer,
                pango_ffi::PANGO_RENDER_PART_FOREGROUND,
                &frame.color,
            )
        };
        let font_ref = unsafe { pango::Font::from_glib_none(font) };
        let string = unsafe { &*glyphs };
        let drawn = ATLAS
            .with(|atlas| frame.glyphs(&mut atlas.borrow_mut(), &font_ref, string, (x, y), color));
        if !drawn {
            // SAFETY: copied out of Pango's live glyph string.
            let copy = unsafe { pango::GlyphString::from_glib_none(glyphs) };
            let units = pango::SCALE as f32;
            let offset = graphene::Point::new(x as f32 / units, y as f32 / units);
            if let Some(node) = gsk::TextNode::new(&font_ref, &copy, &color, &offset) {
                frame.ops.push(Op::Node(node.upcast()));
            }
        }
    });
}

unsafe extern "C" fn draw_rectangle(
    renderer: *mut pango_ffi::PangoRenderer,
    part: pango_ffi::PangoRenderPart,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    FRAME.with(|frame| {
        let mut frame = frame.borrow_mut();
        let Some(frame) = frame.as_mut() else {
            return;
        };
        // SAFETY: Pango passes a live renderer.
        let color = unsafe { part_color(renderer, part, &frame.color) };
        let units = pango::SCALE as f32;
        frame.ops.push(Op::Rect(
            color,
            graphene::Rect::new(
                x as f32 / units,
                y as f32 / units,
                width as f32 / units,
                height as f32 / units,
            ),
        ));
    });
}

/// Error underlines: their bounding box (layouts here never use them).
#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn draw_trapezoid(
    renderer: *mut pango_ffi::PangoRenderer,
    part: pango_ffi::PangoRenderPart,
    y1: f64,
    x11: f64,
    x21: f64,
    y2: f64,
    x12: f64,
    x22: f64,
) {
    let units = f64::from(pango::SCALE);
    let left = x11.min(x12) * units;
    let right = x21.max(x22) * units;
    // SAFETY: forwarded from Pango's own callback.
    unsafe {
        draw_rectangle(
            renderer,
            part,
            left as i32,
            (y1 * units) as i32,
            (right - left) as i32,
            ((y2 - y1) * units) as i32,
        );
    }
}

unsafe extern "C" fn class_init(class: glib::ffi::gpointer, _data: glib::ffi::gpointer) {
    // SAFETY: GObject passes the class struct registered below.
    let class = unsafe { &mut *class.cast::<pango_ffi::PangoRendererClass>() };
    class.draw_glyphs = Some(draw_glyphs);
    class.draw_rectangle = Some(draw_rectangle);
    class.draw_trapezoid = Some(draw_trapezoid);
}

/// The atlas's Pango renderer (one per thread: GTK draws on the main one).
fn renderer() -> Option<pango::Renderer> {
    thread_local! {
        static RENDERER: Option<pango::Renderer> = {
            static TYPE: OnceLock<glib::ffi::GType> = OnceLock::new();
            let type_ = *TYPE.get_or_init(|| unsafe {
                gobject_ffi::g_type_register_static_simple(
                    pango_ffi::pango_renderer_get_type(),
                    c"PamAtlasRenderer".as_ptr(),
                    std::mem::size_of::<pango_ffi::PangoRendererClass>() as u32,
                    Some(class_init),
                    std::mem::size_of::<pango_ffi::PangoRenderer>() as u32,
                    None,
                    0,
                )
            });
            (type_ != 0).then(|| {
                // SAFETY: a registered, instantiable PangoRenderer subclass.
                let type_: glib::Type = unsafe { glib::translate::from_glib(type_) };
                glib::Object::with_type(type_).downcast::<pango::Renderer>().ok()
            }).flatten()
        };
    }
    RENDERER.with(Clone::clone)
}

/// Draws `layout` at the snapshot origin, which sits at `origin` (logical
/// pixels) in the window, from the atlas. False when the atlas is off.
fn draw(
    snapshot: &gtk::Snapshot,
    layout: &pango::Layout,
    color: &gdk::RGBA,
    origin: (f64, f64),
    phase: f64,
    scale: i32,
    software: bool,
) -> bool {
    let Some(renderer) = renderer() else {
        return false;
    };
    let scale = f64::from(scale.clamp(1, 8));
    ATLAS.with(|atlas| {
        let mut atlas = atlas.borrow_mut();
        if atlas.software != software {
            // The renderer changed: pages of the other kind are useless.
            let clock = atlas.clock;
            *atlas = Atlas {
                clock,
                software,
                ..Atlas::default()
            };
        }
        atlas.clock += 1;
    });
    FRAME.with(|frame| {
        *frame.borrow_mut() = Some(Frame {
            origin,
            phase,
            first_baseline: None,
            scale,
            color: *color,
            ops: Vec::new(),
            pending: Vec::new(),
            pending_index: FxMap::default(),
        });
    });
    renderer.draw_layout(layout, 0, 0);
    let Some(frame) = FRAME.with(|frame| frame.borrow_mut().take()) else {
        return true;
    };
    ATLAS.with(|atlas| {
        let mut atlas = atlas.borrow_mut();
        if let Some(packed) = pack(frame.pending, software) {
            atlas.add_page(packed);
        }
        let ops = frame.ops;
        let mut index = 0;
        while index < ops.len() {
            match &ops[index] {
                Op::Glyph(..) if software => {
                    let run = ops[index..]
                        .iter()
                        .take_while(|op| matches!(op, Op::Glyph(..)))
                        .count();
                    paint_glyphs(snapshot, &atlas, &ops[index..index + run], origin, scale);
                    index += run;
                    continue;
                }
                Op::Glyph(key, x, y) => {
                    if let Some(glyph) = atlas.glyphs.get(key)
                        && let Some(texture) = &glyph.texture
                    {
                        snapshot
                            .append_texture(texture, &glyph_rect(glyph, (*x, *y), origin, scale));
                    }
                }
                Op::Rect(color, rect) => snapshot.append_color(color, rect),
                Op::Node(node) => snapshot.append_node(node),
            }
            index += 1;
        }
        atlas.enforce_budget();
    });
    true
}

/// Where a glyph whose origin is at device pixel `at` sits in the snapshot.
fn glyph_rect(glyph: &Glyph, at: (i32, i32), origin: (f64, f64), scale: f64) -> graphene::Rect {
    graphene::Rect::new(
        (f64::from(at.0 + glyph.left) / scale - origin.0) as f32,
        (f64::from(at.1 + glyph.top) / scale - origin.1) as f32,
        (f64::from(glyph.width) / scale) as f32,
        (f64::from(glyph.height) / scale) as f32,
    )
}

/// A run of glyphs for the Cairo renderer: one cairo node copying them out
/// of their page images (the recording keeps references, not copies).
fn paint_glyphs(
    snapshot: &gtk::Snapshot,
    atlas: &Atlas,
    ops: &[Op],
    origin: (f64, f64),
    scale: f64,
) {
    let glyphs: Vec<(&Glyph, (i32, i32))> = ops
        .iter()
        .filter_map(|op| match op {
            Op::Glyph(key, x, y) => atlas.glyphs.get(key).map(|glyph| (glyph, (*x, *y))),
            _ => None,
        })
        .collect();
    let Some(bounds) = glyphs
        .iter()
        .map(|(glyph, at)| glyph_rect(glyph, *at, origin, scale))
        .reduce(|union, rect| union.union(&rect))
    else {
        return;
    };
    let context = snapshot.append_cairo(&bounds);
    context.scale(1.0 / scale, 1.0 / scale);
    for (glyph, (x, y)) in glyphs {
        let Some(surface) = &glyph.page.surface else {
            continue;
        };
        // Whole device pixels: the copy is never resampled.
        let left = f64::from(x + glyph.left) - origin.0 * scale;
        let top = f64::from(y + glyph.top) - origin.1 * scale;
        // A pixel-aligned clip and a paint record far less than a filled
        // path (no path buffer per glyph).
        context.save().ok();
        context.rectangle(left, top, f64::from(glyph.width), f64::from(glyph.height));
        context.clip();
        if context
            .set_source_surface(
                surface,
                left - f64::from(glyph.page_x),
                top - f64::from(glyph.page_y),
            )
            .is_ok()
        {
            let _ = context.paint();
        }
        context.restore().ok();
    }
}

/// Releases every page once no window of the process is mapped (nothing
/// is kept for text nobody sees).
fn release_if_hidden() {
    let mapped = gtk::Window::list_toplevels()
        .iter()
        .any(|window| window.is_mapped());
    if !mapped {
        release();
    }
}

/// Drops the whole atlas.
fn release() {
    ATLAS.with(|atlas| {
        let mut atlas = atlas.borrow_mut();
        let clock = atlas.clock;
        *atlas = Atlas {
            clock,
            ..Atlas::default()
        };
    });
}

/// A text view's link to the shared atlas.
#[derive(Default)]
pub struct Raster {
    _shared: (),
}

impl Raster {
    /// Appends `layout` drawn in `color` at the snapshot's origin, which sits
    /// at `origin` (logical pixels) in the window. Returns false when the
    /// atlas is off; the caller then appends the layout.
    #[allow(clippy::too_many_arguments)]
    pub fn append(
        &mut self,
        widget: &gtk::Widget,
        snapshot: &gtk::Snapshot,
        layout: &pango::Layout,
        color: &gdk::RGBA,
        origin: (f64, f64),
        phase: f64,
        scale: i32,
    ) -> bool {
        enabled()
            && draw(
                snapshot,
                layout,
                color,
                origin,
                phase,
                scale,
                crate::raster::software_rendered(widget),
            )
    }

    /// The view left the screen: the atlas goes when no window is mapped.
    pub fn clear(&mut self) {
        if enabled() {
            release_if_hidden();
        }
    }
}

/// GTK-level checks of the atlas, run from the crate's single GTK test.
#[cfg(test)]
pub fn assert_atlas_draws_and_releases() {
    let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let layout = widget.create_pango_layout(Some("Atlas text, atlas text"));
    let black = gdk::RGBA::new(0.1, 0.1, 0.1, 1.0);
    let snapshot = gtk::Snapshot::new();
    assert!(draw(
        &snapshot,
        &layout,
        &black,
        (10.3, 20.0),
        0.0,
        1,
        false
    ));
    let node = snapshot.to_node().expect("a text node");
    let (glyphs, pages) = ATLAS.with(|atlas| {
        let atlas = atlas.borrow();
        (atlas.glyphs.len(), atlas.pages.len())
    });
    assert!(glyphs > 0, "glyphs rasterized");
    assert_eq!(pages, 1, "one page per batch of new glyphs");
    let mut textures = 0;
    let mut stack = vec![node];
    while let Some(node) = stack.pop() {
        match node.node_type() {
            gsk::RenderNodeType::TextureNode => textures += 1,
            gsk::RenderNodeType::ContainerNode => {
                let container = node.downcast::<gsk::ContainerNode>().unwrap();
                stack.extend((0..container.n_children()).map(|index| container.child(index)));
            }
            _ => {}
        }
    }
    // "Atlas text, atlas text": repeated letters share their glyphs.
    assert!(
        textures >= 18,
        "one texture node per inked glyph: {textures}"
    );
    assert!(glyphs < textures);
    // The same paragraph again: every glyph is a hit, no new page.
    let snapshot = gtk::Snapshot::new();
    draw(&snapshot, &layout, &black, (10.3, 20.0), 0.0, 1, false);
    assert_eq!(ATLAS.with(|atlas| atlas.borrow().pages.len()), 1);
    // A selection (background + white text) and an underline still draw.
    let attributes = pango::AttrList::new();
    attributes.insert(pango::AttrInt::new_underline(pango::Underline::Single));
    let mut background: pango::Attribute =
        pango::AttrColor::new_background(0x3535, 0x8484, 0xE4E4).into();
    background.set_start_index(0);
    background.set_end_index(5);
    attributes.insert(background);
    layout.set_attributes(Some(&attributes));
    let snapshot = gtk::Snapshot::new();
    draw(&snapshot, &layout, &black, (10.0, 20.0), 0.0, 1, false);
    let node = snapshot.to_node().expect("decorated text");
    let mut colors = 0;
    let mut stack = vec![node];
    while let Some(node) = stack.pop() {
        match node.node_type() {
            gsk::RenderNodeType::ColorNode => colors += 1,
            gsk::RenderNodeType::ContainerNode => {
                let container = node.downcast::<gsk::ContainerNode>().unwrap();
                stack.extend((0..container.n_children()).map(|index| container.child(index)));
            }
            _ => {}
        }
    }
    assert!(colors >= 2, "background and underline rectangles: {colors}");
    // GTK's Cairo renderer: the atlas switches to page images and a run of
    // glyphs is one cairo node.
    let snapshot = gtk::Snapshot::new();
    draw(&snapshot, &layout, &black, (10.0, 20.0), 0.0, 1, true);
    let node = snapshot.to_node().expect("software text");
    let mut cairo_nodes = 0;
    let mut stack = vec![node];
    while let Some(node) = stack.pop() {
        match node.node_type() {
            gsk::RenderNodeType::CairoNode => cairo_nodes += 1,
            gsk::RenderNodeType::TextureNode => panic!("no textures for the Cairo renderer"),
            gsk::RenderNodeType::ContainerNode => {
                let container = node.downcast::<gsk::ContainerNode>().unwrap();
                stack.extend((0..container.n_children()).map(|index| container.child(index)));
            }
            _ => {}
        }
    }
    assert!(cairo_nodes >= 1);
    ATLAS.with(|atlas| {
        let atlas = atlas.borrow();
        assert!(atlas.software && atlas.pages.iter().all(|page| page.surface.is_some()));
    });
    // No window is mapped here: hiding releases everything.
    release_if_hidden();
    ATLAS.with(|atlas| {
        let atlas = atlas.borrow();
        assert!(atlas.glyphs.is_empty() && atlas.pages.is_empty() && atlas.fonts.is_empty());
        assert_eq!(atlas.bytes, 0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_keep_the_ends_and_thicken_light_text() {
        let tables = tables();
        for table in tables {
            assert_eq!(table[0], 0);
            assert_eq!(table[255], 255);
        }
        // Light text on dark: coverage goes up (heavier strokes).
        assert!(tables[LEVELS - 1][128] > 140);
        // Dark text on light: coverage goes down a little.
        assert!(tables[0][128] < 128);
    }

    #[test]
    fn levels_follow_luminance() {
        assert_eq!(level(0, 0, 0), 0);
        assert_eq!(level(255, 255, 255), LEVELS - 1);
        assert_eq!(level(231, 237, 245), LEVELS - 1);
    }

    #[test]
    fn positions_bucket_to_cairo_quarter_pixels() {
        assert_eq!(position(10.0), (10, 0));
        assert_eq!(position(10.25), (10, 1));
        assert_eq!(position(10.5), (10, 2));
        assert_eq!(position(10.75), (10, 3));
        // Nearest quarter, carrying into the next pixel.
        assert_eq!(position(10.124), (10, 0));
        assert_eq!(position(10.13), (10, 1));
        assert_eq!(position(10.9), (11, 0));
        assert_eq!(position(-0.3), (-1, 3));
    }

    #[test]
    fn keys_separate_phase_colour_and_scale() {
        let black = [0, 0, 0, 255];
        let key = Key::new(1, 42, 1, (3, 0), black);
        assert_eq!(key.phase(), (3, 0));
        assert_eq!(key, Key::new(1, 42, 1, (3, 0), black));
        assert_ne!(key, Key::new(1, 42, 1, (2, 0), black));
        assert_ne!(key, Key::new(1, 42, 1, (3, 1), black));
        assert_ne!(key, Key::new(1, 42, 2, (3, 0), black));
        assert_ne!(key, Key::new(1, 42, 1, (3, 0), [255, 255, 255, 255]));
        assert_ne!(key, Key::new(2, 42, 1, (3, 0), black));
    }

    #[test]
    fn tint_corrects_coverage_for_the_colour() {
        let pixels = tint(&[0, 128, 255], [255, 255, 255, 255]);
        assert_eq!(&pixels[..4], &[0, 0, 0, 0]);
        assert!(pixels[7] > 140, "light text gains weight");
        assert_eq!(&pixels[8..], &[255, 255, 255, 255]);
        let dark = tint(&[128], [0, 0, 0, 255]);
        assert!(dark[3] < 128, "dark text loses a little");
        assert_eq!(&dark[..3], &[0, 0, 0]);
        let faded = tint(&[255], [255, 0, 0, 128]);
        assert_eq!(faded, vec![0, 0, 128, 128]);
    }

    fn page_of(atlas: &mut Atlas, first: u32, count: u32) {
        let pending = (first..first + count)
            .map(|glyph| Pending {
                key: Key::new(0, glyph, 1, (0, 0), [0, 0, 0, 255]),
                pixels: vec![0; 64 * 64 * 4],
                left: 0,
                top: -64,
                width: 64,
                height: 64,
            })
            .collect();
        let packed = pack(pending, atlas.software).unwrap();
        atlas.add_page(packed);
    }

    #[test]
    fn pages_pack_glyphs_as_views() {
        let mut atlas = Atlas::default();
        page_of(&mut atlas, 0, 10);
        // 8 glyphs of 64 px per 512 px row, two rows.
        assert_eq!(atlas.bytes, 512 * 128 * 4);
        let glyph = atlas.glyphs.values().next().unwrap();
        let texture = glyph.texture.as_ref().unwrap();
        assert_eq!((texture.width(), texture.height()), (64, 64));
        assert_eq!(atlas.pages.len(), 1);
        assert!(atlas.pages[0].surface.is_none());
        // The Cairo renderer's pages are one image, glyphs are places in it.
        let mut software = Atlas {
            software: true,
            ..Atlas::default()
        };
        page_of(&mut software, 0, 10);
        let surface = software.pages[0].surface.as_ref().unwrap();
        assert_eq!((surface.width(), surface.height()), (512, 128));
        let ninth = &software.glyphs[&Key::new(0, 8, 1, (0, 0), [0, 0, 0, 255])];
        assert!(ninth.texture.is_none());
        assert_eq!((ninth.page_x, ninth.page_y), (0, 64));
    }

    #[test]
    fn least_recently_drawn_pages_go_over_the_budget() {
        let mut atlas = Atlas::default();
        // 16 KiB a glyph, 64 glyphs (1 MiB) a page.
        for page in 0..8 {
            atlas.clock += 1;
            page_of(&mut atlas, page * 64, 64);
            // The second page is drawn again with every later frame.
            if page >= 1 {
                assert!(
                    atlas
                        .hit(&Key::new(0, 64, 1, (0, 0), [0, 0, 0, 255]))
                        .is_some()
                );
            }
            atlas.enforce_budget();
            assert!(atlas.bytes <= BUDGET_BYTES, "bound kept: {}", atlas.bytes);
        }
        let has = |atlas: &Atlas, glyph: u32| {
            atlas
                .glyphs
                .contains_key(&Key::new(0, glyph, 1, (0, 0), [0, 0, 0, 255]))
        };
        assert!(!has(&atlas, 0), "the oldest page went first");
        assert!(has(&atlas, 64), "a page in use stays");
        assert!(has(&atlas, 7 * 64), "the newest page stays");
        assert_eq!(
            atlas.bytes,
            atlas.pages.iter().map(|page| page.bytes).sum::<usize>()
        );
        assert_eq!(
            atlas.glyphs.len(),
            atlas
                .pages
                .iter()
                .map(|page| page.keys.len())
                .sum::<usize>()
        );
    }

    #[test]
    fn release_drops_every_page() {
        ATLAS.with(|atlas| page_of(&mut atlas.borrow_mut(), 0, 4));
        release();
        ATLAS.with(|atlas| {
            let atlas = atlas.borrow();
            assert!(atlas.glyphs.is_empty() && atlas.pages.is_empty());
            assert_eq!(atlas.bytes, 0);
        });
    }
}
