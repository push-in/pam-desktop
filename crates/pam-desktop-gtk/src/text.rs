//! Pango paragraph construction shared by measurement and drawing.

use gtk::pango;
use pam_desktop_ui::text::{Ellipsize, TextAlign, TextBlock, TextMetrics};

const SCALE: f32 = pango::SCALE as f32;

fn units(value: f32) -> i32 {
    (value * SCALE).round() as i32
}

fn weight(weight: u16) -> pango::Weight {
    match weight {
        100 => pango::Weight::Thin,
        200 => pango::Weight::Ultralight,
        300 => pango::Weight::Light,
        350 => pango::Weight::Semilight,
        380 => pango::Weight::Book,
        400 => pango::Weight::Normal,
        500 => pango::Weight::Medium,
        600 => pango::Weight::Semibold,
        700 => pango::Weight::Bold,
        800 => pango::Weight::Ultrabold,
        900 => pango::Weight::Heavy,
        1000 => pango::Weight::Ultraheavy,
        other => pango::Weight::__Unknown(i32::from(other)),
    }
}

/// Fills `layout` with the paragraph. Colors are applied as attributes; runs
/// without a color keep the foreground passed when the layout is drawn.
pub fn configure(layout: &pango::Layout, block: &TextBlock) {
    let mut text = String::new();
    let attributes = pango::AttrList::new();
    for run in &block.runs {
        let start = text.len() as u32;
        text.push_str(&run.text);
        let end = text.len() as u32;
        if end == start {
            continue;
        }
        let mut description = pango::FontDescription::new();
        if !run.font.family.is_empty() {
            description.set_family(&run.font.family);
        }
        description.set_absolute_size(f64::from(run.font.size * SCALE));
        description.set_weight(weight(run.font.weight));
        description.set_style(if run.font.italic { pango::Style::Italic } else { pango::Style::Normal });
        let insert = |mut attribute: pango::Attribute| {
            attribute.set_start_index(start);
            attribute.set_end_index(end);
            attributes.insert(attribute);
        };
        insert(pango::AttrFontDesc::new(&description).into());
        if !run.font.features.is_empty() {
            insert(pango::AttrFontFeatures::new(&run.font.features).into());
        }
        if let Some(color) = run.color {
            let wide = |channel: u8| u16::from(channel) * 257;
            insert(pango::AttrColor::new_foreground(wide(color.r), wide(color.g), wide(color.b)).into());
            if color.a != 255 {
                insert(pango::AttrInt::new_foreground_alpha(wide(color.a)).into());
            }
        }
        if let Some(color) = run.background {
            let wide = |channel: u8| u16::from(channel) * 257;
            insert(pango::AttrColor::new_background(wide(color.r), wide(color.g), wide(color.b)).into());
            if color.a != 255 {
                insert(pango::AttrInt::new_background_alpha(wide(color.a)).into());
            }
        }
        if run.underline {
            insert(pango::AttrInt::new_underline(pango::Underline::Single).into());
        }
        if run.strikethrough {
            insert(pango::AttrInt::new_strikethrough(true).into());
        }
        if run.letter_spacing != 0.0 {
            insert(pango::AttrInt::new_letter_spacing(units(run.letter_spacing)).into());
        }
    }
    if block.line_height > 0.0 {
        attributes.insert(pango::AttrInt::new_line_height_absolute(units(block.line_height)));
    }
    layout.set_text(&text);
    layout.set_attributes(Some(&attributes));
    layout.set_wrap(pango::WrapMode::WordChar);
    layout.set_alignment(match block.align {
        TextAlign::Start | TextAlign::Justify => pango::Alignment::Left,
        TextAlign::Center => pango::Alignment::Center,
        TextAlign::End => pango::Alignment::Right,
    });
    layout.set_justify(block.align == TextAlign::Justify);
    if block.max_lines > 0 {
        layout.set_ellipsize(match block.ellipsize {
            Ellipsize::Tail | Ellipsize::Clip => pango::EllipsizeMode::End,
            Ellipsize::Head => pango::EllipsizeMode::Start,
            Ellipsize::Middle => pango::EllipsizeMode::Middle,
        });
        // A negative height is a line count for ellipsized layouts.
        layout.set_height(-(block.max_lines.min(i32::MAX as u32) as i32));
    } else {
        layout.set_ellipsize(pango::EllipsizeMode::None);
        layout.set_height(-1);
    }
}

/// Constrains the layout to `width` logical units (infinite = no wrapping).
pub fn set_width(layout: &pango::Layout, width: f32) {
    if width.is_finite() && width >= 0.0 {
        layout.set_width(units(width.max(0.0)));
    } else {
        layout.set_width(-1);
    }
}

pub fn measure(layout: &pango::Layout, block: &TextBlock, max_width: f32) -> TextMetrics {
    configure(layout, block);
    set_width(layout, max_width);
    let (_, logical) = layout.extents();
    // Round up: a box narrower than its text by a fraction would wrap when drawn.
    TextMetrics {
        width: (logical.width() as f32 / SCALE).ceil(),
        height: (logical.height() as f32 / SCALE).ceil(),
        first_baseline: layout.baseline() as f32 / SCALE,
        line_count: layout.line_count().max(0) as u32,
    }
}

/// Height of the laid out text in logical units.
pub fn height(layout: &pango::Layout) -> f32 {
    let (_, logical) = layout.extents();
    logical.height() as f32 / SCALE
}
