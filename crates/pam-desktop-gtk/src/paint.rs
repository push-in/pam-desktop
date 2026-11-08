//! Box decoration painted with GSK render nodes (GPU composited by GTK).

use gtk::prelude::*;
use gtk::{gdk, graphene, gsk};
use pam_desktop_ui::color::Color;
use pam_desktop_ui::geometry::{Insets, Size};
use pam_desktop_ui::style::{BorderStyle, Decoration, Gradient, GradientShape, Radii};

#[link(name = "pangocairo-1.0")]
unsafe extern "C" {
    fn pango_cairo_create_layout(
        context: *mut gtk::cairo::ffi::cairo_t,
    ) -> *mut gtk::pango::ffi::PangoLayout;
    fn pango_cairo_show_layout(
        context: *mut gtk::cairo::ffi::cairo_t,
        layout: *mut gtk::pango::ffi::PangoLayout,
    );
}

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
        if !crate::compat::push_alpha_mask(snapshot) {
            // GTK before 4.10 has no masks: Cairo fills the ring.
            let inner = rounded(
                size,
                decoration.radii,
                Insets {
                    left,
                    top,
                    right,
                    bottom,
                },
            );
            if let Some(pattern) = cairo_gradient(size, gradient) {
                let context = snapshot.append_cairo(&bounds(size));
                context.set_fill_rule(gtk::cairo::FillRule::EvenOdd);
                trace_rounded(&context, &outline);
                trace_rounded(&context, &inner);
                let _ = context.set_source(&pattern);
                let _ = context.fill();
            }
            return;
        }
        // The gradient shows only through the border ring.
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
    let dotted = border.style == BorderStyle::Dotted;
    let dash = if dotted {
        [0.0, width * 2.0]
    } else {
        [width * 3.0, width * 2.0]
    };
    let color = rgba(
        border
            .colors
            .iter()
            .copied()
            .find(|color| !color.is_transparent())
            .unwrap_or(Color::TRANSPARENT),
    );
    if crate::compat::stroke_rounded_rect(snapshot, &center_line, width, &dash, dotted, &color) {
        return;
    }
    // GTK before 4.14 strokes no paths: Cairo does.
    let context = snapshot.append_cairo(&bounds(size));
    trace_rounded(&context, &center_line);
    context.set_source_rgba(
        f64::from(color.red()),
        f64::from(color.green()),
        f64::from(color.blue()),
        f64::from(color.alpha()),
    );
    context.set_line_width(f64::from(width));
    if dotted {
        context.set_line_cap(gtk::cairo::LineCap::Round);
    }
    context.set_dash(&dash.map(f64::from), 0.0);
    let _ = context.stroke();
}

/// Adds the outline of `rect` to the Cairo path, clockwise from the top
/// left corner.
fn trace_rounded(context: &gtk::cairo::Context, rect: &gsk::RoundedRect) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let bounds = rect.bounds();
    let (x, y) = (f64::from(bounds.x()), f64::from(bounds.y()));
    let (width, height) = (f64::from(bounds.width()), f64::from(bounds.height()));
    let corners = rect.corner();
    // Corner centers, and the angle each quarter ellipse starts at.
    let arcs = [
        (x, y, 1.0, 1.0, PI),
        (x + width, y, -1.0, 1.0, -FRAC_PI_2),
        (x + width, y + height, -1.0, -1.0, 0.0),
        (x, y + height, 1.0, -1.0, FRAC_PI_2),
    ];
    context.new_sub_path();
    for (corner, (cx, cy, sx, sy, start)) in corners.iter().zip(arcs) {
        let (rx, ry) = (f64::from(corner.width()), f64::from(corner.height()));
        if rx <= 0.0 || ry <= 0.0 {
            context.line_to(cx, cy);
            continue;
        }
        context.save().ok();
        context.translate(cx + sx * rx, cy + sy * ry);
        context.scale(rx, ry);
        context.arc(0.0, 0.0, 1.0, start, start + FRAC_PI_2);
        context.restore().ok();
    }
    context.close_path();
}

