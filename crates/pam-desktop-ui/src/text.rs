//! Paragraph description shared by text measurement and text drawing, so a
//! box is always sized by the exact content a backend later paints.

use std::hash::{Hash, Hasher};
use std::ops::Range;

use pam_desktop_wire::{NodeKind, PropKey};

use crate::color::Color;
use crate::geometry::Point;
use crate::props::Props;

#[derive(Clone, Debug, PartialEq)]
pub struct FontSpec {
    /// Empty selects the platform UI font.
    pub family: String,
    /// Logical size, accessibility scale already applied.
    pub size: f32,
    /// CSS weight `1..=1000`.
    pub weight: u16,
    pub italic: bool,
    /// OpenType feature settings (`"tnum", "liga" 0`), empty when unset.
    pub features: String,
}

impl Default for FontSpec {
    fn default() -> Self {
        Self {
            family: String::new(),
            size: 14.0,
            weight: 400,
            italic: false,
            features: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    pub text: String,
    pub font: FontSpec,
    /// `None` uses the platform's label color.
    pub color: Option<Color>,
    pub background: Option<Color>,
    pub underline: bool,
    pub strikethrough: bool,
    pub letter_spacing: f32,
    /// Slot reported by `SpanPress` when the run is pressable.
    pub press: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum TextAlign {
    #[default]
    Start,
    Center,
    End,
    Justify,
}

/// Line breaking, like CSS: by default only between words, and a word wider
/// than the line overflows it (`overflow-wrap: normal`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextWrap {
    #[default]
    Word,
    /// `overflow-wrap: anywhere | break-word`: words break only when they
    /// cannot fit a line on their own.
    WordChar,
    /// `word-break: break-all`: any character boundary.
    Char,
}

impl TextWrap {
    /// From the wire `TextBreakStrategy` (1 high quality, 2 simple, 3 balanced):
    /// the desktop maps `break-all` to simple and `overflow-wrap` to balanced.
    #[must_use]
    pub fn from_break_strategy(strategy: i64) -> Self {
        match strategy {
            2 => Self::Char,
            3 => Self::WordChar,
            _ => Self::Word,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Ellipsize {
    #[default]
    Tail,
    Head,
    Middle,
    Clip,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextShadow {
    pub offset: Point,
    pub radius: f32,
    pub color: Color,
}

/// The CSS inline box of a nested text (`<span>`/`<kbd>` with padding,
/// border or background). Its horizontal padding and border are space in the
/// line, so neighbours move; the vertical ones only paint, like CSS.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InlineBox {
    /// The runs of the block the box wraps.
    pub runs: Range<usize>,
    /// Top, right, bottom, left.
    pub padding: [f32; 4],
    /// Top, right, bottom, left.
    pub border: [f32; 4],
    pub border_color: Option<Color>,
    pub radius: f32,
    /// Painted under the box's text, padding included.
    pub background: Option<Color>,
    /// Height of the content box (the span's CSS `line-height`); 0 uses the
    /// font's ascent plus descent.
    pub line_height: f32,
}

impl InlineBox {
    /// Space before the first character: left padding and border.
    #[must_use]
    pub fn leading(&self) -> f32 {
        self.padding[3] + self.border[3]
    }

    /// Space after the last character: right padding and border.
    #[must_use]
    pub fn trailing(&self) -> f32 {
        self.padding[1] + self.border[1]
    }
}

/// Opens an inline box in a backend's layout text. Its line break class
/// (opening punctuation) joins the box to the text after it while a line may
/// still break before it, and it is a grapheme of its own, so hit-testing
/// its edge lands on the box. Never drawn: backends replace its glyph.
pub const SPACER_OPEN: char = '(';
/// Closes an inline box: never separated from the text before it (WORD JOINER).
pub const SPACER_CLOSE: char = '\u{2060}';

/// One character a backend inserts in its layout text to reserve the
/// horizontal edge of an inline box, given the advance `width` (a Pango shape
/// attribute, a TextKit attachment, a DirectWrite inline object).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spacer {
    /// Inserted before this run (`runs.len()` for the end of the block).
    pub run: usize,
    pub character: char,
    pub width: f32,
    /// Index in `TextBlock::boxes`.
    pub box_index: usize,
}

/// Converts UTF-16 offsets between the paragraph (what the core selects and
/// hit-tests) and a backend's layout text with the spacers inserted. Each
/// spacer is one UTF-16 unit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpacerMap {
    /// Paragraph offset of each spacer, ascending.
    offsets: Vec<u32>,
}

impl SpacerMap {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    /// Layout offset of a paragraph offset. A range start skips the spacers
    /// at that offset (`after`), a range end stops before them, so selecting
    /// a box's text never paints its padding.
    #[must_use]
    pub fn layout_index(&self, offset: u32, after: bool) -> u32 {
        let before = self
            .offsets
            .partition_point(|&spacer| spacer < offset || (after && spacer == offset));
        offset + before as u32
    }

    /// Paragraph offset of a layout offset; a point inside a spacer is the
    /// box edge it reserves.
    #[must_use]
    pub fn paragraph_index(&self, layout: u32) -> u32 {
        // Spacer `k` sits at layout offset `offsets[k] + k`.
        let before = self
            .offsets
            .iter()
            .enumerate()
            .take_while(|&(index, &spacer)| spacer + (index as u32) < layout)
            .count();
        layout - before as u32
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextBlock {
    pub runs: Vec<TextRun>,
    /// Inline boxes over runs; outer boxes come before the boxes they hold.
    /// A backend lays their edges out as `spacers()` and converts offsets
    /// with `spacer_map()`; one that ignores them draws the plain paragraph.
    pub boxes: Vec<InlineBox>,
    /// Where lines may break (CSS `overflow-wrap`/`word-break`).
    pub wrap: TextWrap,
    /// CSS `hyphens`: a hyphen is drawn where a word is broken.
    pub hyphens: bool,
    pub align: TextAlign,
    /// 0 means unlimited.
    pub max_lines: u32,
    pub ellipsize: Ellipsize,
    /// Fixed line box height in logical units; 0 keeps the font's own.
    pub line_height: f32,
    pub shadow: Option<TextShadow>,
    pub selectable: bool,
}

impl TextBlock {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.runs.iter().all(|run| run.text.is_empty())
    }

    #[must_use]
    pub fn plain_text(&self) -> String {
        self.runs.iter().map(|run| run.text.as_str()).collect()
    }

    /// The spacers of the inline boxes in layout order: at one position the
    /// boxes that end close (inner first) before the boxes that start open
    /// (outer first).
    #[must_use]
    pub fn spacers(&self) -> Vec<Spacer> {
        let mut spacers = Vec::with_capacity(self.boxes.len() * 2);
        for (box_index, inline) in self.boxes.iter().enumerate() {
            if inline.runs.is_empty() || inline.runs.end > self.runs.len() {
                continue;
            }
            spacers.push(Spacer {
                run: inline.runs.start,
                character: SPACER_OPEN,
                width: inline.leading(),
                box_index,
            });
            spacers.push(Spacer {
                run: inline.runs.end,
                character: SPACER_CLOSE,
                width: inline.trailing(),
                box_index,
            });
        }
        spacers.sort_by_key(|spacer| {
            let opens = spacer.character == SPACER_OPEN;
            let nesting = if opens {
                spacer.box_index as isize
            } else {
                -(spacer.box_index as isize)
            };
            (spacer.run, opens, nesting)
        });
        spacers
    }

    #[must_use]
    pub fn spacer_map(&self) -> SpacerMap {
        if self.boxes.is_empty() {
            return SpacerMap::default();
        }
        let mut starts = Vec::with_capacity(self.runs.len() + 1);
        let mut offset = 0_u32;
        for run in &self.runs {
            starts.push(offset);
            offset += utf16_len(&run.text);
        }
        starts.push(offset);
        SpacerMap {
            offsets: self
                .spacers()
                .iter()
                .map(|spacer| starts[spacer.run])
                .collect(),
        }
    }

    /// Hash of everything that can change the measured size.
    #[must_use]
    pub fn layout_key(&self, max_width: f32) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for run in &self.runs {
            run.text.hash(&mut hasher);
            run.font.family.hash(&mut hasher);
            run.font.size.to_bits().hash(&mut hasher);
            run.font.weight.hash(&mut hasher);
            run.font.italic.hash(&mut hasher);
            run.font.features.hash(&mut hasher);
            run.letter_spacing.to_bits().hash(&mut hasher);
        }
        for inline in &self.boxes {
            inline.runs.hash(&mut hasher);
            inline.leading().to_bits().hash(&mut hasher);
            inline.trailing().to_bits().hash(&mut hasher);
        }
        self.max_lines.hash(&mut hasher);
        self.wrap.hash(&mut hasher);
        self.hyphens.hash(&mut hasher);
        (self.ellipsize == Ellipsize::Clip).hash(&mut hasher);
        self.line_height.to_bits().hash(&mut hasher);
        max_width.to_bits().hash(&mut hasher);
        hasher.finish()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextMetrics {
    pub width: f32,
    pub height: f32,
    /// Distance from the top of the box to the first baseline.
    pub first_baseline: f32,
    pub line_count: u32,
}

/// One inline style override over a range of code points.
#[derive(Clone, Debug, Default, PartialEq)]
struct SpanSpec {
    start: usize,
    end: usize,
    size: Option<f32>,
    weight: Option<u16>,
    italic: Option<bool>,
    color: Option<Color>,
    background: Option<Color>,
    decoration: Option<i64>,
    letter_spacing: Option<f32>,
    family: Option<String>,
    press: Option<u32>,
    transform: Option<i64>,
}

/// `start,end,size,weight,italic,color,background,decoration,letterSpacing,
/// family,press,transform` records separated by `;`; offsets are code points.
fn parse_spans(wire: &str) -> Vec<SpanSpec> {
    let mut spans = Vec::new();
    for record in wire.split(';') {
        if record.is_empty() {
            continue;
        }
        let fields: Vec<&str> = record.split(',').collect();
        let field = |index: usize| fields.get(index).copied().filter(|value| !value.is_empty());
        let (Some(start), Some(end)) = (
            field(0).and_then(|value| value.parse::<usize>().ok()),
            field(1).and_then(|value| value.parse::<usize>().ok()),
        ) else {
            continue;
        };
        if end <= start {
            continue;
        }
        spans.push(SpanSpec {
            start,
            end,
            size: field(2)
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|size| size.is_finite() && *size > 0.0),
            weight: field(3)
                .and_then(|value| value.parse::<i64>().ok())
                .map(|weight| weight.clamp(1, 1000) as u16),
            italic: field(4)
                .and_then(|value| value.parse::<i64>().ok())
                .map(|style| style == 2),
            color: field(5)
                .and_then(|value| value.parse::<i64>().ok())
                .map(Color::from_wire),
            background: field(6)
                .and_then(|value| value.parse::<i64>().ok())
                .map(Color::from_wire),
            decoration: field(7).and_then(|value| value.parse::<i64>().ok()),
            letter_spacing: field(8)
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|spacing| spacing.is_finite()),
            family: field(9).map(str::to_owned),
            press: field(10)
                .and_then(|value| value.parse::<i64>().ok())
                .filter(|slot| *slot >= 0)
                .map(|slot| slot as u32),
            transform: field(11).and_then(|value| value.parse::<i64>().ok()),
        });
    }
    spans
}

/// One inline box over a range of code points (`TextSpanBoxes`).
#[derive(Clone, Debug, Default, PartialEq)]
struct BoxSpec {
    start: usize,
    end: usize,
    padding: [f32; 4],
    border: [f32; 4],
    border_color: Option<Color>,
    radius: f32,
    background: Option<Color>,
    line_height: f32,
}

/// `start,end,paddingTop,paddingRight,paddingBottom,paddingLeft,borderTop,
/// borderRight,borderBottom,borderLeft,borderColor,radius,background,
/// lineHeight` records separated by `;`; offsets are code points.
fn parse_boxes(wire: &str) -> Vec<BoxSpec> {
    let mut boxes = Vec::new();
    for record in wire.split(';') {
        let fields: Vec<&str> = record.split(',').collect();
        let field = |index: usize| fields.get(index).copied().filter(|value| !value.is_empty());
        let length = |index: usize| {
            field(index)
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|value| value.is_finite())
                .map_or(0.0, |value| value.clamp(0.0, 1_000.0))
        };
        let color = |index: usize| {
            field(index)
                .and_then(|value| value.parse::<i64>().ok())
                .map(Color::from_wire)
        };
        let (Some(start), Some(end)) = (
            field(0).and_then(|value| value.parse::<usize>().ok()),
            field(1).and_then(|value| value.parse::<usize>().ok()),
        ) else {
            continue;
        };
        if end <= start {
            continue;
        }
        boxes.push(BoxSpec {
            start,
            end,
            padding: [length(2), length(3), length(4), length(5)],
            border: [length(6), length(7), length(8), length(9)],
            border_color: color(10),
            radius: length(11),
            background: color(12),
            line_height: length(13),
        });
    }
    boxes
}

/// 1 none, 2 uppercase, 3 lowercase, 4 capitalize.
#[must_use]
pub fn transform_text(text: &str, transform: i64) -> String {
    match transform {
        2 => text.to_uppercase(),
        3 => text.to_lowercase(),
        4 => {
            let mut output = String::with_capacity(text.len());
            let mut word_start = true;
            for character in text.chars() {
                if character.is_alphanumeric() {
                    if word_start {
                        output.extend(character.to_uppercase());
                    } else {
                        output.push(character);
                    }
                    word_start = false;
                } else {
                    output.push(character);
                    word_start = true;
                }
            }
            output
        }
        _ => text.to_owned(),
    }
}

/// The paragraph inputs of a text-bearing node, exactly as the engine hands
/// them to the host measurer.
#[derive(Clone, Debug)]
pub struct TextSource<'a> {
    pub text: &'a str,
    pub spans: &'a str,
    /// Wire `TextSpanBoxes` (empty when unset).
    pub boxes: &'a str,
    pub family: &'a str,
    pub features: &'a str,
    pub font_size: f32,
    pub font_scale: f32,
    pub letter_spacing: f32,
    pub line_height: f32,
    pub weight: u16,
    pub italic: bool,
    pub transform: i64,
    pub max_lines: u32,
    /// Wire `TextBreakStrategy` (0 when unset).
    pub break_strategy: i64,
    /// Wire `TextHyphenationFrequency` (0 when unset; 1 none).
    pub hyphenation: i64,
    /// `white-space: nowrap` (single line clipped, no ellipsis).
    pub nowrap: bool,
}

impl<'a> TextSource<'a> {
    #[must_use]
    pub fn from_props(kind: NodeKind, props: Props<'a>, font_scale: f32) -> Self {
        let single_line_input =
            kind == NodeKind::Input && !props.flag_or(PropKey::Multiline, false);
        Self {
            text: props.text_or_empty(PropKey::Text),
            spans: props.text_or_empty(PropKey::TextSpans),
            boxes: props.text_or_empty(PropKey::TextSpanBoxes),
            family: props.text_or_empty(PropKey::FontFamily),
            features: props.text_or_empty(PropKey::FontFeatureSettings),
            font_size: props.number_or(PropKey::FontSize, 14.0).max(1.0),
            font_scale: effective_font_scale(props, font_scale),
            letter_spacing: props.number_or(PropKey::LetterSpacing, 0.0),
            line_height: props.number_or(PropKey::LineHeight, 0.0).max(0.0),
            weight: props.integer_or(PropKey::FontWeight, 400).clamp(1, 1000) as u16,
            italic: props.integer(PropKey::FontStyle) == Some(2),
            transform: props.integer_or(PropKey::TextTransform, 1),
            break_strategy: props.integer_or(PropKey::TextBreakStrategy, 0),
            hyphenation: props.integer_or(PropKey::TextHyphenationFrequency, 0),
            nowrap: props.integer(PropKey::NumberOfLines) == Some(1)
                && props.integer(PropKey::TextEllipsizeMode) == Some(4),
            max_lines: if single_line_input {
                1
            } else {
                props
                    .integer(PropKey::NumberOfLines)
                    .filter(|lines| *lines > 0)
                    .map_or(0, |lines| lines.min(i64::from(u32::MAX)) as u32)
            },
        }
    }

