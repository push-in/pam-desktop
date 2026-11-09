//! CSS inline formatting contexts (CSS 2.1 §9.4.2, §10.6.1, §10.8), as
//! Blink lays them out.
//!
//! A container whose in-flow children are all inline-level (`display:
//! inline`, `inline-block`, `inline-flex`/`inline-grid`: `InlineLevel`) is a
//! block container of line boxes. Each child is one box of a line (a text
//! keeps its own paragraph); a line breaks before a box that does not fit.
//! Every line box starts with the container's strut: a zero-width box with
//! the container's font and `line-height`, so its ascent and descent (Blink:
//! rounded font metrics plus the floored half-leading) are the least the
//! line holds. Boxes sit on the line's baseline unless their
//! `vertical-align` says otherwise:
//!
//! - an inline text box takes its line height around its baseline, and its
//!   box is the font's content area plus padding and border (painted
//!   padding never grows the line);
//! - an inline-block, inline-flex box or field is atomic: its margin box
//!   sits on the baseline at its own baseline: the first line of its text
//!   (an inline-block: its last line; a single-line field: its centred
//!   text), or the bottom margin edge when it has no line or clips its
//!   overflow (a `<textarea>`), which leaves the strut's descent under it.
//!
//! The lines are aligned horizontally by `text-align`, and as one block
//! vertically like the single item of the container (`align-items` of a
//! row, `justify-content` of a column, a table cell's `vertical-align`).

use std::collections::BTreeMap;

use pam_desktop_wire::{Layout, Node, NodeKind, PropKey, PropValue, Tree};

use super::{
    Axis, LayoutContext, LayoutError, TextMetrics, align_items, border_edges, dimension,
    intrinsic_extent, is_grid, justify_content, layout_node, margin_main, number, out_of_flow,
    table, visible,
};

const INLINE: i64 = 1;
const INLINE_BLOCK: i64 = 2;

/// `InlineLevel` of a box.
fn level(node: &Node) -> Option<i64> {
    super::integer(node, PropKey::InlineLevel).filter(|level| (1..=3).contains(level))
}

/// True when `node` lays its in-flow children (`flow`) on line boxes.
pub(super) fn establishes(node: &Node, flow: &[&Node]) -> bool {
    !flow.is_empty() && container(node) && flow.iter().all(|child| level(child).is_some())
}

/// In-flow children of `node`, in document order.
pub(super) fn flow<'a>(children: &BTreeMap<u64, Vec<&'a Node>>, node: &Node) -> Vec<&'a Node> {
    children
        .get(&node.id)
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .copied()
        .filter(|child| visible(child) && child.kind != NodeKind::Modal && !out_of_flow(child))
        .collect()
}

/// First id of the anonymous block boxes (top of the id space, which the
/// protocol never hands out). They exist only during a layout pass.
pub(super) const ANONYMOUS_BASE: u64 = 1 << 62;

/// Inherited text properties an anonymous block box takes from its parent
/// (its strut and `text-align`).
const INHERITED: [PropKey; 10] = [
    PropKey::FontSize,
    PropKey::FontFamily,
    PropKey::FontWeight,
    PropKey::FontStyle,
    PropKey::LineHeight,
    PropKey::FontFeatureSettings,
    PropKey::LengthUnits,
    PropKey::TextAllowFontScaling,
    PropKey::TextMaxFontSizeMultiplier,
    PropKey::TextAlign,
];

/// Anonymous block boxes (CSS 2.1 §9.2.1.1) of one layout pass: in a
/// vertical container whose in-flow children mix inline-level and block
/// boxes, each run of consecutive inline-level children is wrapped in a
/// block of line boxes with the container's font.
#[derive(Default)]
pub(super) struct Anonymous {
    pub(super) nodes: BTreeMap<u64, Node>,
    /// (parent, anonymous block, its children's ids).
    runs: Vec<(u64, u64, Vec<u64>)>,
}

