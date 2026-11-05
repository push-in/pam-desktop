//! DirectWrite paragraph construction shared by measurement and drawing, so
//! a box is always sized by exactly the layout that is later painted.

use std::cell::RefCell;
use std::collections::HashMap;

use pam_desktop_ui::backend::VerticalAlign;
use pam_desktop_ui::color::Color;
use pam_desktop_ui::geometry::{Insets, Point, Rect, Size};
use pam_desktop_ui::text::{FontSpec, TextAlign, TextBlock, TextMetrics};
use windows::Win32::Globalization::GetUserDefaultLocaleName;
use windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_FEATURE, DWRITE_FONT_FEATURE_TAG, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_ITALIC, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT,
    DWRITE_HIT_TEST_METRICS, DWRITE_LINE_METRICS, DWRITE_LINE_SPACING_METHOD_UNIFORM,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_JUSTIFIED, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS, DWRITE_TEXT_RANGE, DWRITE_TRIMMING,
    DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_EMERGENCY_BREAK,
    DWRITE_WORD_WRAPPING_NO_WRAP, IDWriteFactory, IDWriteFontCollection, IDWriteTextFormat,
    IDWriteTextLayout, IDWriteTextLayout1,
};
use windows::Win32::UI::WindowsAndMessaging::{
    NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    SystemParametersInfoW,
};
use windows::core::{BOOL, Interface, PCWSTR};

use crate::gfx::Device;
use crate::util::{from_wide, wide};

/// Layout width used when the paragraph is unconstrained.
const UNBOUNDED: f32 = 1_000_000.0;
/// Entries kept in the measurement cache before it is reset.
const METRICS_CACHE_LIMIT: usize = 4096;

/// A laid out paragraph.
pub struct Layout {
    pub layout: IDWriteTextLayout,
    /// UTF-16 range of every run of the block, in order.
    pub ranges: Vec<DWRITE_TEXT_RANGE>,
    /// Height of the visible lines.
    pub height: f32,
    pub width: f32,
    /// Width of the widest line, which may exceed the layout box.
    pub natural_width: f32,
    pub first_baseline: f32,
    pub line_count: u32,
    /// Lines beyond `max_lines` exist and must not be painted.
    pub truncated: bool,
    pub utf16_length: u32,
}

pub struct TextEngine {
    locale: Vec<u16>,
    /// The system's message font, used when a run names no family.
    ui_font: String,
    metrics: HashMap<u64, TextMetrics>,
    /// Installed fonts, to pick the first available family of a list.
    collection: Option<IDWriteFontCollection>,
    /// Authored family (or list) to the installed family it resolves to.
    families: RefCell<HashMap<String, String>>,
}

