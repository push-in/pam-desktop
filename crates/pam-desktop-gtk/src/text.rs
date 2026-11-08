//! Pango paragraph construction shared by measurement and drawing.

use gtk::pango;
use pam_desktop_ui::geometry::Rect;
use pam_desktop_ui::style::{Border, Decoration, Radii};
use pam_desktop_ui::text::{
    Ellipsize, SpacerMap, TextAlign, TextBlock, TextMetrics, TextRun, TextWrap,
};

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
fn font_description(layout: &pango::Layout, run: &TextRun) -> pango::FontDescription {
    let mut description = pango::FontDescription::new();
    if !run.font.family.is_empty() {
        let (family, changed) = crate::fonts::stack(&run.font.family);
        if changed {
            if let Some(map) = layout.context().font_map() {
                crate::fonts::invalidate_map(&map);
            }
            layout.context_changed();
        }
        if !family.is_empty() {
            description.set_family(&family);
        }
    }
    description.set_absolute_size(f64::from(run.font.size * SCALE));
    description.set_weight(weight(run.font.weight));
    description.set_style(if run.font.italic {
        pango::Style::Italic
    } else {
        pango::Style::Normal
    });
    description
}

/// Ascent and ascent + descent of a run's font in Pango units, as Pango
/// sizes that font's text runs (the font extents, not the fontset metrics),
/// so an edge beside the text gets exactly the text's line box.
fn font_extents(layout: &pango::Layout, run: &TextRun) -> (i32, i32) {
    use pango::prelude::FontExt;
    layout
        .context()
        .load_font(&font_description(layout, run))
        .map_or((0, 0), |font| {
            let (_, logical) = font.glyph_extents(0);
            (-logical.y(), logical.height())
        })
}