pub(super) fn anonymous_blocks(tree: &Tree, children: &BTreeMap<u64, Vec<&Node>>) -> Anonymous {
    let mut anonymous = Anonymous::default();
    if !tree
        .nodes
        .values()
        .any(|node| node.properties.contains_key(&PropKey::InlineLevel))
    {
        return anonymous;
    }
    for (parent, kids) in children {
        let Some(parent) = tree.nodes.get(parent) else {
            continue;
        };
        let in_flow =
            |child: &&Node| visible(child) && child.kind != NodeKind::Modal && !out_of_flow(child);
        let flow = kids.iter().copied().filter(in_flow).collect::<Vec<_>>();
        if flows_horizontally(parent)
            || !container(parent)
            || flow.iter().all(|child| level(child).is_some())
            || flow.iter().all(|child| level(child).is_none())
        {
            continue;
        }
        let mut run: Vec<&Node> = Vec::new();
        let close = |run: &mut Vec<&Node>, anonymous: &mut Anonymous| {
            if run.is_empty() {
                return;
            }
            let id = ANONYMOUS_BASE + anonymous.nodes.len() as u64;
            let properties = parent
                .properties
                .iter()
                .filter(|(key, _)| INHERITED.contains(key))
                .map(|(key, value)| (*key, value.clone()))
                .collect();
            anonymous.nodes.insert(
                id,
                Node {
                    id,
                    parent: parent.id,
                    index: run[0].index,
                    kind: NodeKind::Column,
                    properties,
                },
            );
            anonymous
                .runs
                .push((parent.id, id, run.drain(..).map(|node| node.id).collect()));
        };
        for child in flow {
            if level(child).is_some() {
                run.push(child);
            } else {
                close(&mut run, &mut anonymous);
            }
        }
        close(&mut run, &mut anonymous);
    }
    anonymous
}

/// `children` with each run of `anonymous` moved into its block, or `None`
/// when there is none.
pub(super) fn regroup<'a>(
    children: &BTreeMap<u64, Vec<&'a Node>>,
    anonymous: &'a Anonymous,
) -> Option<BTreeMap<u64, Vec<&'a Node>>> {
    if anonymous.runs.is_empty() {
        return None;
    }
    let mut regrouped = children.clone();
    for (parent, id, members) in &anonymous.runs {
        let block = &anonymous.nodes[id];
        let Some(siblings) = regrouped.get_mut(parent) else {
            continue;
        };
        let moved = siblings
            .iter()
            .copied()
            .filter(|child| members.contains(&child.id))
            .collect::<Vec<_>>();
        let Some(position) = siblings.iter().position(|child| child.id == members[0]) else {
            continue;
        };
        siblings.retain(|child| !members.contains(&child.id));
        siblings.insert(position, block);
        regrouped.insert(*id, moved);
    }
    Some(regrouped)
}

/// A box that can be a block container of line boxes.
fn container(node: &Node) -> bool {
    !matches!(
        node.kind,
        NodeKind::Text
            | NodeKind::Input
            | NodeKind::Button
            | NodeKind::Scroll
            | NodeKind::List
            | NodeKind::SectionList
            | NodeKind::VirtualList
            | NodeKind::RefreshControl
            | NodeKind::TabHost
            | NodeKind::NavigationHost
            | NodeKind::DrawerLayout
    ) && !is_grid(node)
        && !matches!(
            table::role(node),
            Some(table::Role::Table | table::Role::Group | table::Role::Row)
        )
}

/// Padding plus border: left, top, right, bottom.
fn insets(node: &Node) -> [f32; 4] {
    let all = number(node, PropKey::Padding).unwrap_or(0.0);
    let horizontal = number(node, PropKey::PaddingHorizontal).unwrap_or(all);
    let vertical = number(node, PropKey::PaddingVertical).unwrap_or(all);
    let side = |key: PropKey, fallback: f32| number(node, key).unwrap_or(fallback).max(0.0);
    let (left, top, right, bottom) = border_edges(node);
    [
        side(PropKey::PaddingLeft, horizontal) + left,
        side(PropKey::PaddingTop, vertical) + top,
        side(PropKey::PaddingRight, horizontal) + right,
        side(PropKey::PaddingBottom, vertical) + bottom,
    ]
}

/// Margins (auto margins are zero in a line): left, top, right, bottom.
fn margins(node: &Node) -> [f32; 4] {
    let finite = |value: f32| if value.is_finite() { value } else { 0.0 };
    let (left, right) = margin_main(node, Axis::Horizontal);
    let (top, bottom) = margin_main(node, Axis::Vertical);
    [finite(left), finite(top), finite(right), finite(bottom)]
}

fn font_scale(node: &Node, text_scale: f32) -> f32 {
    if matches!(
        node.properties.get(&PropKey::TextAllowFontScaling),
        Some(PropValue::Boolean(false))
    ) {
        return 1.0;
    }
    let maximum = number(node, PropKey::TextMaxFontSizeMultiplier).unwrap_or(0.0);
    if maximum > 0.0 {
        text_scale.min(maximum.max(1.0))
    } else {
        text_scale
    }
}

