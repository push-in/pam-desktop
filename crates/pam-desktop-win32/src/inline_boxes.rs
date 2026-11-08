//! CSS inline boxes of nested texts (`TextSpanBoxes`: a `<span>` or `<kbd>`
//! with padding, border or background) in DirectWrite. The horizontal edges
//! of a box are spacer characters of the layout text replaced by inline
//! objects of the edge's width, so the line breaker and hit-testing see
//! them; vertically an edge never grows the line, unless the box has a line
//! height of its own, like a CSS inline element. The box itself is painted
//! per line under the text (`Fragment`).

use pam_desktop_ui::geometry::Rect;
use pam_desktop_ui::style::{Border, Decoration, Radii};
use pam_desktop_ui::text::{FontSpec, SPACER_OPEN, Spacer, TextAlign, TextBlock};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_BREAK_CONDITION, DWRITE_BREAK_CONDITION_MAY_NOT_BREAK, DWRITE_BREAK_CONDITION_NEUTRAL,
    DWRITE_CLUSTER_METRICS, DWRITE_FONT_METRICS, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_ITALIC, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT,
    DWRITE_HIT_TEST_METRICS, DWRITE_INLINE_OBJECT_METRICS, DWRITE_LINE_METRICS,
    DWRITE_OVERHANG_METRICS, DWRITE_TEXT_RANGE, IDWriteFontCollection, IDWriteInlineObject,
    IDWriteInlineObject_Impl, IDWriteTextLayout, IDWriteTextRenderer,
};
use windows::core::{BOOL, IUnknown, PCWSTR, Ref, Result, implement};

use crate::util::wide;

/// A spacer at its UTF-16 offset in the layout text.
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    pub position: u32,
    pub spacer: Spacer,
}

/// One line's piece of an inline box, in layout coordinates.
#[derive(Clone, Debug)]
pub struct Fragment {
    pub rect: Rect,
    pub decoration: Decoration,
}

/// The layout text of `block` with the spacers of its boxes, the range of
/// every run in it and where the spacers are.
pub fn layout_text(block: &TextBlock) -> (Vec<u16>, Vec<DWRITE_TEXT_RANGE>, Vec<Placed>) {
    let spacers = block.spacers();
    let mut next = spacers.iter().peekable();
    let mut text: Vec<u16> = Vec::new();
    let mut ranges = Vec::with_capacity(block.runs.len());
    let mut placed = Vec::with_capacity(spacers.len());
    for index in 0..=block.runs.len() {
        while let Some(spacer) = next.next_if(|spacer| spacer.run == index) {
            placed.push(Placed {
                position: text.len() as u32,
                spacer: *spacer,
            });
            let mut unit = [0_u16; 2];
            text.extend_from_slice(spacer.character.encode_utf16(&mut unit));
        }
        if let Some(run) = block.runs.get(index) {
            let start = text.len() as u32;
            text.extend(run.text.encode_utf16());
            ranges.push(DWRITE_TEXT_RANGE {
                startPosition: start,
                length: text.len() as u32 - start,
            });
        }
    }
    (text, ranges, placed)
}

/// Ascent and descent of a font, in logical units: what CSS calls the
/// content area of an inline box.
pub fn font_extents(
    collection: Option<&IDWriteFontCollection>,
    family: &str,
    font: &FontSpec,
) -> (f32, f32) {
    let fallback = (font.size * 0.9, font.size * 0.25);
    let Some(collection) = collection else {
        return fallback;
    };
    let name = wide(family);
    let style = if font.italic {
        DWRITE_FONT_STYLE_ITALIC
    } else {
        DWRITE_FONT_STYLE_NORMAL
    };
    // SAFETY: COM calls on a live collection; the name is NUL-terminated and
    // the out pointers refer to live locals.
    unsafe {
        let (mut index, mut exists) = (0_u32, BOOL(0));
        if collection
            .FindFamilyName(PCWSTR(name.as_ptr()), &mut index, &mut exists)
            .is_err()
            || !exists.as_bool()
        {
            return fallback;
        }
        let Ok(found) = collection.GetFontFamily(index).and_then(|family| {
            family.GetFirstMatchingFont(
                DWRITE_FONT_WEIGHT(i32::from(font.weight.clamp(1, 999))),
                DWRITE_FONT_STRETCH_NORMAL,
                style,
            )
        }) else {
            return fallback;
        };
        let mut metrics = DWRITE_FONT_METRICS::default();
        found.GetMetrics(&mut metrics);
        if metrics.designUnitsPerEm == 0 {
            return fallback;
        }
        let unit = font.size / f32::from(metrics.designUnitsPerEm);
        (
            f32::from(metrics.ascent) * unit,
            f32::from(metrics.descent) * unit,
        )
    }
}

