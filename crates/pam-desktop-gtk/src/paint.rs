//! Box decoration painted with GSK render nodes (GPU composited by GTK).

use gtk::prelude::*;
use gtk::{gdk, graphene, gsk};
use pam_desktop_ui::color::Color;
use pam_desktop_ui::geometry::{Insets, Size};
use pam_desktop_ui::style::{BorderStyle, Decoration, Gradient, GradientShape, Radii};

pub fn rgba(color: Color) -> gdk::RGBA {
    let [r, g, b, a] = color.to_f32();
    gdk::RGBA::new(r, g, b, a)
}

pub fn bounds(size: Size) -> graphene::Rect {
    graphene::Rect::new(0.0, 0.0, size.width, size.height)
}

/// Rounded outline of a box inset by `insets` (corners shrink with it).
pub fn rounded(size: Size, radii: Radii, insets: Insets) -> gsk::RoundedRect {
    let rect = graphene::Rect::new(
        insets.left,
        insets.top,
        (size.width - insets.left - insets.right).max(0.0),
        (size.height - insets.top - insets.bottom).max(0.0),
    );
    let radii = radii.clamped(size);
    let corner = |radius: f32, horizontal: f32, vertical: f32| {
        graphene::Size::new((radius - horizontal).max(0.0), (radius - vertical).max(0.0))
    };
    gsk::RoundedRect::new(
        rect,
        corner(radii.top_left, insets.left, insets.top),
        corner(radii.top_right, insets.right, insets.top),
        corner(radii.bottom_right, insets.right, insets.bottom),
        corner(radii.bottom_left, insets.left, insets.bottom),
    )
}

fn stops(gradient: &Gradient) -> Vec<gsk::ColorStop> {
    let mut stops: Vec<gsk::ColorStop> = gradient
        .stops
        .iter()
        .map(|stop| gsk::ColorStop::new(stop.offset.clamp(0.0, 1.0), rgba(stop.color)))
        .collect();
    if stops.len() == 1 {
        let color = *stops[0].color();
        stops.push(gsk::ColorStop::new(1.0, color));
    }
    stops
}

fn append_gradient(snapshot: &gtk::Snapshot, size: Size, gradient: &Gradient) {
    let area = bounds(size);
    let stops = stops(gradient);
    if stops.len() < 2 {
        return;
    }
    match &gradient.shape {
        GradientShape::Linear { .. } | GradientShape::LinearPoints { .. } => {
            let Some((start, end)) = gradient.line(size) else {
                return;
            };
            let start = graphene::Point::new(start.x, start.y);
            let end = graphene::Point::new(end.x, end.y);
            if gradient.repeating {
                snapshot.append_repeating_linear_gradient(&area, &start, &end, &stops);
            } else {
                snapshot.append_linear_gradient(&area, &start, &end, &stops);
            }
        }
        GradientShape::Radial {
            center,
            radius_x,
            radius_y,
        } => {
            let center = graphene::Point::new(center.x * size.width, center.y * size.height);
            // Farthest-corner sizing, like the CSS default.
            let horizontal = (radius_x * size.width * std::f32::consts::SQRT_2).max(1.0);
            let vertical = (radius_y * size.height * std::f32::consts::SQRT_2).max(1.0);
            if gradient.repeating {
                snapshot.append_repeating_radial_gradient(
                    &area, &center, horizontal, vertical, 0.0, 1.0, &stops,
                );
            } else {
                snapshot
                    .append_radial_gradient(&area, &center, horizontal, vertical, 0.0, 1.0, &stops);
            }
        }
        GradientShape::Conic { center, angle } => {
            let center = graphene::Point::new(center.x * size.width, center.y * size.height);
            snapshot.append_conic_gradient(&area, &center, *angle, &stops);
        }
    }
}