/// A line of text: height, first baseline and line count.
#[derive(Clone, Copy, Debug)]
struct Lines {
    height: f32,
    baseline: f32,
    count: u32,
}

/// Measures the text of `node` (its own `line-height`, or `normal` when
/// `normal`), without insets. Without a host measurer the line box comes
/// from the font size like the engine's text fallback.
fn text_lines(node: &Node, width: f32, text_scale: f32, normal: bool) -> Lines {
    let scale = font_scale(node, text_scale);
    let measured = if normal {
        let mut probe = node.clone();
        probe
            .properties
            .insert(PropKey::LineHeight, PropValue::Float(0.0));
        crate::text_measure::measure(&probe, width, scale)
    } else {
        crate::text_measure::measure(node, width, scale)
    };
    if let Some(measured) = measured {
        return Lines {
            height: measured.height,
            baseline: measured.first_baseline.min(measured.height),
            count: measured.line_count.max(1),
        };
    }
    let size = number(node, PropKey::FontSize).unwrap_or(14.0).max(1.0);
    let line = if normal {
        size * 1.15
    } else {
        number(node, PropKey::LineHeight)
            .filter(|height| *height > 0.0)
            .unwrap_or(size * 1.4)
    } * scale;
    let size = size * scale;
    Lines {
        height: line,
        baseline: (line - size).max(0.0) / 2.0 + size * 0.8,
        count: 1,
    }
}

/// The container's strut: its font and line height on a zero-width text.
fn strut(node: &Node, text_scale: f32, normal: bool) -> Lines {
    const KEYS: [PropKey; 9] = [
        PropKey::FontSize,
        PropKey::FontFamily,
        PropKey::FontWeight,
        PropKey::FontStyle,
        PropKey::LineHeight,
        PropKey::FontFeatureSettings,
        PropKey::LengthUnits,
        PropKey::TextAllowFontScaling,
        PropKey::TextMaxFontSizeMultiplier,
    ];
    let mut properties = node
        .properties
        .iter()
        .filter(|(key, _)| KEYS.contains(key))
        .map(|(key, value)| (*key, value.clone()))
        .collect::<BTreeMap<_, _>>();
    properties.insert(PropKey::Text, PropValue::String("\u{200B}".to_owned()));
    let probe = Node {
        id: node.id,
        parent: node.parent,
        index: node.index,
        kind: NodeKind::Text,
        properties,
    };
    text_lines(&probe, f32::INFINITY, text_scale, normal)
}

/// One box of a line.
#[derive(Clone, Copy, Debug)]
struct Item<'a> {
    node: &'a Node,
    /// Border box.
    width: f32,
    height: f32,
    margins: [f32; 4],
    /// Height of the line box it needs above and below its baseline.
    above: f32,
    below: f32,
    /// Its baseline below the top of its border box.
    baseline: f32,
    align: i64,
}

impl Item<'_> {
    fn outer_width(&self) -> f32 {
        self.margins[0] + self.width + self.margins[2]
    }
}