fn system_ui_font() -> String {
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
        ..Default::default()
    };
    // SAFETY: `metrics` is a properly sized NONCLIENTMETRICSW with cbSize set.
    let found = unsafe {
        SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            Some(std::ptr::from_mut(&mut metrics).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    let name = if found.is_ok() {
        from_wide(&metrics.lfMessageFont.lfFaceName)
    } else {
        String::new()
    };
    if name.is_empty() {
        String::from("Segoe UI")
    } else {
        name
    }
}

/// `"tnum", "liga" 0` (CSS `font-feature-settings`) as OpenType features.
fn parse_features(features: &str) -> Vec<DWRITE_FONT_FEATURE> {
    let mut parsed = Vec::new();
    for entry in features.split(',') {
        let entry = entry.trim();
        let (tag, rest) = match entry.strip_prefix(['"', '\'']) {
            Some(unquoted) => match unquoted.find(['"', '\'']) {
                Some(end) => (&unquoted[..end], unquoted[end + 1..].trim()),
                None => continue,
            },
            None => match entry.split_once(char::is_whitespace) {
                Some((tag, rest)) => (tag, rest.trim()),
                None => (entry, ""),
            },
        };
        let bytes = tag.as_bytes();
        if bytes.len() != 4 || !tag.is_ascii() {
            continue;
        }
        let parameter = match rest {
            "" | "on" => 1,
            "off" => 0,
            number => match number.parse::<u32>() {
                Ok(value) => value,
                Err(_) => continue,
            },
        };
        let name = u32::from(bytes[0])
            | (u32::from(bytes[1]) << 8)
            | (u32::from(bytes[2]) << 16)
            | (u32::from(bytes[3]) << 24);
        parsed.push(DWRITE_FONT_FEATURE {
            nameTag: DWRITE_FONT_FEATURE_TAG(name),
            parameter,
        });
    }
    parsed
}

fn weight(font: &FontSpec) -> DWRITE_FONT_WEIGHT {
    DWRITE_FONT_WEIGHT(i32::from(font.weight.clamp(1, 999)))
}

impl TextEngine {
    pub fn new(dwrite: &IDWriteFactory) -> Self {
        let mut collection = None;
        // SAFETY: the out pointer refers to a live local.
        unsafe {
            let _ = dwrite.GetSystemFontCollection(&mut collection, false);
        }
        let mut locale = [0_u16; 85];
        // SAFETY: the buffer is LOCALE_NAME_MAX_LENGTH units long.
        let written = unsafe { GetUserDefaultLocaleName(&mut locale) };
        let locale = if written > 0 {
            locale[..written as usize].to_vec()
        } else {
            wide("en-us")
        };
        Self {
            locale,
            ui_font: system_ui_font(),
            metrics: HashMap::new(),
            collection,
            families: RefCell::new(HashMap::new()),
        }
    }

    fn installed(&self, family: &str) -> bool {
        let Some(collection) = &self.collection else {
            return true;
        };
        let name = wide(family);
        let (mut index, mut exists) = (0_u32, BOOL(0));
        // SAFETY: the name is NUL-terminated and the out pointers refer to live locals.
        unsafe { collection.FindFamilyName(PCWSTR(name.as_ptr()), &mut index, &mut exists) }.is_ok()
            && exists.as_bool()
    }

    /// The installed family a font resolves to: the first available entry
    /// of a CSS-like list, with the generic names mapped to system fonts
    /// and the system UI font when nothing matches.
    pub fn family(&self, font: &FontSpec) -> String {
        if font.family.is_empty() {
            return self.ui_font.clone();
        }
        if let Some(known) = self.families.borrow().get(&font.family) {
            return known.clone();
        }
        let resolved = font
            .family
            .split(',')
            .map(|entry| entry.trim().trim_matches(['"', '\'']))
            .filter(|entry| !entry.is_empty())
            .find_map(|entry| match entry.to_ascii_lowercase().as_str() {
                "system-ui" | "system" | "sans-serif" | "ui-sans-serif" | "-apple-system"
                | "blinkmacsystemfont" => Some(self.ui_font.clone()),
                "serif" | "ui-serif" => Some(String::from("Times New Roman")),
                "monospace" | "ui-monospace" => Some(String::from("Consolas")),
                _ => self.installed(entry).then(|| entry.to_owned()),
            })
            .unwrap_or_else(|| self.ui_font.clone());
        self.families
            .borrow_mut()
            .insert(font.family.clone(), resolved.clone());
        resolved
    }

    fn format(
        &self,
        dwrite: &IDWriteFactory,
        font: &FontSpec,
    ) -> windows::core::Result<IDWriteTextFormat> {
        let family = wide(&self.family(font));
        let style = if font.italic {
            DWRITE_FONT_STYLE_ITALIC
        } else {
            DWRITE_FONT_STYLE_NORMAL
        };
        // SAFETY: `family` and `self.locale` are NUL-terminated and outlive the call.
        unsafe {
            dwrite.CreateTextFormat(
                PCWSTR(family.as_ptr()),
                None,
                weight(font),
                style,
                DWRITE_FONT_STRETCH_NORMAL,
                font.size.max(0.1),
                PCWSTR(self.locale.as_ptr()),
            )
        }
    }

    /// Builds the paragraph wrapped to `max_width` (infinite = one line per
    /// paragraph). Measurement and drawing both go through here.
    pub fn build(
        &self,
        dwrite: &IDWriteFactory,
        block: &TextBlock,
        max_width: f32,
    ) -> Option<Layout> {
        let mut text: Vec<u16> = Vec::new();
        let mut ranges = Vec::with_capacity(block.runs.len());
        for run in &block.runs {
            let start = text.len() as u32;
            text.extend(run.text.encode_utf16());
            ranges.push(DWRITE_TEXT_RANGE {
                startPosition: start,
                length: text.len() as u32 - start,
            });
        }
        let fallback = FontSpec::default();
        let base = block
            .runs
            .iter()
            .find(|run| !run.text.is_empty())
            .or(block.runs.first())
            .map_or(&fallback, |run| &run.font);
        let format = self.format(dwrite, base).ok()?;
        let bounded = max_width.is_finite();
        let wraps = bounded && block.max_lines != 1;
        // SAFETY: COM calls on live DirectWrite objects; every pointer passed
        // refers to a local that outlives the call.
        unsafe {
            let _ = format.SetWordWrapping(if wraps {
                DWRITE_WORD_WRAPPING_EMERGENCY_BREAK
            } else {
                DWRITE_WORD_WRAPPING_NO_WRAP
            });
            // An unbounded layout is only measured: alignment would move it far away.
            let alignment = match block.align {
                _ if !bounded => DWRITE_TEXT_ALIGNMENT_LEADING,
                TextAlign::Start => DWRITE_TEXT_ALIGNMENT_LEADING,
                TextAlign::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                TextAlign::End => DWRITE_TEXT_ALIGNMENT_TRAILING,
                TextAlign::Justify => DWRITE_TEXT_ALIGNMENT_JUSTIFIED,
            };
            let _ = format.SetTextAlignment(alignment);
            let width = if bounded {
                max_width.max(0.0)
            } else {
                UNBOUNDED
            };
            let layout = dwrite
                .CreateTextLayout(&text, &format, width, UNBOUNDED)
                .ok()?;
            let spaced = layout.cast::<IDWriteTextLayout1>().ok();
            for (run, range) in block.runs.iter().zip(&ranges) {
                if range.length == 0 {
                    continue;
                }
                if run.font != *base {
                    let family = wide(&self.family(&run.font));
                    let _ = layout.SetFontFamilyName(PCWSTR(family.as_ptr()), *range);
                    let _ = layout.SetFontSize(run.font.size.max(0.1), *range);
                    let _ = layout.SetFontWeight(weight(&run.font), *range);
                    let _ = layout.SetFontStyle(
                        if run.font.italic {
                            DWRITE_FONT_STYLE_ITALIC
                        } else {
                            DWRITE_FONT_STYLE_NORMAL
                        },
                        *range,
                    );
                }
                if !run.font.features.is_empty() {
                    let features = parse_features(&run.font.features);
                    if !features.is_empty()
                        && let Ok(typography) = dwrite.CreateTypography()
                    {
                        for feature in features {
                            let _ = typography.AddFontFeature(feature);
                        }
                        let _ = layout.SetTypography(&typography, *range);
                    }
                }
                if run.letter_spacing != 0.0
                    && let Some(spaced) = &spaced
                {
                    let _ = spaced.SetCharacterSpacing(0.0, run.letter_spacing, 0.0, *range);
                }
                if run.underline {
                    let _ = layout.SetUnderline(true, *range);
                }
                if run.strikethrough {
                    let _ = layout.SetStrikethrough(true, *range);
                }
            }

            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics).ok()?;
            // Natural line boxes: the first gives the font's own ascent, the
            // rest are needed when lines are cut.
            let count = metrics.lineCount.max(1) as usize;
            let mut few = [DWRITE_LINE_METRICS::default(); 8];
            let mut many: Vec<DWRITE_LINE_METRICS> = Vec::new();
            let lines: &mut [DWRITE_LINE_METRICS] = if count <= few.len() {
                &mut few[..count]
            } else {
                many.resize(count, DWRITE_LINE_METRICS::default());
                &mut many
            };
            let mut reported = 0_u32;
            let _ = layout.GetLineMetrics(Some(&mut *lines), &mut reported);
            let mut first_baseline = lines[0].baseline;
            if block.line_height > 0.0 {
                // The font's line box is centered in the fixed line height (CSS half-leading).
                let baseline = (block.line_height - lines[0].height) / 2.0 + lines[0].baseline;
                let _ = layout.SetLineSpacing(
                    DWRITE_LINE_SPACING_METHOD_UNIFORM,
                    block.line_height,
                    baseline,
                );
                first_baseline = baseline;
                layout.GetMetrics(&mut metrics).ok()?;
            }
            let mut height = metrics.height;
            let mut line_count = metrics.lineCount.max(1);
            let mut truncated = false;
            if block.max_lines > 0 {
                // Lines come from wrapping and from explicit line breaks:
                // both count, whatever the width.
                if metrics.lineCount > block.max_lines {
                    let visible = if block.line_height > 0.0 {
                        block.line_height * block.max_lines as f32
                    } else {
                        lines
                            .iter()
                            .take(block.max_lines as usize)
                            .map(|line| line.height)
                            .sum()
                    };
                    // Only the first `max_lines` lines fit: the last one gets the ellipsis.
                    let _ = layout.SetMaxHeight(visible + 0.01);
                    height = visible;
                    line_count = block.max_lines;
                    truncated = true;
                }
                if truncated || (bounded && !wraps) {
                    let trimming = DWRITE_TRIMMING {
                        granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                        delimiter: 0,
                        delimiterCount: 0,
                    };
                    if let Ok(sign) = dwrite.CreateEllipsisTrimmingSign(&format) {
                        let _ = layout.SetTrimming(&trimming, &sign);
                    }
                    let _ = layout.GetMetrics(&mut metrics);
                }
            }
            let natural = metrics.widthIncludingTrailingWhitespace;
            let width = if bounded {
                natural.min(max_width.max(0.0))
            } else {
                natural
            };
            Some(Layout {
                layout,
                ranges,
                height,
                width,
                natural_width: natural,
                first_baseline,
                line_count,
                truncated,
                utf16_length: text.len() as u32,
            })
        }
    }

    /// Measures a paragraph exactly as `build` lays it out for drawing.
    pub fn measure(
        &mut self,
        dwrite: &IDWriteFactory,
        block: &TextBlock,
        max_width: f32,
    ) -> TextMetrics {
        let key = block.layout_key(max_width);
        if let Some(metrics) = self.metrics.get(&key) {
            return *metrics;
        }
        let metrics = match self.build(dwrite, block, max_width) {
            // Round up: a box narrower than its text by a fraction would wrap when drawn.
            Some(layout) => TextMetrics {
                width: layout.width.ceil(),
                height: layout.height.ceil(),
                first_baseline: layout.first_baseline,
                line_count: layout.line_count,
            },
            None => TextMetrics::default(),
        };
        if self.metrics.len() >= METRICS_CACHE_LIMIT {
            self.metrics.clear();
        }
        self.metrics.insert(key, metrics);
        metrics
    }
}