/// Shadows, background color and gradients, and inset shadows.
pub fn paint_background(snapshot: &gtk::Snapshot, size: Size, decoration: &Decoration) {
    if size.is_empty() {
        return;
    }
    let outline = rounded(size, decoration.radii, Insets::ZERO);
    for shadow in decoration
        .shadows
        .iter()
        .filter(|shadow| !shadow.inset && !shadow.color.is_transparent())
    {
        snapshot.append_outset_shadow(
            &outline,
            &rgba(shadow.color),
            shadow.offset.x,
            shadow.offset.y,
            shadow.spread,
            shadow.blur,
        );
    }
    let has_fill = !decoration.background.is_transparent() || !decoration.gradients.is_empty();
    let has_inset = decoration.shadows.iter().any(|shadow| shadow.inset);
    if !has_fill && !has_inset {
        return;
    }
    let rounded_corners = !decoration.radii.is_zero();
    if rounded_corners {
        snapshot.push_rounded_clip(&outline);
    }
    if !decoration.background.is_transparent() {
        snapshot.append_color(&rgba(decoration.background), &bounds(size));
    }
    // The first gradient layer is the topmost one.
    for gradient in decoration.gradients.iter().rev() {
        append_gradient(snapshot, size, gradient);
    }
    if rounded_corners {
        snapshot.pop();
    }
    for shadow in decoration
        .shadows
        .iter()
        .filter(|shadow| shadow.inset && !shadow.color.is_transparent())
    {
        snapshot.append_inset_shadow(
            &outline,
            &rgba(shadow.color),
            shadow.offset.x,
            shadow.offset.y,
            shadow.spread,
            shadow.blur,
        );
    }
}

pub fn paint_border(snapshot: &gtk::Snapshot, size: Size, decoration: &Decoration) {
    let border = &decoration.border;
    if size.is_empty() || !border.has_width() {
        return;
    }
    let [left, top, right, bottom] = border.widths;
    let outline = rounded(size, decoration.radii, Insets::ZERO);

    if let Some(gradient) = &decoration.border_gradient {
        // The gradient shows only through the border ring.
        snapshot.push_mask(gsk::MaskMode::Alpha);
        let opaque = gdk::RGBA::new(0.0, 0.0, 0.0, 1.0);
        snapshot.append_border(
            &outline,
            &[top, right, bottom, left],
            &[opaque, opaque, opaque, opaque],
        );
        snapshot.pop();
        append_gradient(snapshot, size, gradient);
        snapshot.pop();
        return;
    }
    if !border.is_visible() {
        return;
    }

    if border.style == BorderStyle::Solid {
        let [left_color, top_color, right_color, bottom_color] = border.colors;
        snapshot.append_border(
            &outline,
            &[top, right, bottom, left],
            &[
                rgba(top_color),
                rgba(right_color),
                rgba(bottom_color),
                rgba(left_color),
            ],
        );
        return;
    }

    // Dashed and dotted borders follow the center line of the ring.
    let width = left.max(top).max(right).max(bottom);
    let half = width / 2.0;
    let center_line = rounded(size, decoration.radii, Insets::all(half));
    let builder = gsk::PathBuilder::new();
    builder.add_rounded_rect(&center_line);
    let path = builder.to_path();
    let stroke = gsk::Stroke::new(width);
    if border.style == BorderStyle::Dotted {
        stroke.set_line_cap(gsk::LineCap::Round);
        stroke.set_dash(&[0.0, width * 2.0]);
    } else {
        stroke.set_dash(&[width * 3.0, width * 2.0]);
    }
    let color = border
        .colors
        .iter()
        .copied()
        .find(|color| !color.is_transparent())
        .unwrap_or(Color::TRANSPARENT);
    snapshot.append_stroke(&path, &stroke, &rgba(color));
}