#[allow(clippy::too_many_arguments)]
fn item<'a>(
    children: &BTreeMap<u64, Vec<&Node>>,
    node: &'a Node,
    available: f32,
    containing_width: f32,
    text_scale: f32,
    text_metrics: &TextMetrics,
    depth: usize,
) -> Result<Item<'a>, LayoutError> {
    let margins = margins(node);
    let room = (available - margins[0] - margins[2]).max(0.0);
    let bound =
        |value: f32, minimum: PropKey, minimum_percent, maximum: PropKey, maximum_percent| {
            let value = dimension(node, maximum, maximum_percent, containing_width)
                .map_or(value, |maximum| value.min(maximum));
            dimension(node, minimum, minimum_percent, containing_width)
                .map_or(value, |minimum| value.max(minimum))
        };
    let width = match dimension(
        node,
        PropKey::Width,
        PropKey::WidthPercent,
        containing_width,
    ) {
        Some(width) => width,
        // Shrink-to-fit (CSS 2.1 §10.3.9): the preferred width, at most the
        // line's room but never below the min-content width (it overflows).
        None => {
            let measure = |width: f32| {
                intrinsic_extent(
                    children,
                    node,
                    Axis::Horizontal,
                    width,
                    f32::INFINITY,
                    text_scale,
                    text_metrics,
                    depth,
                )
            };
            let preferred = measure(f32::INFINITY)?;
            if preferred <= room {
                preferred
            } else {
                let minimum = super::min_content(|| measure(1.0))?;
                room.max(minimum).min(preferred)
            }
        }
    };
    let width = bound(
        width,
        PropKey::MinWidth,
        PropKey::MinWidthPercent,
        PropKey::MaxWidth,
        PropKey::MaxWidthPercent,
    )
    .max(0.0);
    let insets = insets(node);
    let inner = (width - insets[0] - insets[2]).max(0.0);
    let level = level(node).unwrap_or(INLINE);
    let align = super::integer(node, PropKey::VerticalAlign).unwrap_or(1);
    let explicit_height = number(node, PropKey::Height);
    let content_height = |node: &Node| -> Result<f32, LayoutError> {
        let height = match explicit_height {
            Some(height) => height,
            None => intrinsic_extent(
                children,
                node,
                Axis::Vertical,
                width,
                f32::INFINITY,
                text_scale,
                text_metrics,
                depth,
            )?,
        };
        Ok(bound(
            height,
            PropKey::MinHeight,
            PropKey::MinHeightPercent,
            PropKey::MaxHeight,
            PropKey::MaxHeightPercent,
        )
        .max(0.0))
    };
    let atomic = |height: f32, baseline: f32| Item {
        node,
        width,
        height,
        margins,
        above: margins[1] + baseline,
        below: height - baseline + margins[3],
        baseline,
        align,
    };
    if node.kind == NodeKind::Text {
        let lines = text_lines(node, inner, text_scale, false);
        if level == INLINE && lines.count == 1 && explicit_height.is_none() {
            // A non-replaced inline box: the line holds its line height
            // around the baseline; the box is the font's content area plus
            // its padding and border, which overflow the line box.
            let content = text_lines(node, inner, text_scale, true);
            return Ok(Item {
                node,
                width,
                height: insets[1] + content.height + insets[3],
                margins,
                above: lines.baseline,
                below: lines.height - lines.baseline,
                baseline: insets[1] + content.baseline,
                align,
            });
        }
        let height = content_height(node)?;
        // An inline-block's baseline is that of its last line.
        let last = if level == INLINE_BLOCK && lines.count > 1 {
            lines.height / lines.count as f32 * (lines.count - 1) as f32
        } else {
            0.0
        };
        let baseline = (insets[1] + lines.baseline + last).min(height);
        return Ok(atomic(height, baseline));
    }
    let height = content_height(node)?;
    if node.kind == NodeKind::Input {
        let multiline = matches!(
            node.properties.get(&PropKey::Multiline),
            Some(PropValue::Boolean(true))
        );
        if multiline {
            // A textarea scrolls its text: its baseline is the bottom
            // margin edge.
            return Ok(atomic(height, height + margins[3]));
        }
        let lines = text_lines(node, inner, text_scale, false);
        let field = (height - insets[1] - insets[3]).max(0.0);
        let baseline = insets[1] + ((field - lines.height) / 2.0).max(0.0) + lines.baseline;
        return Ok(atomic(height, baseline.min(height)));
    }
    let clips = super::integer(node, PropKey::Overflow) == Some(2);
    let baseline = (!clips)
        .then(|| {
            content_baseline(
                children,
                node,
                height,
                text_scale,
                text_metrics,
                depth,
                level != INLINE_BLOCK,
            )
        })
        .flatten();
    Ok(match baseline {
        Some(baseline) => atomic(height, baseline.min(height)),
        None => atomic(height, height + margins[3]),
    })
}

