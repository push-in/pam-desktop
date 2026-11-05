//! DirectWrite paragraph construction shared by measurement and drawing, so
//! a box is always sized by exactly the layout that is later painted.

use std::cell::RefCell;
use std::collections::HashMap;

use pam_desktop_ui::backend::VerticalAlign;
use pam_desktop_ui::color::Color;
use pam_desktop_ui::geometry::{Insets, Point, Rect, Size};
use pam_desktop_ui::text::{Ellipsize, FontSpec, TextAlign, TextBlock, TextMetrics};
use windows::Win32::Globalization::GetUserDefaultLocaleName;
use windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_CLUSTER_METRICS, DWRITE_FONT_FEATURE, DWRITE_FONT_FEATURE_TAG,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_ITALIC, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_HIT_TEST_METRICS, DWRITE_LINE_METRICS,
    DWRITE_LINE_SPACING_METHOD_UNIFORM, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_JUSTIFIED, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING,
    DWRITE_TEXT_METRICS, DWRITE_TEXT_RANGE, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_EMERGENCY_BREAK, DWRITE_WORD_WRAPPING_NO_WRAP, IDWriteFactory,
    IDWriteFontCollection, IDWriteTextFormat, IDWriteTextLayout, IDWriteTextLayout1,
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
/// The sign that stands for the text cut out of a line (U+2026).
const ELLIPSIS: u16 = 0x2026;

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
    /// UTF-16 range of the block's text that a head or middle ellipsis
    /// stands for. The layout holds the shortened text: offsets of the
    /// block go through `visible_index` before they reach it.
    pub elided: Option<(u32, u32)>,
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

/// UTF-16 offsets where a line may be cut: the cluster boundaries of the
/// shaped text, or every code point when DirectWrite does not report them.
fn cluster_stops(layout: &IDWriteTextLayout, text: &[u16]) -> Vec<u32> {
    let mut clusters = vec![DWRITE_CLUSTER_METRICS::default(); text.len()];
    let mut count = 0_u32;
    // SAFETY: the buffer is live and its length is given by the slice; a
    // text never has more clusters than UTF-16 units.
    let reported = unsafe { layout.GetClusterMetrics(Some(&mut clusters), &mut count) };
    let mut stops = Vec::with_capacity(text.len() + 1);
    stops.push(0_u32);
    if reported.is_ok() {
        let mut end = 0_u32;
        for cluster in clusters.iter().take(count as usize) {
            if cluster.length > 0 {
                end += u32::from(cluster.length);
                stops.push(end);
            }
        }
    }
    if stops.last() != Some(&(text.len() as u32)) {
        stops.truncate(1);
        // A low surrogate continues the unit before it.
        stops.extend(
            (1..text.len())
                .filter(|index| !(0xDC00..=0xDFFF).contains(&text[*index]))
                .map(|index| index as u32),
        );
        stops.push(text.len() as u32);
    }
    stops
}

/// The UTF-16 range that goes away when `removed` clusters are cut from the
/// start of the line (`head`) or from its middle.
fn cut_range(stops: &[u32], removed: usize, head: bool) -> (u32, u32) {
    let clusters = stops.len().saturating_sub(1);
    let removed = removed.min(clusters);
    // A middle cut leaves the extra cluster of an odd rest before the ellipsis.
    let first = if head {
        0
    } else {
        (clusters - removed).div_ceil(2)
    };
    (stops[first], stops[first + removed])
}