    /// Splits the paragraph into uniformly styled runs.
    #[must_use]
    pub fn runs(&self, color: Option<Color>, decoration: i64) -> Vec<TextRun> {
        self.styled(color, decoration).0
    }

    /// The runs and the inline boxes over them; a box always starts and ends
    /// on run boundaries.
    #[must_use]
    pub fn styled(&self, color: Option<Color>, decoration: i64) -> (Vec<TextRun>, Vec<InlineBox>) {
        let base_font = FontSpec {
            family: self.family.to_owned(),
            size: self.font_size * self.font_scale,
            weight: self.weight,
            italic: self.italic,
            features: self.features.to_owned(),
        };
        let base = TextRun {
            text: String::new(),
            font: base_font,
            color,
            background: None,
            underline: matches!(decoration, 2 | 4),
            strikethrough: matches!(decoration, 3 | 4),
            letter_spacing: self.letter_spacing,
            press: None,
        };
        let spans = parse_spans(self.spans);
        let boxes = parse_boxes(self.boxes);
        if spans.is_empty() && boxes.is_empty() {
            return (
                vec![TextRun {
                    text: transform_text(self.text, self.transform),
                    ..base
                }],
                Vec::new(),
            );
        }

        // Boundaries in code points, then one run per segment with every
        // covering span applied in order (later spans win, like nested runs).
        let characters: Vec<char> = self.text.chars().collect();
        let length = characters.len();
        let mut boundaries: Vec<usize> = vec![0, length];
        for (start, end) in spans
            .iter()
            .map(|span| (span.start, span.end))
            .chain(boxes.iter().map(|inline| (inline.start, inline.end)))
        {
            boundaries.push(start.min(length));
            boundaries.push(end.min(length));
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        let mut runs = Vec::with_capacity(boundaries.len());
        let mut segments = Vec::with_capacity(boundaries.len());
        for pair in boundaries.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            if end <= start {
                continue;
            }
            segments.push((start, end));
            let mut run = base.clone();
            let mut transform = self.transform;
            for span in spans
                .iter()
                .filter(|span| span.start <= start && span.end >= end)
            {
                if let Some(size) = span.size {
                    run.font.size = size * self.font_scale;
                }
                if let Some(weight) = span.weight {
                    run.font.weight = weight;
                }
                if let Some(italic) = span.italic {
                    run.font.italic = italic;
                }
                if let Some(color) = span.color {
                    run.color = Some(color);
                }
                if let Some(background) = span.background {
                    run.background = Some(background);
                }
                if let Some(decoration) = span.decoration {
                    run.underline = matches!(decoration, 2 | 4);
                    run.strikethrough = matches!(decoration, 3 | 4);
                }
                if let Some(spacing) = span.letter_spacing {
                    run.letter_spacing = spacing;
                }
                if let Some(family) = &span.family {
                    run.font.family.clone_from(family);
                }
                if span.press.is_some() {
                    run.press = span.press;
                }
                if let Some(span_transform) = span.transform {
                    transform = span_transform;
                }
            }
            let segment: String = characters[start..end].iter().collect();
            run.text = transform_text(&segment, transform);
            runs.push(run);
        }
        let boxes = boxes
            .into_iter()
            .filter_map(|inline| {
                let first = segments
                    .iter()
                    .position(|&(start, _)| start >= inline.start)?;
                let last = segments.iter().rposition(|&(_, end)| end <= inline.end)?;
                (first <= last).then(|| InlineBox {
                    runs: first..last + 1,
                    padding: inline.padding,
                    border: inline.border,
                    border_color: inline.border_color,
                    radius: inline.radius,
                    background: inline.background,
                    line_height: inline.line_height * self.font_scale,
                })
            })
            .collect();
        if runs.is_empty() {
            runs.push(base);
        }
        (runs, boxes)
    }

