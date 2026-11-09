//! CSS tables (`display: table`, `table-row-group`, `table-row`,
//! `table-cell`): the automatic and fixed column width algorithms, row
//! heights from their cells, `vertical-align` of cells and the collapsing
//! border model (CSS 2.1 §17.6.2), where each grid line takes the widest of
//! the borders that meet on it and every row and cell takes half of it.
//!
//! A table lays out its rows itself: its children are rows or row groups
//! (any container, a `VirtualizedList` included, whose children are rows),
//! and the children of a row are its cells. Cells keep their own flex layout
//! inside the box the table gives them.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use pam_desktop_wire::{Layout, Node, NodeKind, PropKey};

use super::{
    Axis, LayoutContext, LayoutError, TextMetrics, border_edges_raw, dimension, integer,
    intrinsic_extent, layout_node, number, visible,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Role {
    Table,
    Group,
    Row,
    Cell,
}

/// The table role of a node (`TableDisplay`).
pub(super) fn role(node: &Node) -> Option<Role> {
    match integer(node, PropKey::TableDisplay)? {
        1 => Some(Role::Table),
        2 => Some(Role::Group),
        3 => Some(Role::Row),
        4 => Some(Role::Cell),
        _ => None,
    }
}

thread_local! {
    /// Border widths that take space in the collapsing model (left, top,
    /// right, bottom): halves of the grid lines for cells, nothing for rows,
    /// the outer halves for the table. Rebuilt whenever a table is measured.
    static COLLAPSED: RefCell<HashMap<u64, (f32, f32, f32, f32)>> = RefCell::new(HashMap::new());
}

/// Forgets the collapsed borders of the previous layout pass.
pub(super) fn reset() {
    COLLAPSED.with(|collapsed| collapsed.borrow_mut().clear());
}

/// The border widths `node` occupies when it belongs to a collapsing table.
pub(super) fn collapsed_edges(node: &Node) -> Option<(f32, f32, f32, f32)> {
    integer(node, PropKey::TableDisplay)?;
    COLLAPSED.with(|collapsed| collapsed.borrow().get(&node.id).copied())
}

/// `vertical-align` of a cell as a flex alignment of its content (1 start,
/// 2 center, 3 end). Cells are vertically centred by default, like the
/// `td`/`th` of a browser's default style sheet (`vertical-align: middle`).
pub(super) fn cell_alignment(node: &Node) -> Option<i64> {
    if role(node) != Some(Role::Cell) {
        return None;
    }
    Some(match integer(node, PropKey::VerticalAlign) {
        Some(2 | 5) => 1,
        Some(4 | 6) => 3,
        _ => 2,
    })
}

fn collapsing(table: &Node) -> bool {
    integer(table, PropKey::BorderCollapse) == Some(2)
}

struct Row<'a> {
    node: &'a Node,
    cells: Vec<&'a Node>,
}

enum Entry<'a> {
    Row(usize),
    Group(&'a Node, std::ops::Range<usize>),
}

struct Structure<'a> {
    rows: Vec<Row<'a>>,
    entries: Vec<Entry<'a>>,
}

fn kids<'a>(children: &BTreeMap<u64, Vec<&'a Node>>, node: &Node) -> Vec<&'a Node> {
    children
        .get(&node.id)
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .copied()
        .filter(|child| visible(child) && child.kind != NodeKind::Modal)
        .collect()
}

fn structure<'a>(children: &BTreeMap<u64, Vec<&'a Node>>, table: &Node) -> Structure<'a> {
    let mut rows = Vec::new();
    let mut entries = Vec::new();
    let row = |node: &'a Node| Row {
        node,
        cells: kids(children, node),
    };
    for child in kids(children, table) {
        if role(child) == Some(Role::Row) {
            entries.push(Entry::Row(rows.len()));
            rows.push(row(child));
        } else {
            // A row group: a `table-*-group`, or any container of rows.
            let start = rows.len();
            for grandchild in kids(children, child) {
                rows.push(row(grandchild));
            }
            entries.push(Entry::Group(child, start..rows.len()));
        }
    }
    Structure { rows, entries }
}