/// The baseline of the first line inside `node`, `height` tall, below its
/// border-box top: its text, or its first in-flow child's baseline where
/// its flex alignment puts that child. `None` when it holds no text, unless
/// `synthesize` (a flex container): then the first child's bottom border
/// edge stands for it (CSS Flexbox §8.5).
fn content_baseline(
    children: &BTreeMap<u64, Vec<&Node>>,
    node: &Node,
    height: f32,
    text_scale: f32,
    text_metrics: &TextMetrics,
    depth: usize,
    synthesize: bool,
) -> Option<f32> {
    if depth > super::MAX_LAYOUT_DEPTH {
        return None;
    }
    let insets = insets(node);
    if matches!(
        node.kind,
        NodeKind::Text | NodeKind::Input | NodeKind::Button
    ) {
        let lines = text_lines(node, f32::INFINITY, text_scale, false);
        return Some(insets[1] + lines.baseline);
    }
    let first = flow(children, node).into_iter().next()?;
    let margins = margins(first);
    let child_height = number(first, PropKey::Height).or_else(|| {
        intrinsic_extent(
            children,
            first,
            Axis::Vertical,
            f32::INFINITY,
            f32::INFINITY,
            text_scale,
            text_metrics,
            depth + 1,
        )
        .ok()
    })?;
    let horizontal = flows_horizontally(node);
    let room = height - insets[1] - insets[3] - margins[1] - margins[3] - child_height;
    let alignment = super::integer(first, PropKey::AlignSelf)
        .filter(|value| *value != 0)
        .unwrap_or_else(|| align_items(node, Axis::Horizontal));
    // Where the flex container puts its first item: the cross alignment of a
    // row, the main-axis justification of a column (a single item).
    let offset = if horizontal {
        match alignment {
            2 => super::layout_half(room),
            3 => room,
            _ => 0.0,
        }
    } else {
        match justify_content(node, Axis::Vertical) {
            2 => super::layout_half(room),
            3 => room,
            _ => 0.0,
        }
    };
    let baseline = content_baseline(
        children,
        first,
        child_height,
        text_scale,
        text_metrics,
        depth + 1,
        synthesize,
    )
    .or_else(|| synthesize.then_some(child_height))?;
    Some(insets[1] + margins[1] + offset + baseline)
}

/// True for a container whose flex items run horizontally.
fn flows_horizontally(node: &Node) -> bool {
    match super::integer(node, PropKey::FlexDirection) {
        Some(2 | 4) => true,
        Some(1 | 3) => false,
        _ => node.kind == NodeKind::Row,
    }
}

/// A line box: its boxes and where its baseline sits.
struct LineBox<'a> {
    items: Vec<Item<'a>>,
    width: f32,
    above: f32,
    below: f32,
    /// Each box's baseline relative to the line's baseline (vertical-align).
    shifts: Vec<f32>,
}

impl LineBox<'_> {
    fn height(&self) -> f32 {
        self.above + self.below
    }
}

/// Breaks `items` into line boxes `width` wide and aligns each line.
fn line_boxes<'a>(
    node: &Node,
    items: Vec<Item<'a>>,
    width: f32,
    gap: f32,
    text_scale: f32,
) -> Vec<LineBox<'a>> {
    let strut = strut(node, text_scale, false);
    let mut content: Option<Lines> = None;
    let mut content_area = || *content.get_or_insert_with(|| self::strut(node, text_scale, true));
    let font_size =
        number(node, PropKey::FontSize).unwrap_or(14.0).max(1.0) * font_scale(node, text_scale);
    let mut groups: Vec<Vec<Item<'a>>> = Vec::new();
    let mut used = 0.0;
    for item in items {
        let outer = item.outer_width();
        match groups.last_mut() {
            Some(line) if used + gap + outer <= width + 0.01 => {
                used += gap + outer;
                line.push(item);
            }
            _ => {
                used = outer;
                groups.push(vec![item]);
            }
        }
    }
    groups
        .into_iter()
        .map(|items| {
            let mut above = strut.baseline;
            let mut below = strut.height - strut.baseline;
            let mut shifts = Vec::with_capacity(items.len());
            for item in &items {
                let shift = match item.align {
                    // The box's midpoint on the baseline plus half the
                    // parent's x-height (CSS: 0.5em without font data).
                    3 => -font_size * 0.25 - (item.below - item.above) / 2.0,
                    5 => {
                        let area = content_area();
                        item.above - area.baseline
                    }
                    6 => {
                        let area = content_area();
                        (area.height - area.baseline) - item.below
                    }
                    _ => 0.0,
                };
                shifts.push(shift);
                if !matches!(item.align, 2 | 4) {
                    above = above.max(item.above - shift);
                    below = below.max(item.below + shift);
                }
            }
            // `top` and `bottom` boxes align with the line box edges and
            // grow it on the other side when taller.
            for item in items.iter().filter(|item| matches!(item.align, 2 | 4)) {
                let extent = item.above + item.below;
                if extent > above + below {
                    if item.align == 2 {
                        below = extent - above;
                    } else {
                        above = extent - below;
                    }
                }
            }
            for (item, shift) in items.iter().zip(shifts.iter_mut()) {
                match item.align {
                    2 => *shift = item.above - above,
                    4 => *shift = below - item.below,
                    _ => {}
                }
            }
            let width = items.iter().map(Item::outer_width).sum::<f32>()
                + gap * items.len().saturating_sub(1) as f32;
            LineBox {
                items,
                width,
                above,
                below,
                shifts,
            }
        })
        .collect()
}