/// The height and baseline an edge takes in its line: nothing, so the line
/// keeps the height of its text, or the box's own line height around the
/// content area (CSS half-leading).
fn edge_metrics(
    width: f32,
    line_height: f32,
    (ascent, descent): (f32, f32),
) -> DWRITE_INLINE_OBJECT_METRICS {
    let (height, baseline) = if line_height > 0.0 {
        let leading = (line_height - ascent - descent) / 2.0;
        (line_height, ascent + leading)
    } else {
        (0.0, 0.0)
    };
    DWRITE_INLINE_OBJECT_METRICS {
        width: width.max(0.0),
        height,
        baseline,
        supportsSideways: BOOL(0),
    }
}

/// An edge of an inline box: empty space of the edge's width.
#[implement(IDWriteInlineObject)]
struct Edge {
    metrics: DWRITE_INLINE_OBJECT_METRICS,
    opens: bool,
}

impl IDWriteInlineObject_Impl for Edge_Impl {
    fn Draw(
        &self,
        _context: *const core::ffi::c_void,
        _renderer: Ref<IDWriteTextRenderer>,
        _x: f32,
        _y: f32,
        _sideways: BOOL,
        _right_to_left: BOOL,
        _effect: Ref<IUnknown>,
    ) -> Result<()> {
        // The box is painted under the line, not by its edges.
        Ok(())
    }

    fn GetMetrics(&self) -> Result<DWRITE_INLINE_OBJECT_METRICS> {
        Ok(self.metrics)
    }

    fn GetOverhangMetrics(&self) -> Result<DWRITE_OVERHANG_METRICS> {
        Ok(DWRITE_OVERHANG_METRICS::default())
    }

    fn GetBreakConditions(
        &self,
        before: *mut DWRITE_BREAK_CONDITION,
        after: *mut DWRITE_BREAK_CONDITION,
    ) -> Result<()> {
        // A line may break before a box, never between an edge and the
        // box's text.
        let (first, second) = if self.opens {
            (
                DWRITE_BREAK_CONDITION_NEUTRAL,
                DWRITE_BREAK_CONDITION_MAY_NOT_BREAK,
            )
        } else {
            (
                DWRITE_BREAK_CONDITION_MAY_NOT_BREAK,
                DWRITE_BREAK_CONDITION_NEUTRAL,
            )
        };
        // SAFETY: DirectWrite passes valid out pointers (checked for null).
        unsafe {
            if !before.is_null() {
                *before = first;
            }
            if !after.is_null() {
                *after = second;
            }
        }
        Ok(())
    }
}

/// Puts the edges of the boxes into `layout`. `extents` gives the content
/// area of a box's first run.
pub fn attach(
    layout: &IDWriteTextLayout,
    block: &TextBlock,
    placed: &[Placed],
    extents: &dyn Fn(&FontSpec) -> (f32, f32),
) {
    for place in placed {
        let Some(inline) = block.boxes.get(place.spacer.box_index) else {
            continue;
        };
        let content = block
            .runs
            .get(inline.runs.start)
            .map_or((0.0, 0.0), |run| extents(&run.font));
        let edge: IDWriteInlineObject = Edge {
            metrics: edge_metrics(place.spacer.width, inline.line_height, content),
            opens: place.spacer.character == SPACER_OPEN,
        }
        .into();
        // SAFETY: COM call on a live layout; the range is inside its text.
        unsafe {
            let _ = layout.SetInlineObject(
                &edge,
                DWRITE_TEXT_RANGE {
                    startPosition: place.position,
                    length: 1,
                },
            );
        }
    }
}