/// Grid lines of a collapsing table: `horizontal[r][c]` is the line above
/// row `r` over column `c` (`r == rows` is the bottom edge), `vertical[r][c]`
/// the line left of column `c` in row `r` (`c == columns` the right edge).
struct Lines {
    horizontal: Vec<Vec<f32>>,
    vertical: Vec<Vec<f32>>,
}

fn lines(table: &Node, rows: &[Row<'_>], columns: usize) -> Lines {
    let width = |node: &Node, side: usize| {
        let (left, top, right, bottom) = border_edges_raw(node);
        [left, top, right, bottom][side]
    };
    let cell = |r: usize, c: usize| rows.get(r).and_then(|row| row.cells.get(c).copied());
    let (left, top, right, bottom) = (0, 1, 2, 3);
    let count = rows.len();
    let mut horizontal = vec![vec![0.0_f32; columns]; count + 1];
    for (r, line) in horizontal.iter_mut().enumerate() {
        for (c, value) in line.iter_mut().enumerate() {
            let mut widest = 0.0_f32;
            if r > 0 {
                widest = widest.max(width(rows[r - 1].node, bottom));
                if let Some(above) = cell(r - 1, c) {
                    widest = widest.max(width(above, bottom));
                }
            } else {
                widest = widest.max(width(table, top));
            }
            if r < count {
                widest = widest.max(width(rows[r].node, top));
                if let Some(below) = cell(r, c) {
                    widest = widest.max(width(below, top));
                }
            } else {
                widest = widest.max(width(table, bottom));
            }
            *value = widest;
        }
    }
    let mut vertical = vec![vec![0.0_f32; columns + 1]; count];
    for (r, line) in vertical.iter_mut().enumerate() {
        for (c, value) in line.iter_mut().enumerate() {
            let mut widest = 0.0_f32;
            if c > 0 {
                if let Some(before) = cell(r, c - 1) {
                    widest = widest.max(width(before, right));
                }
            } else {
                widest = widest
                    .max(width(rows[r].node, left))
                    .max(width(table, left));
            }
            if c < columns {
                if let Some(after) = cell(r, c) {
                    widest = widest.max(width(after, left));
                }
            } else {
                widest = widest
                    .max(width(rows[r].node, right))
                    .max(width(table, right));
            }
            *value = widest;
        }
    }
    Lines {
        horizontal,
        vertical,
    }
}

/// Records the collapsed widths every node of the table occupies and
/// returns the table's own (outer halves).
fn record_collapsed(
    table: &Node,
    structure: &Structure<'_>,
    columns: usize,
) -> (f32, f32, f32, f32) {
    let lines = lines(table, &structure.rows, columns);
    let rows = structure.rows.len();
    COLLAPSED.with(|collapsed| {
        let mut collapsed = collapsed.borrow_mut();
        for (r, row) in structure.rows.iter().enumerate() {
            collapsed.insert(row.node.id, (0.0, 0.0, 0.0, 0.0));
            for (c, cell) in row.cells.iter().enumerate() {
                collapsed.insert(
                    cell.id,
                    (
                        lines.vertical[r][c] / 2.0,
                        lines.horizontal[r][c] / 2.0,
                        lines.vertical[r][c + 1] / 2.0,
                        lines.horizontal[r + 1][c] / 2.0,
                    ),
                );
            }
        }
        for entry in &structure.entries {
            if let Entry::Group(group, _) = entry {
                collapsed.insert(group.id, (0.0, 0.0, 0.0, 0.0));
            }
        }
        let widest = |values: &mut dyn Iterator<Item = f32>| values.fold(0.0_f32, f32::max) / 2.0;
        let outer = (
            widest(&mut lines.vertical.iter().map(|line| line[0])),
            widest(&mut lines.horizontal[0].iter().copied()),
            widest(&mut lines.vertical.iter().map(|line| line[columns])),
            widest(&mut lines.horizontal[rows].iter().copied()),
        );
        collapsed.insert(table.id, outer);
        outer
    })
}

/// The table's own insets: in the collapsing model the outer halves of its
/// grid lines (CSS ignores a collapsing table's padding); otherwise its
/// border and padding.
fn insets(table: &Node, outer: Option<(f32, f32, f32, f32)>) -> (f32, f32, f32, f32) {
    if let Some(outer) = outer {
        return outer;
    }
    let length = |key| number(table, key).filter(|value| value.is_finite() && *value >= 0.0);
    let all = length(PropKey::Padding).unwrap_or(0.0);
    let horizontal = length(PropKey::PaddingHorizontal).unwrap_or(all);
    let vertical = length(PropKey::PaddingVertical).unwrap_or(all);
    let (left, top, right, bottom) = border_edges_raw(table);
    (
        left + length(PropKey::PaddingLeft).unwrap_or(horizontal),
        top + length(PropKey::PaddingTop).unwrap_or(vertical),
        right + length(PropKey::PaddingRight).unwrap_or(horizontal),
        bottom + length(PropKey::PaddingBottom).unwrap_or(vertical),
    )
}

struct Geometry {
    insets: (f32, f32, f32, f32),
    widths: Vec<f32>,
    heights: Vec<f32>,
}

#[derive(Clone, Copy, Default)]
struct Column {
    minimum: f32,
    maximum: f32,
    fixed: Option<f32>,
    percent: Option<f32>,
}

#[allow(clippy::too_many_arguments)]
fn geometry(
    children: &BTreeMap<u64, Vec<&Node>>,
    table: &Node,
    structure: &Structure<'_>,
    width: f32,
    text_scale: f32,
    text_metrics: &TextMetrics,
    depth: usize,
) -> Result<Geometry, LayoutError> {
    let columns = structure
        .rows
        .iter()
        .map(|row| row.cells.len())
        .max()
        .unwrap_or(0);
    let outer = collapsing(table).then(|| record_collapsed(table, structure, columns));
    let insets = insets(table, outer);
    let available = if width.is_finite() {
        (width - insets.0 - insets.2).max(0.0)
    } else {
        f32::INFINITY
    };
    let measure = |cell: &Node, axis: Axis, width: f32| {
        intrinsic_extent(
            children,
            cell,
            axis,
            width,
            f32::INFINITY,
            text_scale,
            text_metrics,
            depth + 3,
        )
    };
    let mut specs = vec![Column::default(); columns];
    let fixed_layout = integer(table, PropKey::TableLayout) == Some(2) && available.is_finite();
    for (r, row) in structure.rows.iter().enumerate() {
        for (c, cell) in row.cells.iter().enumerate() {
            let spec = &mut specs[c];
            if let Some(points) = number(cell, PropKey::Width).filter(|value| value.is_finite()) {
                spec.fixed = Some(spec.fixed.map_or(points, |fixed| fixed.max(points)));
            }
            if let Some(percent) =
                number(cell, PropKey::WidthPercent).filter(|value| value.is_finite())
            {
                spec.percent = Some(spec.percent.map_or(percent, |value| value.max(percent)));
            }
            if fixed_layout && r > 0 {
                continue;
            }
            let maximum = measure(cell, Axis::Horizontal, f32::INFINITY)?;
            let minimum = super::min_content(|| measure(cell, Axis::Horizontal, 1.0))?.min(maximum);
            spec.maximum = spec.maximum.max(maximum);
            spec.minimum = spec.minimum.max(minimum);
        }
    }
    let widths = if fixed_layout {
        fixed_widths(&specs, available)
    } else {
        auto_widths(&specs, available)
    };
    let mut heights = Vec::with_capacity(structure.rows.len());
    for row in &structure.rows {
        let mut height = dimension(
            row.node,
            PropKey::Height,
            PropKey::HeightPercent,
            f32::INFINITY,
        )
        .filter(|value| value.is_finite())
        .unwrap_or(0.0)
        .max(number(row.node, PropKey::MinHeight).unwrap_or(0.0));
        for (c, cell) in row.cells.iter().enumerate() {
            height = height.max(measure(cell, Axis::Vertical, widths[c])?);
        }
        heights.push(height);
    }
    Ok(Geometry {
        insets,
        widths,
        heights,
    })
}

/// CSS 2.1 §17.5.2.1 fixed layout: widths of the first row's cells (points
/// or percentages); the others share what is left.
fn fixed_widths(specs: &[Column], available: f32) -> Vec<f32> {
    let declared = |spec: &Column| {
        spec.fixed
            .or_else(|| spec.percent.map(|percent| available * percent / 100.0))
    };
    let used = specs.iter().filter_map(declared).sum::<f32>();
    let open = specs.iter().filter(|spec| declared(spec).is_none()).count();
    let share = if open > 0 {
        ((available - used) / open as f32).max(0.0)
    } else {
        0.0
    };
    specs
        .iter()
        .map(|spec| declared(spec).unwrap_or(share))
        .collect()
}

/// The automatic table layout of browsers: columns start at their
/// preferred widths (max-content, an authored width, a percentage of the
/// table); spare room goes to auto columns in proportion to their
/// max-content widths, a shortfall is taken from the room each column has
/// above its min-content width.
fn auto_widths(specs: &[Column], available: f32) -> Vec<f32> {
    let preferred = specs
        .iter()
        .map(|spec| {
            let wanted = match (spec.percent, spec.fixed) {
                (Some(percent), _) if available.is_finite() => available * percent / 100.0,
                (_, Some(fixed)) => fixed,
                _ => spec.maximum,
            };
            wanted.max(spec.minimum)
        })
        .collect::<Vec<_>>();
    if !available.is_finite() {
        return preferred;
    }
    let total = preferred.iter().sum::<f32>();
    let mut widths = preferred.clone();
    if total < available {
        let spare = available - total;
        let auto = |spec: &Column| spec.fixed.is_none() && spec.percent.is_none();
        let weight = |pick: &dyn Fn(&Column) -> bool| {
            specs
                .iter()
                .zip(&preferred)
                .filter(|(spec, _)| pick(spec))
                .map(|(_, width)| *width)
                .sum::<f32>()
        };
        let groups: [&dyn Fn(&Column) -> bool; 3] = [
            &auto,
            &|spec: &Column| spec.fixed.is_some() && spec.percent.is_none(),
            &|_: &Column| true,
        ];
        for pick in groups {
            let weights = weight(pick);
            if weights > 0.0 {
                for ((spec, width), base) in specs.iter().zip(widths.iter_mut()).zip(&preferred) {
                    if pick(spec) {
                        *width = base + spare * base / weights;
                    }
                }
                return widths;
            }
        }
        let count = widths.len().max(1) as f32;
        for width in &mut widths {
            *width += spare / count;
        }
    } else if total > available {
        let room = specs
            .iter()
            .zip(&preferred)
            .map(|(spec, width)| (width - spec.minimum).max(0.0))
            .sum::<f32>();
        let excess = (total - available).min(room);
        if room > 0.0 {
            for ((spec, width), base) in specs.iter().zip(widths.iter_mut()).zip(&preferred) {
                *width = base - excess * (base - spec.minimum).max(0.0) / room;
            }
        }
    }
    widths
}

fn group_height(group: &Node, rows: &[f32]) -> f32 {
    let content = rows.iter().sum::<f32>();
    let maximum = number(group, PropKey::MaxHeight)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(f32::INFINITY);
    content.min(maximum)
}

/// The table's intrinsic extent: its height at `available_width`, or the
/// sum of its columns' preferred widths.
#[allow(clippy::too_many_arguments)]
pub(super) fn intrinsic(
    children: &BTreeMap<u64, Vec<&Node>>,
    table: &Node,
    axis: Axis,
    available_width: f32,
    text_scale: f32,
    text_metrics: &TextMetrics,
    depth: usize,
) -> Result<f32, LayoutError> {
    let structure = structure(children, table);
    let width = match axis {
        Axis::Vertical => available_width,
        Axis::Horizontal => f32::INFINITY,
    };
    let geometry = geometry(
        children,
        table,
        &structure,
        width,
        text_scale,
        text_metrics,
        depth,
    )?;
    let (left, top, right, bottom) = geometry.insets;
    Ok(match axis {
        Axis::Horizontal => geometry.widths.iter().sum::<f32>() + left + right,
        Axis::Vertical => {
            let mut height = top + bottom;
            for entry in &structure.entries {
                height += match entry {
                    Entry::Row(index) => geometry.heights[*index],
                    Entry::Group(group, range) => {
                        group_height(group, &geometry.heights[range.clone()])
                    }
                };
            }
            height
        }
    })
}

/// The width `table` takes when laid out `available_width` wide: its
/// columns never go below their min-content widths, so it can be wider.
pub(super) fn used_width(
    children: &BTreeMap<u64, Vec<&Node>>,
    table: &Node,
    available_width: f32,
    text_scale: f32,
    text_metrics: &TextMetrics,
    depth: usize,
) -> Result<f32, LayoutError> {
    let structure = structure(children, table);
    let geometry = geometry(
        children,
        table,
        &structure,
        available_width,
        text_scale,
        text_metrics,
        depth,
    )?;
    let (left, _, right, _) = geometry.insets;
    Ok(geometry.widths.iter().sum::<f32>() + left + right)
}

/// Lays out the rows and cells of `table` inside its `frame`.
pub(super) fn layout(
    context: &LayoutContext<'_>,
    table: &Node,
    frame: Layout,
    depth: usize,
    output: &mut BTreeMap<u64, Layout>,
) -> Result<(), LayoutError> {
    let structure = structure(context.children, table);
    let geometry = geometry(
        context.children,
        table,
        &structure,
        frame.width,
        context.text_scale,
        context.text_metrics,
        depth,
    )?;
    let (left, top, _, _) = geometry.insets;
    let x = frame.x + left;
    let width = geometry.widths.iter().sum::<f32>();
    let mut y = frame.y + top;
    let place_row =
        |index: usize, y: f32, output: &mut BTreeMap<u64, Layout>| -> Result<f32, LayoutError> {
            let row = &structure.rows[index];
            let height = geometry.heights[index];
            output.insert(
                row.node.id,
                Layout {
                    x,
                    y,
                    width,
                    height,
                }
                .layout_units(),
            );
            let mut cursor = x;
            for (c, cell) in row.cells.iter().enumerate() {
                let cell_width = geometry.widths.get(c).copied().unwrap_or(0.0);
                layout_node(
                    context,
                    cell.id,
                    Layout {
                        x: cursor,
                        y,
                        width: cell_width,
                        height,
                    },
                    false,
                    depth + 2,
                    output,
                )?;
                cursor += cell_width;
            }
            Ok(height)
        };
    for entry in &structure.entries {
        match entry {
            Entry::Row(index) => y += place_row(*index, y, output)?,
            Entry::Group(group, range) => {
                let height = group_height(group, &geometry.heights[range.clone()]);
                output.insert(
                    group.id,
                    Layout {
                        x,
                        y,
                        width,
                        height,
                    }
                    .layout_units(),
                );
                let mut cursor = y;
                for index in range.clone() {
                    cursor += place_row(index, cursor, output)?;
                }
                y += height;
            }
        }
    }
    Ok(())
}