/// Rasterises a drawing surface with Cairo. `hairline` is the width of one
/// device pixel in logical units.
pub fn paint_drawing(
    context: &gtk::cairo::Context,
    commands: &[pam_desktop_ui::backend::DrawCommand],
    hairline: f64,
) {
    use gtk::cairo::{LineCap, LineJoin, Operator};
    use pam_desktop_ui::backend::DrawCommand;
    use std::f64::consts::{FRAC_PI_2, PI, TAU};

    let source = |color: pam_desktop_ui::color::Color| {
        let [red, green, blue, alpha] = color.to_f32();
        context.set_source_rgba(
            f64::from(red),
            f64::from(green),
            f64::from(blue),
            f64::from(alpha),
        );
    };
    let mut groups = 0_u32;
    for command in commands {
        // Cairo keeps going after an error; a failed command just draws nothing.
        match command {
            DrawCommand::Rect {
                rect,
                radius,
                color,
            } => {
                let (x, y, width, height) = (
                    f64::from(rect.x),
                    f64::from(rect.y),
                    f64::from(rect.width),
                    f64::from(rect.height),
                );
                let radius = f64::from(*radius).min(width / 2.0).min(height / 2.0);
                source(*color);
                if radius > 0.0 {
                    context.new_sub_path();
                    context.arc(x + width - radius, y + radius, radius, -FRAC_PI_2, 0.0);
                    context.arc(
                        x + width - radius,
                        y + height - radius,
                        radius,
                        0.0,
                        FRAC_PI_2,
                    );
                    context.arc(x + radius, y + height - radius, radius, FRAC_PI_2, PI);
                    context.arc(x + radius, y + radius, radius, PI, PI + FRAC_PI_2);
                    context.close_path();
                } else {
                    context.rectangle(x, y, width, height);
                }
                let _ = context.fill();
            }
            DrawCommand::Circle {
                center,
                radius,
                color,
            } => {
                source(*color);
                context.new_sub_path();
                context.arc(
                    f64::from(center.x),
                    f64::from(center.y),
                    f64::from(*radius),
                    0.0,
                    TAU,
                );
                let _ = context.fill();
            }
            DrawCommand::Line {
                from,
                to,
                width,
                color,
            } => {
                source(*color);
                context.set_line_cap(LineCap::Round);
                context.set_line_width(if *width > 0.0 {
                    f64::from(*width)
                } else {
                    hairline
                });
                context.move_to(f64::from(from.x), f64::from(from.y));
                context.line_to(f64::from(to.x), f64::from(to.y));
                let _ = context.stroke();
            }
            DrawCommand::Stroke {
                points,
                width,
                color,
                erase,
            } => {
                let Some(first) = points.first() else {
                    continue;
                };
                source(*color);
                context.set_operator(if *erase {
                    Operator::Clear
                } else {
                    Operator::Over
                });
                if points.len() == 1 {
                    context.new_sub_path();
                    context.arc(
                        f64::from(first.x),
                        f64::from(first.y),
                        f64::from(*width) / 2.0,
                        0.0,
                        TAU,
                    );
                    let _ = context.fill();
                } else {
                    context.set_line_cap(LineCap::Round);
                    context.set_line_join(LineJoin::Round);
                    context.set_line_width(f64::from(*width));
                    context.move_to(f64::from(first.x), f64::from(first.y));
                    for point in &points[1..] {
                        context.line_to(f64::from(point.x), f64::from(point.y));
                    }
                    let _ = context.stroke();
                }
                context.set_operator(Operator::Over);
            }
            DrawCommand::PushGroup { clip } => {
                let _ = context.save();
                context.rectangle(
                    f64::from(clip.x),
                    f64::from(clip.y),
                    f64::from(clip.width),
                    f64::from(clip.height),
                );
                context.clip();
                context.push_group();
                groups += 1;
            }
            DrawCommand::PopGroup => {
                if groups > 0 {
                    groups -= 1;
                    let _ = context.pop_group_to_source();
                    let _ = context.paint();
                    let _ = context.restore();
                }
            }
        }
    }
    // An unbalanced list must not leave groups open on the shared context.
    for _ in 0..groups {
        let _ = context.pop_group_to_source();
        let _ = context.paint();
        let _ = context.restore();
    }
}