/// The painted pieces of the boxes of a laid out block: one per line a box
/// spans, from the leading edge of its opening spacer to the trailing edge
/// of its closing one, as tall as the content area of its text (or its line
/// height) plus its vertical padding and border.
pub fn fragments(
    layout: &IDWriteTextLayout,
    block: &TextBlock,
    placed: &[Placed],
    extents: &dyn Fn(&FontSpec) -> (f32, f32),
) -> Vec<Fragment> {
    let mut fragments = Vec::new();
    // SAFETY: COM calls on a live layout; buffers are live and sized.
    unsafe {
        let mut count = 0_u32;
        let mut lines = vec![DWRITE_LINE_METRICS::default(); 8];
        if layout.GetLineMetrics(Some(&mut lines), &mut count).is_err() {
            lines.resize(count as usize, DWRITE_LINE_METRICS::default());
            if layout.GetLineMetrics(Some(&mut lines), &mut count).is_err() {
                return fragments;
            }
        }
        lines.truncate(count as usize);
        // Top of every line.
        let mut tops = Vec::with_capacity(lines.len());
        let mut top = 0.0_f32;
        for line in &lines {
            tops.push(top);
            top += line.height;
        }
        let clusters = cluster_edges(layout, block, &lines);
        for (box_index, inline) in block.boxes.iter().enumerate() {
            let start = placed.iter().find(|place| {
                place.spacer.box_index == box_index && place.spacer.character == SPACER_OPEN
            });
            let end = placed.iter().find(|place| {
                place.spacer.box_index == box_index && place.spacer.character != SPACER_OPEN
            });
            let (Some(start), Some(end)) = (start, end) else {
                continue;
            };
            let (ascent, descent) = block
                .runs
                .get(inline.runs.start)
                .map_or((0.0, 0.0), |run| extents(&run.font));
            let (above, below) = if inline.line_height > 0.0 {
                let leading = (inline.line_height - ascent - descent) / 2.0;
                (ascent + leading, descent + leading)
            } else {
                (ascent, descent)
            };
            let color = inline.border_color.unwrap_or_default();
            let [top_width, right_width, bottom_width, left_width] = inline.border;
            let decoration = Decoration {
                background: inline.background.unwrap_or_default(),
                radii: Radii::all(inline.radius),
                border: Border {
                    widths: [left_width, top_width, right_width, bottom_width],
                    colors: [color; 4],
                    ..Border::default()
                },
                ..Decoration::default()
            };
            // Per line the box spans: from the leading edge of its first
            // character there to the trailing edge of its last.
            let (from, to) = (start.position, end.position + 1);
            let mut line_start = 0_u32;
            for (line, metrics) in lines.iter().enumerate() {
                let line_end = line_start + metrics.length;
                let (first, last) = (from.max(line_start), to.min(line_end));
                line_start = line_end;
                if first >= last {
                    continue;
                }
                let edge = |position: u32, trailing: bool| {
                    let (mut x, mut y) = (0.0_f32, 0.0_f32);
                    let mut hit = DWRITE_HIT_TEST_METRICS::default();
                    layout
                        .HitTestTextPosition(position, trailing, &mut x, &mut y, &mut hit)
                        .ok()
                        .map(|()| x)
                        .or_else(|| {
                            // Without hit-testing (Wine), from the clusters.
                            let unit = clusters.get(position as usize)?;
                            Some(if trailing { unit.1 } else { unit.0 })
                        })
                };
                let (Some(left), Some(right)) = (edge(first, false), edge(last - 1, true)) else {
                    continue;
                };
                let (left, right) = (left.min(right), left.max(right));
                if right <= left {
                    continue;
                }
                let baseline = tops[line] + metrics.baseline;
                let top = baseline - above - inline.padding[0] - inline.border[0];
                let bottom = baseline + below + inline.padding[2] + inline.border[2];
                fragments.push(Fragment {
                    rect: Rect::new(left, top, right - left, bottom - top),
                    decoration: decoration.clone(),
                });
            }
        }
    }
    fragments
}

/// The leading and trailing x of the cluster of every UTF-16 unit, summed
/// from the cluster advances and the line alignment: what hit-testing gives
/// where DirectWrite cannot hit-test (Wine).
fn cluster_edges(
    layout: &IDWriteTextLayout,
    block: &TextBlock,
    lines: &[DWRITE_LINE_METRICS],
) -> Vec<(f32, f32)> {
    let total: u32 = lines.iter().map(|line| line.length).sum();
    let mut clusters = vec![DWRITE_CLUSTER_METRICS::default(); total as usize];
    let mut count = 0_u32;
    // SAFETY: the buffer is live and sized; a layout never has more
    // clusters than UTF-16 units.
    let (reported, max_width) = unsafe {
        (
            layout.GetClusterMetrics(Some(&mut clusters), &mut count),
            layout.GetMaxWidth(),
        )
    };
    if reported.is_err() {
        return Vec::new();
    }
    clusters.truncate(count as usize);
    let mut edges = Vec::with_capacity(total as usize);
    let mut cluster = clusters.iter().peekable();
    for line in lines {
        let start = edges.len() as u32;
        // The clusters of this line and their width without trailing spaces.
        let mut members = Vec::new();
        let mut covered = 0_u32;
        while covered < line.length {
            let Some(next) = cluster.next() else { break };
            covered += u32::from(next.length);
            members.push(*next);
        }
        let mut width: f32 = members.iter().map(|member| member.width).sum();
        for member in members.iter().rev() {
            if member._bitfield & 0x2 == 0 {
                break;
            }
            width -= member.width;
        }
        let free = if max_width < 1.0e5 {
            (max_width - width).max(0.0)
        } else {
            0.0
        };
        let mut x = match block.align {
            TextAlign::Center => free / 2.0,
            TextAlign::End => free,
            TextAlign::Start | TextAlign::Justify => 0.0,
        };
        for member in members {
            for _ in 0..member.length {
                edges.push((x, x + member.width));
            }
            x += member.width;
        }
        edges.truncate((start + line.length) as usize);
    }
    edges
}