fn gap(node: &Node) -> f32 {
    number(node, PropKey::GridColumnGap)
        .or_else(|| number(node, PropKey::Gap))
        .filter(|gap| gap.is_finite())
        .unwrap_or(0.0)
        .max(0.0)
}

#[allow(clippy::too_many_arguments)]
fn items<'a>(
    children: &BTreeMap<u64, Vec<&Node>>,
    flow: &[&'a Node],
    width: f32,
    text_scale: f32,
    text_metrics: &TextMetrics,
    depth: usize,
) -> Result<Vec<Item<'a>>, LayoutError> {
    flow.iter()
        .map(|child| {
            item(
                children,
                child,
                width,
                if width.is_finite() { width } else { 0.0 },
                text_scale,
                text_metrics,
                depth + 1,
            )
        })
        .collect()
}

/// Intrinsic extent of an inline formatting context: its widest line, or
/// the height of its lines `available_width` wide, plus its insets.
#[allow(clippy::too_many_arguments)]
pub(super) fn intrinsic(
    children: &BTreeMap<u64, Vec<&Node>>,
    node: &Node,
    flow: &[&Node],
    axis: Axis,
    available_width: f32,
    text_scale: f32,
    text_metrics: &TextMetrics,
    depth: usize,
) -> Result<f32, LayoutError> {
    let insets = insets(node);
    let width = if available_width.is_finite() {
        (available_width - insets[0] - insets[2]).max(0.0)
    } else {
        f32::INFINITY
    };
    let items = items(children, flow, width, text_scale, text_metrics, depth)?;
    let lines = line_boxes(node, items, width, gap(node), text_scale);
    Ok(match axis {
        Axis::Horizontal => {
            lines.iter().map(|line| line.width).fold(0.0, f32::max) + insets[0] + insets[2]
        }
        Axis::Vertical => lines.iter().map(LineBox::height).sum::<f32>() + insets[1] + insets[3],
    })
}

