//! Paragraph description shared by text measurement and text drawing, so a
//! box is always sized by the exact content a backend later paints.

use std::hash::{Hash, Hasher};

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
        Self { family: String::new(), size: 14.0, weight: 400, italic: false, features: String::new() }
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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextBlock {
    pub runs: Vec<TextRun>,
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
        self.max_lines.hash(&mut hasher);
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
            size: field(2).and_then(|value| value.parse::<f32>().ok()).filter(|size| size.is_finite() && *size > 0.0),
            weight: field(3).and_then(|value| value.parse::<i64>().ok()).map(|weight| weight.clamp(1, 1000) as u16),
            italic: field(4).and_then(|value| value.parse::<i64>().ok()).map(|style| style == 2),
            color: field(5).and_then(|value| value.parse::<i64>().ok()).map(Color::from_wire),
            background: field(6).and_then(|value| value.parse::<i64>().ok()).map(Color::from_wire),
            decoration: field(7).and_then(|value| value.parse::<i64>().ok()),
            letter_spacing: field(8).and_then(|value| value.parse::<f32>().ok()).filter(|spacing| spacing.is_finite()),
            family: field(9).map(str::to_owned),
            press: field(10).and_then(|value| value.parse::<i64>().ok()).filter(|slot| *slot >= 0).map(|slot| slot as u32),
            transform: field(11).and_then(|value| value.parse::<i64>().ok()),
        });
    }
    spans
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
}

impl<'a> TextSource<'a> {
    #[must_use]
    pub fn from_props(kind: NodeKind, props: Props<'a>, font_scale: f32) -> Self {
        let single_line_input = kind == NodeKind::Input && !props.flag_or(PropKey::Multiline, false);
        Self {
            text: props.text_or_empty(PropKey::Text),
            spans: props.text_or_empty(PropKey::TextSpans),
            family: props.text_or_empty(PropKey::FontFamily),
            features: props.text_or_empty(PropKey::FontFeatureSettings),
            font_size: props.number_or(PropKey::FontSize, 14.0).max(1.0),
            font_scale: effective_font_scale(props, font_scale),
            letter_spacing: props.number_or(PropKey::LetterSpacing, 0.0),
            line_height: props.number_or(PropKey::LineHeight, 0.0).max(0.0),
            weight: props.integer_or(PropKey::FontWeight, 400).clamp(1, 1000) as u16,
            italic: props.integer(PropKey::FontStyle) == Some(2),
            transform: props.integer_or(PropKey::TextTransform, 1),
            max_lines: if single_line_input {
                1
            } else {
                props.integer(PropKey::NumberOfLines).filter(|lines| *lines > 0).map_or(0, |lines| lines.min(i64::from(u32::MAX)) as u32)
            },
        }
    }

    /// Splits the paragraph into uniformly styled runs.
    #[must_use]
    pub fn runs(&self, color: Option<Color>, decoration: i64) -> Vec<TextRun> {
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
        if spans.is_empty() {
            return vec![TextRun { text: transform_text(self.text, self.transform), ..base }];
        }

        // Boundaries in code points, then one run per segment with every
        // covering span applied in order (later spans win, like nested runs).
        let characters: Vec<char> = self.text.chars().collect();
        let length = characters.len();
        let mut boundaries: Vec<usize> = vec![0, length];
        for span in &spans {
            boundaries.push(span.start.min(length));
            boundaries.push(span.end.min(length));
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        let mut runs = Vec::with_capacity(boundaries.len());
        for pair in boundaries.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            if end <= start {
                continue;
            }
            let mut run = base.clone();
            let mut transform = self.transform;
            for span in spans.iter().filter(|span| span.start <= start && span.end >= end) {
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
        if runs.is_empty() {
            runs.push(base);
        }
        runs
    }

    /// The paragraph used for measurement (colors and decorations are irrelevant).
    #[must_use]
    pub fn measure_block(&self) -> TextBlock {
        TextBlock {
            runs: self.runs(None, 1),
            max_lines: self.max_lines,
            line_height: self.line_height * self.font_scale,
            ..TextBlock::default()
        }
    }
}

/// Accessibility text scale after `allowFontScaling` / `maxFontSizeMultiplier`.
#[must_use]
pub fn effective_font_scale(props: Props<'_>, scale: f32) -> f32 {
    if !props.flag_or(PropKey::TextAllowFontScaling, true) {
        return 1.0;
    }
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
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
        map.insert(PropKey::TextSpans, PropValue::String("0,7,,700;5,7,,,,-65536,,,,,3".into()));
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
    fn single_line_inputs_measure_one_line() {
        let mut map = PropMap::new();
        map.insert(PropKey::NumberOfLines, PropValue::Integer(3));
        assert_eq!(TextSource::from_props(NodeKind::Input, Props::new(&map), 1.0).max_lines, 1);
        assert_eq!(TextSource::from_props(NodeKind::Text, Props::new(&map), 1.0).max_lines, 3);
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
    fn capitalize_only_touches_word_starts() {
        assert_eq!(transform_text("olá mundo-novo", 4), "Olá Mundo-Novo");
    }
}