impl Layout {
    /// Colors every run (runs without a color use the brush passed when
    /// drawing) and paints `selection` with the highlight text color.
    pub fn apply_colors(
        &self,
        block: &TextBlock,
        device: &Device,
        selection: Option<((u32, u32), Color)>,
    ) {
        let mut brushes: Vec<(Color, ID2D1SolidColorBrush)> = Vec::new();
        let mut brush_for = |color: Color| -> Option<ID2D1SolidColorBrush> {
            if let Some((_, brush)) = brushes.iter().find(|(known, _)| *known == color) {
                return Some(brush.clone());
            }
            // SAFETY: COM call on the live device context; the color outlives the call.
            let brush = unsafe {
                device
                    .context
                    .CreateSolidColorBrush(&crate::util::color_f(color), None)
                    .ok()?
            };
            brushes.push((color, brush.clone()));
            Some(brush)
        };
        // SAFETY: COM calls on the live layout; brushes are kept alive by the layout itself.
        unsafe {
            for (run, range) in block.runs.iter().zip(&self.ranges) {
                if range.length == 0 {
                    continue;
                }
                match run.color.and_then(&mut brush_for) {
                    Some(brush) => {
                        let _ = self.layout.SetDrawingEffect(&brush, *range);
                    }
                    None => {
                        let _ = self.layout.SetDrawingEffect(None, *range);
                    }
                }
            }
            if let Some(((start, end), color)) = selection {
                let end = end.min(self.utf16_length);
                if start < end
                    && let Some(brush) = brush_for(color)
                {
                    let _ = self.layout.SetDrawingEffect(
                        &brush,
                        DWRITE_TEXT_RANGE {
                            startPosition: start,
                            length: end - start,
                        },
                    );
                }
            }
        }
    }