    /// The paragraph used for measurement (colors and decorations are irrelevant).
    #[must_use]
    pub fn measure_block(&self) -> TextBlock {
        let (runs, boxes) = self.styled(None, 1);
        TextBlock {
            runs,
            boxes,
            wrap: TextWrap::from_break_strategy(self.break_strategy),
            hyphens: self.hyphenation > 1,
            ellipsize: if self.nowrap {
                Ellipsize::Clip
            } else {
                Ellipsize::Tail
            },
            max_lines: self.max_lines,
            line_height: self.line_height * self.font_scale,
            ..TextBlock::default()
        }
    }
}

/// Length of `text` in UTF-16 code units (the unit of protocol offsets).
#[must_use]
pub fn utf16_len(text: &str) -> u32 {
    text.chars()
        .map(|character| character.len_utf16() as u32)
        .sum()
}

/// Converts a UTF-16 offset to a character (code point) offset, clamped to
/// the text and never splitting a surrogate pair.
#[must_use]
pub fn utf16_to_char_offset(text: &str, offset: u32) -> u32 {
    let mut units = 0_u32;
    let mut characters = 0_u32;
    for character in text.chars() {
        let width = character.len_utf16() as u32;
        if units + width > offset {
            break;
        }
        units += width;
        characters += 1;
    }
    characters
}

/// Converts a character (code point) offset to UTF-16 code units.
#[must_use]
pub fn char_to_utf16_offset(text: &str, offset: u32) -> u32 {
    text.chars()
        .take(offset as usize)
        .map(|character| character.len_utf16() as u32)
        .sum()
}

/// Accessibility text scale after `allowFontScaling` / `maxFontSizeMultiplier`.
#[must_use]
pub fn effective_font_scale(props: Props<'_>, scale: f32) -> f32 {
    if !props.flag_or(PropKey::TextAllowFontScaling, true) {
        return 1.0;
    }
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    match props.number(PropKey::TextMaxFontSizeMultiplier) {
        Some(maximum) if maximum >= 1.0 => scale.min(maximum),
        _ => scale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::props::PropMap;
    use pam_desktop_wire::PropValue;

    fn source<'a>(map: &'a PropMap) -> TextSource<'a> {
        TextSource::from_props(NodeKind::Text, Props::new(map), 1.0)
    }