#[cfg(test)]
mod tests {
    use super::*;
    use pam_desktop_ui::color::Color;
    use pam_desktop_ui::geometry::Point;
    use pam_desktop_ui::text::{InlineBox, TextRun};
    use windows::Win32::Graphics::DirectWrite::{
        DWRITE_FACTORY_TYPE_SHARED, DWriteCreateFactory, IDWriteFactory,
    };

    fn run(text: &str) -> TextRun {
        TextRun {
            text: text.to_owned(),
            font: FontSpec {
                size: 14.0,
                ..FontSpec::default()
            },
            color: None,
            background: None,
            underline: false,
            strikethrough: false,
            letter_spacing: 0.0,
            press: None,
        }
    }

    fn block() -> TextBlock {
        TextBlock {
            runs: vec![run("Press "), run("Ctrl"), run(" now")],
            boxes: vec![InlineBox {
                runs: 1..2,
                padding: [2.0, 6.0, 2.0, 6.0],
                border: [1.0, 1.0, 2.0, 1.0],
                border_color: Some(Color::rgba(0, 0, 0, 255)),
                radius: 4.0,
                background: Some(Color::rgba(230, 230, 230, 255)),
                line_height: 0.0,
            }],
            ..TextBlock::default()
        }
    }

    #[test]
    fn spacers_surround_the_box_text() {
        let (text, ranges, placed) = layout_text(&block());
        assert_eq!(
            String::from_utf16(&text).unwrap(),
            "Press (Ctrl\u{2060} now"
        );
        let starts: Vec<u32> = ranges.iter().map(|range| range.startPosition).collect();
        assert_eq!(starts, [0, 7, 12]);
        let positions: Vec<u32> = placed.iter().map(|place| place.position).collect();
        assert_eq!(positions, [6, 11]);
        assert_eq!(placed[0].spacer.width, 7.0);
    }

    #[test]
    fn boxes_take_their_width_in_the_line_and_paint_per_line() {
        // SAFETY: creating the shared DirectWrite factory.
        let dwrite: IDWriteFactory =
            unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }.unwrap();
        let engine = crate::text::TextEngine::new(&dwrite);
        let boxed = block();
        let plain = TextBlock {
            boxes: Vec::new(),
            ..boxed.clone()
        };
        let with = engine.build(&dwrite, &boxed, f32::INFINITY).unwrap();
        let without = engine.build(&dwrite, &plain, f32::INFINITY).unwrap();
        // 7 before the box text, 7 after it; the line keeps its height.
        assert!(
            (with.natural_width - without.natural_width - 14.0).abs() < 0.5,
            "{} vs {}",
            with.natural_width,
            without.natural_width
        );
        assert!((with.height - without.height).abs() < 0.5);
        assert_eq!(with.boxes.len(), 1);
        let fragment = &with.boxes[0];
        assert!(fragment.rect.width > 14.0 && fragment.rect.height > 14.0);
        // Paragraph offsets skip the edges: the box text starts at 6.
        assert_eq!(with.visible_index(6, true), 7);
        assert_eq!(with.visible_index(6, false), 6);
        // Hit-testing (absent from Wine's DirectWrite) lands on the box text.
        let inside = Point::new(
            fragment.rect.x + 8.0,
            fragment.rect.y + fragment.rect.height / 2.0,
        );
        let index = with.index_at(inside);
        assert!(index == 0 || (6..=7).contains(&index), "{index}");
        // The box starts after "Press " and is 14 wider than "Ctrl".
        assert!(fragment.rect.x > 20.0);
        // Wrapped: the box moves to the next line whole.
        let narrow = engine
            .build(&dwrite, &boxed, without.natural_width - 20.0)
            .unwrap();
        assert!(narrow.line_count >= 2);
        assert_eq!(narrow.boxes.len(), 1);
    }
}