    /// The UTF-16 offset of the character boundary nearest to `point`
    /// (relative to the layout's origin).
    pub fn index_at(&self, point: Point) -> u32 {
        let mut trailing = BOOL(0);
        let mut inside = BOOL(0);
        let mut metrics = DWRITE_HIT_TEST_METRICS::default();
        // SAFETY: the out pointers refer to live locals.
        let hit = unsafe {
            self.layout
                .HitTestPoint(point.x, point.y, &mut trailing, &mut inside, &mut metrics)
        };
        if hit.is_err() {
            return 0;
        }
        let index = if trailing.as_bool() {
            metrics.textPosition + metrics.length
        } else {
            metrics.textPosition
        };
        index.min(self.utf16_length)
    }

    /// Rectangles covering a UTF-16 range, relative to the layout's origin.
    pub fn range_rects(&self, start: u32, end: u32, rects: &mut Vec<Rect>) {
        rects.clear();
        let end = end.min(self.utf16_length);
        if start >= end {
            return;
        }
        let mut count = 0_u32;
        let mut metrics = [DWRITE_HIT_TEST_METRICS::default(); 16];
        // SAFETY: the buffers passed are live and their lengths are given by the slices.
        unsafe {
            let fitted = self.layout.HitTestTextRange(
                start,
                end - start,
                0.0,
                0.0,
                Some(&mut metrics),
                &mut count,
            );
            if fitted.is_ok() {
                rects.extend(
                    metrics
                        .iter()
                        .take(count as usize)
                        .map(|hit| Rect::new(hit.left, hit.top, hit.width, hit.height)),
                );
                return;
            }
            // More rectangles than the stack buffer holds: ask again with the reported count.
            let mut all = vec![DWRITE_HIT_TEST_METRICS::default(); count as usize];
            if self
                .layout
                .HitTestTextRange(start, end - start, 0.0, 0.0, Some(&mut all), &mut count)
                .is_ok()
            {
                rects.extend(
                    all.iter()
                        .take(count as usize)
                        .map(|hit| Rect::new(hit.left, hit.top, hit.width, hit.height)),
                );
            }
        }
    }
}

/// Where a paragraph of `height` starts inside a box of `size`.
pub fn text_origin(size: Size, insets: Insets, vertical: VerticalAlign, height: f32) -> Point {
    let free = size.height - insets.vertical() - height;
    let top = insets.top
        + match vertical {
            VerticalAlign::Top => 0.0,
            VerticalAlign::Center => (free / 2.0).max(0.0),
            VerticalAlign::Bottom => free.max(0.0),
        };
    Point::new(insets.left, top.round())
}