/// The text with the UTF-16 range `cut` replaced by an ellipsis, and the
/// range of every run over it. The ellipsis joins the run of the last unit
/// kept before it (of the first one after it when the cut starts the text),
/// so it is styled like its neighbor.
fn elide(
    text: &[u16],
    ranges: &[DWRITE_TEXT_RANGE],
    (from, to): (u32, u32),
) -> (Vec<u16>, Vec<DWRITE_TEXT_RANGE>) {
    let mut short = Vec::with_capacity(text.len() + 1);
    short.extend_from_slice(&text[..from as usize]);
    short.push(ELLIPSIS);
    short.extend_from_slice(&text[to as usize..]);
    let bounds =
        |range: &DWRITE_TEXT_RANGE| (range.startPosition, range.startPosition + range.length);
    let carrier = ranges
        .iter()
        .position(|range| {
            let (start, end) = bounds(range);
            if from > 0 {
                start < from && from <= end
            } else {
                start <= to && to < end
            }
        })
        .unwrap_or(0);
    let mut position = 0_u32;
    let moved = ranges
        .iter()
        .enumerate()
        .map(|(index, range)| {
            let (start, end) = bounds(range);
            let kept = (from.clamp(start, end) - start) + (end - to.clamp(start, end));
            let range = DWRITE_TEXT_RANGE {
                startPosition: position,
                length: kept + u32::from(index == carrier),
            };
            position += range.length;
            range
        })
        .collect();
    (short, moved)
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
        // DirectWrite only trims the end of a line: a head or middle
        // ellipsis is laid out from a text shortened here.
        if let Some(cut) = self.elision(dwrite, block, &text, &ranges, max_width) {
            let (short, moved) = elide(&text, &ranges, cut);
            if let Some(mut layout) = self.layout(dwrite, block, &short, moved, max_width) {
                layout.elided = Some(cut);
                return Some(layout);
            }
        }
        self.layout(dwrite, block, &text, ranges, max_width)
    }

    /// The UTF-16 range a head or middle ellipsis stands for: the fewest
    /// clusters to take out for the line to fit `max_width`, found by
    /// measuring. `None` when the block is not a single line with such an
    /// ellipsis, or when the line fits.
    fn elision(
        &self,
        dwrite: &IDWriteFactory,
        block: &TextBlock,
        text: &[u16],
        ranges: &[DWRITE_TEXT_RANGE],
        max_width: f32,
    ) -> Option<(u32, u32)> {
        let head = match block.ellipsize {
            Ellipsize::Head => true,
            Ellipsize::Middle => false,
            Ellipsize::Tail | Ellipsize::Clip => return None,
        };
        if block.max_lines != 1 || !max_width.is_finite() || max_width <= 0.0 || text.is_empty() {
            return None;
        }
        let line = self.layout(dwrite, block, text, ranges.to_vec(), f32::INFINITY)?;
        // Explicit line breaks make more than one line: those keep the end ellipsis.
        if line.truncated || line.natural_width <= max_width {
            return None;
        }
        let stops = cluster_stops(&line.layout, text);
        let clusters = stops.len() - 1;
        let fits = |removed: usize| {
            let (short, moved) = elide(text, ranges, cut_range(&stops, removed, head));
            self.layout(dwrite, block, &short, moved, f32::INFINITY)
                .is_some_and(|layout| layout.natural_width <= max_width)
        };
        // Everything goes when not even one cluster fits beside the ellipsis.
        let (mut low, mut high) = (1, clusters);
        while low < high {
            let middle = low + (high - low) / 2;
            if fits(middle) {
                high = middle;
            } else {
                low = middle + 1;
            }
        }
        Some(cut_range(&stops, low, head))
    }

    /// Lays out `text` with the style of the block's runs over `ranges`.
    fn layout(
        &self,
        dwrite: &IDWriteFactory,
        block: &TextBlock,
        text: &[u16],
        ranges: Vec<DWRITE_TEXT_RANGE>,
        max_width: f32,
    ) -> Option<Layout> {
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
                .CreateTextLayout(text, &format, width, UNBOUNDED)
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
                elided: None,
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
        // The core's key leaves the ellipsis mode out, and a line cut at
        // its start or middle is not as wide as one cut at its end.
        let mode = match block.ellipsize {
            Ellipsize::Head if block.max_lines == 1 => 0x9E37_79B9_7F4A_7C15,
            Ellipsize::Middle if block.max_lines == 1 => 0xC2B2_AE3D_27D4_EB4F,
            _ => 0,
        };
        let key = block.layout_key(max_width) ^ mode;
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
                let start = self.visible_index(start);
                let end = self.visible_index(end).min(self.utf16_length);
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

    /// The UTF-16 offset, in the block's text, of the character boundary
    /// nearest to `point` (relative to the layout's origin).
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
        self.source_index(index.min(self.utf16_length))
    }

    /// Where an offset of the block's text is in the laid out text: offsets
    /// inside the cut of a head or middle ellipsis land on the ellipsis.
    pub fn visible_index(&self, index: u32) -> u32 {
        match self.elided {
            Some((from, to)) if index >= to => index - (to - from) + 1,
            Some((from, _)) => index.min(from),
            None => index,
        }
    }

    /// The offset of the block's text for one of the laid out text.
    fn source_index(&self, index: u32) -> u32 {
        match self.elided {
            Some((from, to)) if index > from => index - 1 + (to - from),
            _ => index,
        }
    }

    /// Rectangles covering a UTF-16 range of the laid out text, relative to
    /// the layout's origin.
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