/// `gradient` as a Cairo pattern over a box of `size`.
fn cairo_gradient(size: Size, gradient: &Gradient) -> Option<gtk::cairo::Pattern> {
    use gtk::cairo::{Extend, LinearGradient, Matrix, Mesh, RadialGradient};
    let stops = stops(gradient);
    if stops.len() < 2 {
        return None;
    }
    let add = |pattern: &gtk::cairo::Gradient| {
        for stop in &stops {
            let color = stop.color();
            pattern.add_color_stop_rgba(
                f64::from(stop.offset()),
                f64::from(color.red()),
                f64::from(color.green()),
                f64::from(color.blue()),
                f64::from(color.alpha()),
            );
        }
        pattern.set_extend(if gradient.repeating {
            Extend::Repeat
        } else {
            Extend::Pad
        });
    };
    match &gradient.shape {
        GradientShape::Linear { .. } | GradientShape::LinearPoints { .. } => {
            let (start, end) = gradient.line(size)?;
            let pattern = LinearGradient::new(
                f64::from(start.x),
                f64::from(start.y),
                f64::from(end.x),
                f64::from(end.y),
            );
            add(&pattern);
            Some(gtk::cairo::Pattern::clone(&pattern))
        }
        GradientShape::Radial {
            center,
            radius_x,
            radius_y,
        } => {
            let (cx, cy) = (
                f64::from(center.x * size.width),
                f64::from(center.y * size.height),
            );
            let horizontal = f64::from((radius_x * size.width * std::f32::consts::SQRT_2).max(1.0));
            let vertical = f64::from((radius_y * size.height * std::f32::consts::SQRT_2).max(1.0));
            let pattern = RadialGradient::new(cx, cy, 0.0, cx, cy, horizontal);
            // An ellipse: the circle stretched vertically.
            let ratio = horizontal / vertical;
            pattern.set_matrix(Matrix::new(1.0, 0.0, 0.0, ratio, 0.0, cy - cy * ratio));
            add(&pattern);
            Some(gtk::cairo::Pattern::clone(&pattern))
        }
        GradientShape::Conic { center, angle } => {
            // Cairo has no conic gradient: thin triangles around the center.
            let (cx, cy) = (
                f64::from(center.x * size.width),
                f64::from(center.y * size.height),
            );
            let reach = f64::from(size.width + size.height) * 2.0;
            let color_at = |offset: f32| {
                let after = stops.iter().position(|stop| stop.offset() >= offset);
                let (from, to) = match after {
                    Some(0) => (&stops[0], &stops[0]),
                    Some(index) => (&stops[index - 1], &stops[index]),
                    None => (&stops[stops.len() - 1], &stops[stops.len() - 1]),
                };
                let span = to.offset() - from.offset();
                let t = if span > 0.0 {
                    (offset - from.offset()) / span
                } else {
                    0.0
                };
                let (a, b) = (from.color(), to.color());
                let mix = |a: f32, b: f32| f64::from(a + (b - a) * t);
                [
                    mix(a.red(), b.red()),
                    mix(a.green(), b.green()),
                    mix(a.blue(), b.blue()),
                    mix(a.alpha(), b.alpha()),
                ]
            };
            let point = |offset: f32| {
                let radians = f64::from(*angle + offset * 360.0).to_radians();
                (cx + reach * radians.sin(), cy - reach * radians.cos())
            };
            let mesh = Mesh::new();
            const SEGMENTS: u16 = 72;
            for segment in 0..SEGMENTS {
                let (from, to) = (
                    f32::from(segment) / f32::from(SEGMENTS),
                    f32::from(segment + 1) / f32::from(SEGMENTS),
                );
                let (start, end) = (point(from), point(to));
                mesh.begin_patch();
                mesh.move_to(cx, cy);
                mesh.line_to(start.0, start.1);
                mesh.line_to(end.0, end.1);
                mesh.line_to(cx, cy);
                for (corner, offset) in [(0, from), (1, from), (2, to), (3, to)] {
                    let [r, g, b, a] = color_at(offset);
                    mesh.set_corner_color_rgba(gtk::cairo::MeshCorner::from(corner), r, g, b, a);
                }
                mesh.end_patch();
            }
            Some(gtk::cairo::Pattern::clone(&mesh))
        }
    }
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
            DrawCommand::Path {
                segments,
                fill,
                stroke,
                width,
            } => {
                use pam_desktop_ui::backend::{DrawPaint, DrawPathSegment};
                context.new_path();
                for segment in segments {
                    match segment {
                        DrawPathSegment::Move(point) => {
                            context.move_to(f64::from(point.x), f64::from(point.y))
                        }
                        DrawPathSegment::Line(point) => {
                            context.line_to(f64::from(point.x), f64::from(point.y))
                        }
                        DrawPathSegment::Cubic {
                            control1,
                            control2,
                            to,
                        } => context.curve_to(
                            f64::from(control1.x),
                            f64::from(control1.y),
                            f64::from(control2.x),
                            f64::from(control2.y),
                            f64::from(to.x),
                            f64::from(to.y),
                        ),
                        DrawPathSegment::Close => context.close_path(),
                    }
                }
                if let Some(fill) = fill {
                    match fill {
                        DrawPaint::Solid(color) => source(*color),
                        DrawPaint::LinearGradient {
                            from,
                            to,
                            start,
                            end,
                        } => {
                            let gradient = gtk::cairo::LinearGradient::new(
                                f64::from(from.x),
                                f64::from(from.y),
                                f64::from(to.x),
                                f64::from(to.y),
                            );
                            for (offset, color) in [(0.0, *start), (1.0, *end)] {
                                let [r, g, b, a] = color.to_f32().map(f64::from);
                                gradient.add_color_stop_rgba(offset, r, g, b, a);
                            }
                            let _ = context.set_source(&gradient);
                        }
                    }
                    let _ = context.fill_preserve();
                }
                if let Some(color) = stroke {
                    source(*color);
                    context.set_line_cap(LineCap::Butt);
                    context.set_line_join(LineJoin::Round);
                    context.set_line_width(if *width > 0.0 {
                        f64::from(*width)
                    } else {
                        hairline
                    });
                    let _ = context.stroke();
                } else {
                    context.new_path();
                }
            }
            DrawCommand::Text {
                text,
                baseline,
                font,
                color,
                align,
            } => {
                // Pango provides the same Unicode shaping as the other native text nodes.
                use gtk::glib::translate::{ToGlibPtr, from_glib_full};
                // SAFETY: live Cairo context; the returned owned GObject is released by Layout.
                let layout: gtk::pango::Layout =
                    unsafe { from_glib_full(pango_cairo_create_layout(context.to_raw_none())) };
                let mut description = gtk::pango::FontDescription::new();
                description.set_family(if font.family.is_empty() {
                    "Sans"
                } else {
                    &font.family
                });
                description.set_absolute_size(f64::from(font.size) * f64::from(gtk::pango::SCALE));
                description.set_weight(if font.weight >= 700 {
                    gtk::pango::Weight::Bold
                } else if font.weight >= 500 {
                    gtk::pango::Weight::Medium
                } else {
                    gtk::pango::Weight::Normal
                });
                layout.set_font_description(Some(&description));
                layout.set_text(text);
                let (width, _) = layout.size();
                let width = f64::from(width) / f64::from(gtk::pango::SCALE);
                let x = f64::from(baseline.x)
                    - match align {
                        pam_desktop_ui::text::TextAlign::Center => width / 2.0,
                        pam_desktop_ui::text::TextAlign::End => width,
                        _ => 0.0,
                    };
                let y = f64::from(baseline.y)
                    - f64::from(layout.baseline()) / f64::from(gtk::pango::SCALE);
                source(*color);
                context.move_to(x, y);
                // SAFETY: both objects remain alive throughout the native shaping/drawing call.
                unsafe { pango_cairo_show_layout(context.to_raw_none(), layout.to_glib_none().0) };
            }
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