pub fn configure(layout: &pango::Layout, block: &TextBlock) {
    let mut text = String::new();
    let attributes = pango::AttrList::new();
    if block.line_height > 0.0 {
        attributes.insert(pango::AttrInt::new_line_height_absolute(units(
            block.line_height,
        )));
    }
    // Inline box edges are characters shaped as empty glyphs of the edge's
    // width, so they take space in the line and the line breaker sees them.
    let spacers = block.spacers();
    let mut next_spacer = 0;
    let mut box_starts = vec![0_u32; block.boxes.len()];
    let mut insert_spacers = |text: &mut String, run: usize| {
        while let Some(spacer) = spacers.get(next_spacer).filter(|spacer| spacer.run == run) {
            next_spacer += 1;
            let start = text.len() as u32;
            let inline = &block.boxes[spacer.box_index];
            if spacer.character == pam_desktop_ui::text::SPACER_OPEN {
                box_starts[spacer.box_index] = start;
            } else if inline.line_height > 0.0 {
                // A box's own line height sizes its line box, like the
                // `line-height` of a CSS inline element.
                let mut height: pango::Attribute =
                    pango::AttrInt::new_line_height_absolute(units(inline.line_height)).into();
                height.set_start_index(box_starts[spacer.box_index]);
                height.set_end_index(start + spacer.character.len_utf8() as u32);
                attributes.insert(height);
            }
            text.push(spacer.character);
            // Vertically the edge spans the box text's font, so a line height
            // gives it the same line box as that text and never grows the line.
            let (ascent, height) = block
                .runs
                .get(inline.runs.start)
                .map_or((0, 0), |run| font_extents(layout, run));
            let ink = pango::Rectangle::new(0, 0, units(spacer.width), 0);
            let logical = pango::Rectangle::new(0, -ascent, units(spacer.width), height);
            let mut shape: pango::Attribute = pango::AttrShape::new(&ink, &logical).into();
            shape.set_start_index(start);
            shape.set_end_index(text.len() as u32);
            attributes.insert(shape);
        }
    };
    for (index, run) in block.runs.iter().enumerate() {
        insert_spacers(&mut text, index);
        let start = text.len() as u32;
        text.push_str(&run.text);
        let end = text.len() as u32;
        if end == start {
            continue;
        }
        let description = font_description(layout, run);
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
            insert(
                pango::AttrColor::new_foreground(wide(color.r), wide(color.g), wide(color.b))
                    .into(),
            );
            if color.a != 255 {
                insert(pango::AttrInt::new_foreground_alpha(wide(color.a)).into());
            }
        }
        if let Some(color) = run.background {
            let wide = |channel: u8| u16::from(channel) * 257;
            insert(
                pango::AttrColor::new_background(wide(color.r), wide(color.g), wide(color.b))
                    .into(),
            );
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
    insert_spacers(&mut text, block.runs.len());
    // CSS never hyphenates unless `hyphens` asks for it.
    attributes.insert(pango::AttrInt::new_insert_hyphens(block.hyphens));
    layout.set_text(&text);
    layout.set_attributes(Some(&attributes));
    layout.set_wrap(match block.wrap {
        TextWrap::Word => pango::WrapMode::Word,
        TextWrap::WordChar => pango::WrapMode::WordChar,
        TextWrap::Char => pango::WrapMode::Char,
    });
    layout.set_alignment(match block.align {
        TextAlign::Start | TextAlign::Justify => pango::Alignment::Left,
        TextAlign::Center => pango::Alignment::Center,
        TextAlign::End => pango::Alignment::Right,
    });
    layout.set_justify(block.align == TextAlign::Justify);
    if block.max_lines == 1 && block.ellipsize == Ellipsize::Clip {
        // `white-space: nowrap`: one line that overflows its box, no ellipsis.
        layout.set_ellipsize(pango::EllipsizeMode::None);
        layout.set_height(-1);
    } else if block.max_lines > 0 {
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

/// Lays text out with unrounded (subpixel) glyph positions, like Chromium on
/// Linux. Pango rounds every glyph advance to whole pixels by default, which
/// makes words up to ~2px per word wider than the web renderer and wraps labels
/// that fit there. Measuring and drawing layouts must share this setting.
pub fn use_subpixel_positions(layout: &pango::Layout) {
    let context = layout.context();
    if context.is_round_glyph_positions() {
        context.set_round_glyph_positions(false);
        layout.context_changed();
    }
}

/// `white-space: nowrap`: a single clipped line that is never wrapped.
#[must_use]
pub fn nowrap(block: &TextBlock) -> bool {
    block.max_lines == 1 && block.ellipsize == Ellipsize::Clip
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
    set_width(
        layout,
        if nowrap(block) {
            f32::INFINITY
        } else {
            max_width
        },
    );
    let (_, logical) = layout.extents();
    // Pango removes the outside half-tracking from its logical rectangle,
    // but the line breaker still needs that advance when constrained again.
    // Preserve it so a measured single-line label does not wrap when drawn
    // at its own intrinsic width (notably small, tracked uppercase labels).
    let tracking = block
        .runs
        .iter()
        .map(|run| run.letter_spacing.max(0.0))
        .fold(0.0_f32, f32::max);
    // Round the width up: a box narrower than its text by a fraction would
    // wrap when drawn. The height stays exact so fractional CSS line heights
    // (19.2px, 16.5px...) stack like a browser line box instead of drifting a
    // pixel per row.
    TextMetrics {
        width: (logical.width() as f32 / SCALE + tracking)
            .ceil()
            .min(max_width),
        height: logical.height() as f32 / SCALE,
        first_baseline: layout.baseline() as f32 / SCALE,
        line_count: layout.line_count().max(0) as u32,
    }
}

/// Height of the laid out text in logical units.
pub fn height(layout: &pango::Layout) -> f32 {
    let (_, logical) = layout.extents();
    logical.height() as f32 / SCALE
}

/// Byte offset of a UTF-16 offset in `text`, clamped to its end.
fn byte_offset(text: &str, utf16: u32) -> usize {
    let mut units = 0_u32;
    for (index, character) in text.char_indices() {
        if units >= utf16 {
            return index;
        }
        units += character.len_utf16() as u32;
    }
    text.len()
}

/// Paints a UTF-16 range of an already configured layout as selected, or
/// restores the paragraph's own attributes with `None`.
pub fn set_selection(layout: &pango::Layout, block: &TextBlock, range: Option<(u32, u32)>) {
    configure(layout, block);
    let Some((start, end)) = range.filter(|(start, end)| start < end) else {
        return;
    };
    let text = layout.text();
    let map = block.spacer_map();
    let (start, end) = (
        byte_offset(&text, map.layout_index(start, true)) as u32,
        byte_offset(&text, map.layout_index(end, false)) as u32,
    );
    let Some(attributes) = layout.attributes() else {
        return;
    };
    // The accent blue every GTK theme uses for selections.
    let mut background: pango::Attribute =
        pango::AttrColor::new_background(0x3535, 0x8484, 0xE4E4).into();
    let mut foreground: pango::Attribute =
        pango::AttrColor::new_foreground(0xFFFF, 0xFFFF, 0xFFFF).into();
    for attribute in [&mut background, &mut foreground] {
        attribute.set_start_index(start);
        attribute.set_end_index(end);
    }
    attributes.insert(background);
    attributes.insert(foreground);
    layout.set_attributes(Some(&attributes));
}

/// Paragraph UTF-16 offset of the character boundary nearest to a point of
/// the layout (inline box edges are not characters of the paragraph).
pub fn index_at(layout: &pango::Layout, map: &SpacerMap, x: f32, y: f32) -> u32 {
    let (_, byte, trailing) = layout.xy_to_index(units(x), units(y));
    let text = layout.text();
    let byte = usize::try_from(byte).unwrap_or(0).min(text.len());
    let before = text
        .get(..byte)
        .map_or(0, |prefix| prefix.encode_utf16().count());
    // `trailing` counts the characters to step over when the point is past
    // the middle of the glyph.
    let after: usize = text.get(byte..).map_or(0, |rest| {
        rest.chars()
            .take(usize::try_from(trailing).unwrap_or(0))
            .map(char::len_utf16)
            .sum()
    });
    map.paragraph_index((before + after) as u32)
}

/// The painted fragments of the inline boxes of a configured, wrapped
/// layout, in layout coordinates: one per line a box spans, sized like CSS
/// (content area of the box's font, or its line height, plus padding and
/// border), each with the decoration to paint under the text.
pub fn box_fragments(layout: &pango::Layout, block: &TextBlock) -> Vec<(Rect, Decoration)> {
    if block.boxes.is_empty() {
        return Vec::new();
    }
    let text = layout.text();
    // Byte range of every box in the layout text: from its opening spacer
    // to the end of its closing spacer.
    let mut ranges = vec![(usize::MAX, 0_usize); block.boxes.len()];
    let spacers = block.spacers();
    let mut spacer = spacers.iter().peekable();
    let mut byte = 0;
    for index in 0..=block.runs.len() {
        while let Some(current) = spacer.next_if(|current| current.run == index) {
            let range = &mut ranges[current.box_index];
            if current.character == pam_desktop_ui::text::SPACER_OPEN {
                range.0 = byte;
            } else {
                range.1 = byte + current.character.len_utf8();
            }
            byte += current.character.len_utf8();
        }
        byte += block.runs.get(index).map_or(0, |run| run.text.len());
    }
    let mut fragments = Vec::new();
    let mut lines = layout.iter();
    loop {
        let Some(line) = lines.line_readonly() else {
            break;
        };
        let baseline = lines.baseline() as f32 / SCALE;
        let x_offset = lines.line_extents().1.x() as f32 / SCALE;
        let line_start = line.start_index().max(0) as usize;
        let line_end = line_start + line.length().max(0) as usize;
        for (inline, &(start, end)) in block.boxes.iter().zip(&ranges) {
            let (start, end) = (start.max(line_start), end.min(line_end));
            if start >= end || end > text.len() {
                continue;
            }
            let Some(run) = block.runs.get(inline.runs.start) else {
                continue;
            };
            let (ascent, height) = font_extents(layout, run);
            let (ascent, descent) = (ascent as f32 / SCALE, (height - ascent) as f32 / SCALE);
            // A line height centres the content area in it (half-leading).
            let (above, below) = if inline.line_height > 0.0 {
                let leading = (inline.line_height - ascent - descent) / 2.0;
                (ascent + leading, descent + leading)
            } else {
                (ascent, descent)
            };
            let top = baseline - above - inline.padding[0] - inline.border[0];
            let bottom = baseline + below + inline.padding[2] + inline.border[2];
            // From the leading edge of the opening spacer to the trailing
            // edge of the closing one (or the line's ends when it wraps).
            let x = |index: usize| x_offset + line.index_to_x(index as i32, false) as f32 / SCALE;
            let (a, b) = (x(start), x(end));
            let spans = [(a.min(b), a.max(b))];
            for (left, right) in spans {
                if right <= left {
                    continue;
                }
                let color = inline.border_color.unwrap_or_default();
                let [top_width, right_width, bottom_width, left_width] = inline.border;
                fragments.push((
                    Rect::new(left, top, right - left, bottom - top),
                    Decoration {
                        background: inline.background.unwrap_or_default(),
                        radii: Radii::all(inline.radius),
                        border: Border {
                            widths: [left_width, top_width, right_width, bottom_width],
                            colors: [color; 4],
                            ..Border::default()
                        },
                        ..Decoration::default()
                    },
                ));
            }
        }
        if !lines.next_line() {
            break;
        }
    }
    fragments
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk::{gio, prelude::*};
    use pam_desktop_ui::text::{FontSpec, TextRun};
    use std::{cell::Cell, rc::Rc};

    fn run(text: &str) -> TextRun {
        TextRun {
            text: text.to_owned(),
            font: FontSpec {
                family: "DejaVu Sans".into(),
                size: 14.0,
                ..FontSpec::default()
            },
            letter_spacing: 0.0,
            color: None,
            background: None,
            underline: false,
            strikethrough: false,
            press: None,
        }
    }

    /// `a <kbd>Ctrl</kbd> b`: the box's horizontal padding and border push
    /// the neighbours, never break away from its text, and are not
    /// characters of the paragraph for hit-testing and selection.
    fn assert_inline_boxes(layout: &pango::Layout) {
        let plain = TextBlock {
            runs: vec![run("a "), run("Ctrl"), run(" b")],
            ..TextBlock::default()
        };
        let boxed = TextBlock {
            boxes: vec![pam_desktop_ui::text::InlineBox {
                runs: 1..2,
                padding: [1.0, 6.0, 1.0, 6.0],
                border: [1.0, 1.0, 2.0, 1.0],
                border_color: Some(pam_desktop_ui::color::Color::WHITE),
                radius: 4.0,
                background: Some(pam_desktop_ui::color::Color::BLACK),
                line_height: 0.0,
            }],
            ..plain.clone()
        };
        let without = measure(layout, &plain, f32::INFINITY).width;
        let with = measure(layout, &boxed, f32::INFINITY).width;
        assert!((with - without - 14.0).abs() <= 1.0, "{without} -> {with}");

        configure(layout, &boxed);
        set_width(layout, f32::INFINITY);
        let fragments = box_fragments(layout, &boxed);
        assert_eq!(fragments.len(), 1);
        let (rect, decoration) = &fragments[0];
        configure(layout, &plain);
        let ctrl_start = layout.index_to_pos(2).x() as f32 / SCALE;
        let ctrl_end = layout.index_to_pos(6).x() as f32 / SCALE;
        configure(layout, &boxed);
        assert!(
            (rect.x - ctrl_start).abs() < 0.5,
            "box at {} for {ctrl_start}",
            rect.x
        );
        assert!(
            (rect.width - (ctrl_end - ctrl_start + 14.0)).abs() < 0.5,
            "box {}",
            rect.width
        );
        assert!(rect.height > 14.0 && decoration.border.widths == [1.0, 1.0, 1.0, 2.0]);

        // The text after the box starts after its trailing edge.
        let map = boxed.spacer_map();
        let b_byte = byte_offset(&layout.text(), map.layout_index(7, true)) as i32;
        let b = layout.index_to_pos(b_byte).x() as f32 / SCALE;
        assert!(b >= rect.x + rect.width, "' b' at {b}");
        // Hit-testing the padding lands on the box edge, the text on its
        // own characters, in paragraph offsets.
        assert_eq!(index_at(layout, &map, rect.x + 2.0, 8.0), 2);
        assert_eq!(index_at(layout, &map, rect.x + rect.width - 1.0, 8.0), 6);
        assert_eq!(index_at(layout, &map, b + 40.0, 8.0), 8);
        set_selection(layout, &boxed, Some((0, 8)));

        // A fixed line height is not grown by the edges, on any line.
        let tall = TextBlock {
            line_height: 20.4,
            ..boxed.clone()
        };
        let one_line = measure(layout, &tall, f32::INFINITY).height;
        assert!((one_line - 20.4).abs() < 0.01, "height {one_line}");
        let wrapped = measure(layout, &tall, ctrl_end - ctrl_start + 16.0);
        assert!(
            (wrapped.height - 20.4 * wrapped.line_count as f32).abs() < 0.05,
            "{} lines, height {}",
            wrapped.line_count,
            wrapped.height
        );

        // Narrow: the line breaks after "a ", the leading edge stays with
        // "Ctrl" (the second line starts at the opening spacer).
        configure(layout, &boxed);
        set_width(layout, ctrl_end - ctrl_start + 16.0);
        let lines = layout.lines_readonly();
        assert!(lines.len() >= 2, "{} lines", lines.len());
        assert_eq!(lines[1].start_index(), 2);
        let fragments = box_fragments(layout, &boxed);
        assert_eq!(fragments.len(), 1);
        assert!(fragments[0].0.x.abs() < 0.5 && fragments[0].0.y > 10.0);
    }

    #[test]
    fn tracked_labels_fit_their_measured_width() {
        gtk::init().expect("run GTK tests under xvfb-run");
        crate::view::assert_backdrop_registry();
        crate::backend::assert_classic_scrollbar();
        crate::input::assert_placeholder_fallback();
        // Transparent widgets stay pickable (CSS opacity does not hit-test).
        let probe = gtk::Button::new();
        probe.set_opacity(f64::from(crate::backend::pickable_opacity(0.0)));
        assert!(probe.opacity() > 0.0 && probe.opacity() < 0.005);
        assert_eq!(crate::backend::pickable_opacity(0.5), 0.5);
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let layout = widget.create_pango_layout(None);
        use_subpixel_positions(&layout);
        for (label, size, tracking) in [
            ("PDV", 9.0, 1.62),
            ("SYNC", 9.0, 1.62),
            ("SISTEMA DE PONTO DE VENDA", 11.52, 1.3824),
            ("CRÉDITO", 10.88, 0.8704),
            ("DÉBITO", 10.88, 0.8704),
        ] {
            let block = TextBlock {
                runs: vec![TextRun {
                    text: label.to_owned(),
                    font: FontSpec {
                        family: "DejaVu Sans".into(),
                        size,
                        weight: 800,
                        ..FontSpec::default()
                    },
                    letter_spacing: tracking,
                    color: None,
                    background: None,
                    underline: false,
                    strikethrough: false,
                    press: None,
                }],
                ..TextBlock::default()
            };
            let metrics = measure(&layout, &block, 314.0);
            configure(&layout, &block);
            set_width(&layout, metrics.width);
            assert_eq!(layout.line_count(), 1, "{label} at {}", metrics.width);
        }

        // Fractional CSS line heights keep their exact height so stacked
        // rows (e.g. 37.2px navigation items) do not drift a pixel per row.
        let block = TextBlock {
            runs: vec![TextRun {
                text: "Dashboard".to_owned(),
                font: FontSpec {
                    family: "DejaVu Sans".into(),
                    size: 12.8,
                    weight: 600,
                    ..FontSpec::default()
                },
                letter_spacing: 0.0,
                color: None,
                background: None,
                underline: false,
                strikethrough: false,
                press: None,
            }],
            line_height: 19.2,
            ..TextBlock::default()
        };
        let metrics = measure(&layout, &block, 314.0);
        assert!(
            (metrics.height - 19.2).abs() < 0.01,
            "height {}",
            metrics.height
        );

        // Subpixel advances match Chromium: "R$ 37,50" in bold 18px is
        // 89.59px there; rounded Pango advances made it 92px and wrapped it
        // inside a 90.5px metric tile.
        let block = TextBlock {
            runs: vec![TextRun {
                text: "R$ 37,50".to_owned(),
                font: FontSpec {
                    family: "DejaVu Sans".into(),
                    size: 18.0,
                    weight: 900,
                    ..FontSpec::default()
                },
                letter_spacing: 0.0,
                color: None,
                background: None,
                underline: false,
                strikethrough: false,
                press: None,
            }],
            ..TextBlock::default()
        };
        let metrics = measure(&layout, &block, 314.0);
        assert!(metrics.width <= 90.0, "width {}", metrics.width);
        assert_inline_boxes(&layout);
        // A smaller kbd with its own CSS line height inside a taller line
        // keeps the paragraph's line box (CSS inline line-height).
        let mut text = run("Busque o cliente ");
        text.font.size = 12.16;
        let mut key = run("Ctrl");
        key.font.size = 11.0;
        key.font.weight = 600;
        let kbd = TextBlock {
            runs: vec![text.clone(), key, text],
            boxes: vec![pam_desktop_ui::text::InlineBox {
                runs: 1..2,
                padding: [1.0, 6.0, 1.0, 6.0],
                border: [1.0, 1.0, 2.0, 1.0],
                line_height: 15.4,
                ..Default::default()
            }],
            line_height: 20.4,
            ..TextBlock::default()
        };
        let height = measure(&layout, &kbd, f32::INFINITY).height;
        assert!((height - 20.4).abs() < 0.01, "kbd line {height}");

        let app = gtk::Application::builder()
            .application_id("com.pamdesktop.gtk-window-state-test")
            .build();
        app.register(None::<&gio::Cancellable>)
            .expect("register isolated test application");
        let window = gtk::ApplicationWindow::new(&app);
        let state_changes = Rc::new(Cell::new(0));
        let observed = Rc::clone(&state_changes);
        crate::shell::watch_toplevel_state(&window, move |_| {
            observed.set(observed.get() + 1);
        });
        WidgetExt::realize(&window);
        let toplevel = window
            .surface()
            .and_then(|surface| surface.downcast::<gtk::gdk::Toplevel>().ok())
            .expect("realized application window has a GDK toplevel");
        toplevel.notify("state");
        assert_eq!(
            state_changes.get(),
            1,
            "GDK state notification is forwarded"
        );
        WidgetExt::unrealize(&window);
    }
}