/// Lays the in-flow children of `node` on line boxes inside its content
/// box `inner`.
pub(super) fn layout(
    context: &LayoutContext<'_>,
    node: &Node,
    flow: &[&Node],
    inner: Layout,
    depth: usize,
    output: &mut BTreeMap<u64, Layout>,
) -> Result<(), LayoutError> {
    let gap = gap(node);
    let items = items(
        context.children,
        flow,
        inner.width,
        context.text_scale,
        context.text_metrics,
        depth,
    )?;
    let lines = line_boxes(node, items, inner.width, gap, context.text_scale);
    let total = lines.iter().map(LineBox::height).sum::<f32>();
    let placement = if flows_horizontally(node) {
        align_items(node, Axis::Horizontal)
    } else {
        justify_content(node, Axis::Vertical)
    };
    let free = inner.height - total;
    let mut top = inner.y
        + match placement {
            2 => free / 2.0,
            3 => free,
            _ => 0.0,
        };
    let text_align = super::integer(node, PropKey::TextAlign).unwrap_or(1);
    for line in &lines {
        let free = inner.width - line.width;
        let mut x = inner.x
            + match text_align {
                2 => free / 2.0,
                3 => free,
                _ => 0.0,
            };
        let baseline = top + line.above;
        for (item, shift) in line.items.iter().zip(&line.shifts) {
            let frame = Layout {
                x: x + item.margins[0],
                y: baseline + shift - item.baseline,
                width: item.width,
                height: item.height,
            };
            layout_node(context, item.node.id, frame, false, depth + 1, output)?;
            x += item.outer_width() + gap;
        }
        top += line.height();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use pam_desktop_wire::{Node, NodeKind, PropKey, PropValue, Tree};

    use super::super::{Size, calculate};

    fn node(
        id: u64,
        parent: u64,
        index: u32,
        kind: NodeKind,
        props: &[(PropKey, PropValue)],
    ) -> Node {
        Node {
            id,
            parent,
            index,
            kind,
            properties: props.iter().cloned().collect(),
        }
    }

    fn f(value: f64) -> PropValue {
        PropValue::Float(value)
    }

    fn i(value: i64) -> PropValue {
        PropValue::Integer(value)
    }

    fn text(value: &str) -> PropValue {
        PropValue::String(value.to_owned())
    }

    fn tree(nodes: Vec<Node>) -> Tree {
        Tree {
            root: 1,
            nodes: nodes
                .into_iter()
                .map(|node| (node.id, node))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    /// Without a host measurer a line of text is `font-size × 1.4` tall
    /// (unless `line-height` says otherwise) with its baseline at
    /// `half-leading + 0.8em`, and `normal` is `1.15em`.
    #[test]
    fn inline_text_sits_on_the_strut_baseline() {
        // Parent 16px/24px: strut baseline 4 + 12.8 = 16.8, descent 7.2.
        // Child 10px/12px: baseline 1 + 8 = 9, below 3; content area 11.5
        // (baseline 0.75 + 8 = 8.75).
        let layouts = calculate(
            &tree(vec![
                node(1, 0, 0, NodeKind::Column, &[(PropKey::Width, f(200.0))]),
                node(
                    2,
                    1,
                    0,
                    NodeKind::Row,
                    &[(PropKey::FontSize, f(16.0)), (PropKey::LineHeight, f(24.0))],
                ),
                node(
                    3,
                    2,
                    0,
                    NodeKind::Text,
                    &[
                        (PropKey::Text, text("a")),
                        (PropKey::FontSize, f(10.0)),
                        (PropKey::LineHeight, f(12.0)),
                        (PropKey::InlineLevel, i(1)),
                    ],
                ),
            ]),
            Size {
                width: 200.0,
                height: 400.0,
            },
        )
        .expect("inline layout");
        assert!(
            (layouts[&2].height - 24.0).abs() < 0.02,
            "line {:?}",
            layouts[&2]
        );
        // Box top: line baseline 16.8 - content baseline 8.75.
        assert!(
            (layouts[&3].y - 8.05).abs() < 0.02,
            "text {:?}",
            layouts[&3]
        );
        assert!(
            (layouts[&3].height - 11.5).abs() < 0.02,
            "text {:?}",
            layouts[&3]
        );
    }

    #[test]
    fn a_textarea_sits_on_the_baseline_with_the_strut_descent_under_it() {
        let layouts = calculate(
            &tree(vec![
                node(1, 0, 0, NodeKind::Column, &[(PropKey::Width, f(300.0))]),
                node(
                    2,
                    1,
                    0,
                    NodeKind::Column,
                    &[(PropKey::FontSize, f(16.0)), (PropKey::LineHeight, f(24.0))],
                ),
                node(
                    3,
                    2,
                    0,
                    NodeKind::Input,
                    &[
                        (PropKey::Multiline, PropValue::Boolean(true)),
                        (PropKey::WidthPercent, f(100.0)),
                        (PropKey::Height, f(130.0)),
                        (PropKey::InlineLevel, i(2)),
                    ],
                ),
            ]),
            Size {
                width: 300.0,
                height: 400.0,
            },
        )
        .expect("inline layout");
        assert_eq!(layouts[&3].y, 0.0);
        assert_eq!(layouts[&3].width, 300.0);
        // 130 above the baseline plus the strut's 7.2 descent.
        assert!(
            (layouts[&2].height - 137.2).abs() < 0.02,
            "{:?}",
            layouts[&2]
        );
    }

    #[test]
    fn vertical_align_and_line_breaks() {
        let boxed = |id: u64, index: u32, width: f64, height: f64, align: i64| {
            node(
                id,
                2,
                index,
                NodeKind::View,
                &[
                    (PropKey::Width, f(width)),
                    (PropKey::Height, f(height)),
                    (PropKey::InlineLevel, i(2)),
                    (PropKey::VerticalAlign, i(align)),
                ],
            )
        };
        let layouts = calculate(
            &tree(vec![
                node(1, 0, 0, NodeKind::Column, &[(PropKey::Width, f(100.0))]),
                node(
                    2,
                    1,
                    0,
                    NodeKind::Column,
                    &[(PropKey::FontSize, f(10.0)), (PropKey::LineHeight, f(20.0))],
                ),
                // Strut: baseline 5 + 8 = 13, descent 7.
                boxed(3, 0, 40.0, 30.0, 1),
                boxed(4, 1, 40.0, 10.0, 2),
                boxed(5, 2, 40.0, 50.0, 4),
            ]),
            Size {
                width: 100.0,
                height: 400.0,
            },
        )
        .expect("inline layout");
        // Line 1: the baseline box (30 above the baseline, strut descent 7)
        // and a top box; line 2: a 50px bottom box, taller than the strut.
        assert_eq!(layouts[&3].y, 0.0);
        assert_eq!(layouts[&4].y, 0.0);
        assert_eq!(layouts[&4].x, 40.0);
        assert!((layouts[&5].y - 37.0).abs() < 0.02, "{:?}", layouts[&5]);
        assert_eq!(layouts[&5].x, 0.0);
        assert!(
            (layouts[&2].height - 87.0).abs() < 0.02,
            "{:?}",
            layouts[&2]
        );
    }

    #[test]
    fn inline_runs_among_blocks_get_an_anonymous_block() {
        let block =
            |id: u64, index: u32| node(id, 1, index, NodeKind::View, &[(PropKey::Height, f(10.0))]);
        let layouts = calculate(
            &tree(vec![
                node(
                    1,
                    0,
                    0,
                    NodeKind::Column,
                    &[
                        (PropKey::Width, f(300.0)),
                        (PropKey::FontSize, f(16.0)),
                        (PropKey::LineHeight, f(24.0)),
                        (PropKey::Gap, f(4.0)),
                    ],
                ),
                block(2, 0),
                node(
                    3,
                    1,
                    1,
                    NodeKind::Input,
                    &[
                        (PropKey::Multiline, PropValue::Boolean(true)),
                        (PropKey::WidthPercent, f(100.0)),
                        (PropKey::Height, f(130.0)),
                        (PropKey::InlineLevel, i(2)),
                    ],
                ),
                block(4, 2),
            ]),
            Size {
                width: 300.0,
                height: 400.0,
            },
        )
        .expect("anonymous block layout");
        assert!(layouts.keys().all(|id| *id < super::ANONYMOUS_BASE));
        assert_eq!(layouts[&3].y, 14.0);
        assert_eq!(layouts[&3].width, 300.0);
        // The anonymous block holds the textarea plus the strut's descent.
        assert!(
            (layouts[&4].y - (14.0 + 137.2 + 4.0)).abs() < 0.02,
            "{:?}",
            layouts[&4]
        );
    }

    #[test]
    fn overflow_auto_scrolls_a_wide_table_on_both_axes_with_classic_bars() {
        let cell = |id: u64, parent: u64, index: u32, width: f64| {
            node(
                id,
                parent,
                index,
                NodeKind::View,
                &[
                    (PropKey::TableDisplay, i(4)),
                    (PropKey::Width, f(width)),
                    (PropKey::Height, f(100.0)),
                ],
            )
        };
        let row = |id: u64, index: u32| {
            node(
                id,
                3,
                index,
                NodeKind::Row,
                &[(PropKey::TableDisplay, i(3))],
            )
        };
        let layouts = calculate(
            &tree(vec![
                node(1, 0, 0, NodeKind::Column, &[(PropKey::Width, f(200.0))]),
                node(
                    2,
                    1,
                    0,
                    NodeKind::Scroll,
                    &[
                        (PropKey::Height, f(150.0)),
                        (PropKey::ScrollBothAxes, PropValue::Boolean(true)),
                        (PropKey::ScrollbarColor, i(0x14FF_FFFF)),
                    ],
                ),
                node(3, 2, 0, NodeKind::Column, &[(PropKey::TableDisplay, i(1))]),
                row(4, 0),
                cell(5, 4, 0, 150.0),
                cell(6, 4, 1, 150.0),
                row(7, 1),
                cell(8, 7, 0, 150.0),
                cell(9, 7, 1, 150.0),
            ]),
            Size {
                width: 200.0,
                height: 400.0,
            },
        )
        .expect("two-axis scroll layout");
        // 300 wide and 200 tall in a 200 x 150 viewport: both 15 px bars.
        assert_eq!(layouts[&3].width, 300.0);
        assert_eq!(layouts[&3].height, 200.0);
        assert_eq!(layouts[&6].x, 150.0);
    }

    #[test]
    fn mixed_children_keep_the_flex_layout() {
        let layouts = calculate(
            &tree(vec![
                node(1, 0, 0, NodeKind::Row, &[(PropKey::Width, f(100.0))]),
                node(
                    2,
                    1,
                    0,
                    NodeKind::View,
                    &[
                        (PropKey::Width, f(10.0)),
                        (PropKey::Height, f(10.0)),
                        (PropKey::InlineLevel, i(2)),
                    ],
                ),
                node(
                    3,
                    1,
                    1,
                    NodeKind::View,
                    &[(PropKey::Width, f(10.0)), (PropKey::Height, f(10.0))],
                ),
            ]),
            Size {
                width: 100.0,
                height: 100.0,
            },
        )
        .expect("flex layout");
        assert_eq!(layouts[&3].x, 10.0);
        assert_eq!(layouts[&2].y, 0.0);
    }
}