    #[test]
    fn plain_text_is_one_run() {
        let mut map = PropMap::new();
        map.insert(PropKey::Text, PropValue::String("Olá".into()));
        map.insert(PropKey::FontSize, PropValue::Float(18.0));
        map.insert(PropKey::FontWeight, PropValue::Integer(700));
        map.insert(PropKey::TextTransform, PropValue::Integer(2));
        let runs = source(&map).runs(Some(Color::BLACK), 2);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "OLÁ");
        assert_eq!(runs[0].font.size, 18.0);
        assert_eq!(runs[0].font.weight, 700);
        assert!(runs[0].underline && !runs[0].strikethrough);
    }

    #[test]
    fn spans_split_on_code_points_and_nest() {
        let mut map = PropMap::new();
        map.insert(PropKey::Text, PropValue::String("ação já feita".into()));
        // "ação" bold; "já" bold + red + pressable (nested inside a longer bold span).
        map.insert(
            PropKey::TextSpans,
            PropValue::String("0,7,,700;5,7,,,,-65536,,,,,3".into()),
        );
        let runs = source(&map).runs(None, 1);
        let texts: Vec<&str> = runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(texts, vec!["ação ", "já", " feita"]);
        assert_eq!(runs[0].font.weight, 700);
        assert_eq!(runs[1].font.weight, 700);
        assert_eq!(runs[1].color, Some(Color::from_argb(0xFFFF_0000)));
        assert_eq!(runs[1].press, Some(3));
        assert_eq!(runs[2].font.weight, 400);
    }

    #[test]
    fn inline_boxes_split_runs_and_map_offsets() {
        let mut map = PropMap::new();
        map.insert(PropKey::Text, PropValue::String("a Ctrl+Enter b".into()));
        map.insert(PropKey::TextSpans, PropValue::String("2,6,,600".into()));
        map.insert(
            PropKey::TextSpanBoxes,
            PropValue::String("2,6,1,6,1,6,1,1,2,1,-1,4,-16777216,15.4;7,12,,4,,4".into()),
        );
        let (runs, boxes) = source(&map).styled(None, 1);
        let texts: Vec<&str> = runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(texts, vec!["a ", "Ctrl", "+", "Enter", " b"]);
        assert_eq!(boxes.len(), 2);
        assert_eq!(boxes[0].runs, 1..2);
        assert_eq!(boxes[0].leading(), 7.0);
        assert_eq!(boxes[0].border, [1.0, 1.0, 2.0, 1.0]);
        assert_eq!(boxes[0].line_height, 15.4);
        assert_eq!(boxes[1].runs, 3..4);
        assert_eq!(boxes[1].trailing(), 4.0);
        let block = TextBlock {
            runs,
            boxes,
            ..TextBlock::default()
        };
        let spacers: Vec<(usize, char)> = block
            .spacers()
            .iter()
            .map(|spacer| (spacer.run, spacer.character))
            .collect();
        assert_eq!(
            spacers,
            vec![
                (1, SPACER_OPEN),
                (2, SPACER_CLOSE),
                (3, SPACER_OPEN),
                (4, SPACER_CLOSE)
            ]
        );
        // Layout "a ␂Ctrl␃+␂Enter␃ b": paragraph offsets skip the spacers.
        let offsets = block.spacer_map();
        assert_eq!(offsets.layout_index(2, true), 3);
        assert_eq!(offsets.layout_index(2, false), 2);
        assert_eq!(offsets.layout_index(6, false), 7);
        assert_eq!(offsets.layout_index(14, false), 18);
        for (layout, paragraph) in [(0, 0), (2, 2), (3, 2), (7, 6), (8, 6), (9, 7), (18, 14)] {
            assert_eq!(
                offsets.paragraph_index(layout),
                paragraph,
                "layout {layout}"
            );
        }
        // Nested boxes on one range: outer opens first, inner closes first.
        let nested = TextBlock {
            boxes: vec![
                InlineBox {
                    runs: 1..2,
                    ..InlineBox::default()
                },
                InlineBox {
                    runs: 1..2,
                    ..InlineBox::default()
                },
            ],
            ..block.clone()
        };
        let order: Vec<(char, usize)> = nested
            .spacers()
            .iter()
            .map(|spacer| (spacer.character, spacer.box_index))
            .collect();
        assert_eq!(
            order,
            vec![
                (SPACER_OPEN, 0),
                (SPACER_OPEN, 1),
                (SPACER_CLOSE, 1),
                (SPACER_CLOSE, 0)
            ]
        );
    }

    #[test]
    fn single_line_inputs_measure_one_line() {
        let mut map = PropMap::new();
        map.insert(PropKey::NumberOfLines, PropValue::Integer(3));
        assert_eq!(
            TextSource::from_props(NodeKind::Input, Props::new(&map), 1.0).max_lines,
            1
        );
        assert_eq!(
            TextSource::from_props(NodeKind::Text, Props::new(&map), 1.0).max_lines,
            3
        );
    }

    #[test]
    fn font_scale_respects_the_node_policy() {
        let mut map = PropMap::new();
        assert_eq!(effective_font_scale(Props::new(&map), 1.5), 1.5);
        map.insert(PropKey::TextMaxFontSizeMultiplier, PropValue::Float(1.2));
        assert_eq!(effective_font_scale(Props::new(&map), 1.5), 1.2);
        map.insert(PropKey::TextAllowFontScaling, PropValue::Boolean(false));
        assert_eq!(effective_font_scale(Props::new(&map), 1.5), 1.0);
    }

    #[test]
    fn utf16_offsets_convert_without_splitting_pairs() {
        let text = "a😀é";
        assert_eq!(utf16_len(text), 4);
        assert_eq!(utf16_to_char_offset(text, 0), 0);
        assert_eq!(utf16_to_char_offset(text, 1), 1);
        // Offset 2 falls inside the surrogate pair: stay before it.
        assert_eq!(utf16_to_char_offset(text, 2), 1);
        assert_eq!(utf16_to_char_offset(text, 3), 2);
        assert_eq!(utf16_to_char_offset(text, 99), 3);
        assert_eq!(char_to_utf16_offset(text, 2), 3);
        assert_eq!(char_to_utf16_offset(text, 99), 4);
    }

    #[test]
    fn capitalize_only_touches_word_starts() {
        assert_eq!(transform_text("olá mundo-novo", 4), "Olá Mundo-Novo");
    }
}
