//! Box decoration, images and drawing surfaces painted with Core Graphics
//! inside a flipped view (the origin is the top-left corner, y points down).

use objc2_core_foundation::{CFRetained, CGAffineTransform, CGFloat, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBlendMode, CGColorSpace, CGContext, CGGradient, CGGradientDrawingOptions, CGImage, CGLineCap,
    CGLineJoin, CGMutablePath, CGPath, kCGColorSpaceSRGB,
};
use pam_desktop_ui::backend::{DrawCommand, ImageContent, ImageFit};
use pam_desktop_ui::color::Color;
use pam_desktop_ui::geometry::{Insets, Point, Rect, Size};
use pam_desktop_ui::style::{
    BorderStyle, Decoration, Gradient, GradientShape, GradientStop, Radii, Shadow,
};

use crate::util::{cg_color, cg_point, cg_rect};

/// Control point distance of the cubic that approximates a quarter circle.
const KAPPA: f32 = 0.552_284_8;
/// Stops a repeating gradient may expand to.
const MAX_REPEATED_STOPS: usize = 512;

/// Core Graphics takes shadow offsets in the context's base space, which no
/// transform reaches. The backing layer of a flipped view has flipped
/// contents, so that space already has its origin at the top-left corner
/// and a positive `y` falls below the shape. Should a system ever cast
/// these shadows upwards, this is the one switch.
const SHADOW_SPACE_Y_DOWN: bool = true;

/// A shadow offset of the engine (y points down) for Core Graphics.
pub fn shadow_offset(offset: Point) -> CGSize {
    CGSize::new(
        CGFloat::from(offset.x),
        CGFloat::from(if SHADOW_SPACE_Y_DOWN {
            offset.y
        } else {
            -offset.y
        }),
    )
}

/// Receives path segments: the current path of a context, or a path object.
pub trait PathSink {
    fn move_to(&mut self, x: f32, y: f32);
    fn line_to(&mut self, x: f32, y: f32);
    fn curve_to(&mut self, control_1: (f32, f32), control_2: (f32, f32), end: (f32, f32));
    fn close(&mut self);
}

struct ContextPath<'a>(&'a CGContext);

impl PathSink for ContextPath<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        CGContext::move_to_point(Some(self.0), CGFloat::from(x), CGFloat::from(y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        CGContext::add_line_to_point(Some(self.0), CGFloat::from(x), CGFloat::from(y));
    }

    fn curve_to(&mut self, control_1: (f32, f32), control_2: (f32, f32), end: (f32, f32)) {
        CGContext::add_curve_to_point(
            Some(self.0),
            CGFloat::from(control_1.0),
            CGFloat::from(control_1.1),
            CGFloat::from(control_2.0),
            CGFloat::from(control_2.1),
            CGFloat::from(end.0),
            CGFloat::from(end.1),
        );
    }

    fn close(&mut self) {
        CGContext::close_path(Some(self.0));
    }
}

struct ObjectPath(CFRetained<CGMutablePath>);

impl PathSink for ObjectPath {
    fn move_to(&mut self, x: f32, y: f32) {
        // SAFETY: a null transform means "no transform".
        unsafe {
            CGMutablePath::move_to_point(
                Some(&self.0),
                std::ptr::null(),
                CGFloat::from(x),
                CGFloat::from(y),
            )
        };
    }

    fn line_to(&mut self, x: f32, y: f32) {
        // SAFETY: as above.
        unsafe {
            CGMutablePath::add_line_to_point(
                Some(&self.0),
                std::ptr::null(),
                CGFloat::from(x),
                CGFloat::from(y),
            )
        };
    }

    fn curve_to(&mut self, control_1: (f32, f32), control_2: (f32, f32), end: (f32, f32)) {
        // SAFETY: as above.
        unsafe {
            CGMutablePath::add_curve_to_point(
                Some(&self.0),
                std::ptr::null(),
                CGFloat::from(control_1.0),
                CGFloat::from(control_1.1),
                CGFloat::from(control_2.0),
                CGFloat::from(control_2.1),
                CGFloat::from(end.0),
                CGFloat::from(end.1),
            );
        }
    }

    fn close(&mut self) {
        CGMutablePath::close_subpath(Some(&self.0));
    }
}

/// A rectangle whose corners are quarter ellipses: `(horizontal, vertical)`
/// radii for the top-left, top-right, bottom-right and bottom-left corners.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoundedRect {
    pub rect: Rect,
    pub corners: [(f32, f32); 4],
}

impl RoundedRect {
    /// Outline of a box of `size` inset by `insets` (corners shrink with it).
    pub fn new(size: Size, radii: Radii, insets: Insets) -> Self {
        let rect = Rect::new(
            insets.left,
            insets.top,
            (size.width - insets.horizontal()).max(0.0),
            (size.height - insets.vertical()).max(0.0),
        );
        let radii = radii.clamped(size);
        let corner = |radius: f32, horizontal: f32, vertical: f32| {
            ((radius - horizontal).max(0.0), (radius - vertical).max(0.0))
        };
        Self {
            rect,
            corners: [
                corner(radii.top_left, insets.left, insets.top),
                corner(radii.top_right, insets.right, insets.top),
                corner(radii.bottom_right, insets.right, insets.bottom),
                corner(radii.bottom_left, insets.left, insets.bottom),
            ],
        }
    }

    /// The outline grown by `amount` on every side (shadow spread). Sharp
    /// corners stay sharp, as in CSS.
    pub fn outset(&self, amount: f32) -> Self {
        let grow = |radius: f32| {
            if radius <= 0.0 && amount > 0.0 {
                0.0
            } else {
                (radius + amount).max(0.0)
            }
        };
        let rect = self.rect.outset(amount);
        Self {
            rect: Rect::new(rect.x, rect.y, rect.width.max(0.0), rect.height.max(0.0)),
            corners: self
                .corners
                .map(|(horizontal, vertical)| (grow(horizontal), grow(vertical))),
        }
    }

    pub fn translate(&self, dx: f32, dy: f32) -> Self {
        Self {
            rect: self.rect.translate(dx, dy),
            corners: self.corners,
        }
    }

    pub fn is_rect(&self) -> bool {
        self.corners
            .iter()
            .all(|(horizontal, vertical)| *horizontal <= 0.0 || *vertical <= 0.0)
    }

    /// The radius when every corner is the same circular arc.
    pub fn uniform_radius(&self) -> Option<f32> {
        let (radius, _) = self.corners[0];
        self.corners
            .iter()
            .all(|(horizontal, vertical)| *horizontal == radius && *vertical == radius)
            .then_some(radius)
    }

    pub fn add_to(&self, sink: &mut impl PathSink) {
        let Rect {
            x,
            y,
            width,
            height,
        } = self.rect;
        let (right, bottom) = (x + width, y + height);
        if self.is_rect() {
            sink.move_to(x, y);
            sink.line_to(right, y);
            sink.line_to(right, bottom);
            sink.line_to(x, bottom);
            sink.close();
            return;
        }
        // Radii never exceed half the box they were clamped to, but an
        // inset or outset copy is clamped again to stay a valid outline.
        let limit = |(horizontal, vertical): (f32, f32)| {
            (
                horizontal.min(width / 2.0).max(0.0),
                vertical.min(height / 2.0).max(0.0),
            )
        };
        let [top_left, top_right, bottom_right, bottom_left] = self.corners.map(limit);
        sink.move_to(x + top_left.0, y);
        sink.line_to(right - top_right.0, y);
        sink.curve_to(
            (right - top_right.0 * (1.0 - KAPPA), y),
            (right, y + top_right.1 * (1.0 - KAPPA)),
            (right, y + top_right.1),
        );
        sink.line_to(right, bottom - bottom_right.1);
        sink.curve_to(
            (right, bottom - bottom_right.1 * (1.0 - KAPPA)),
            (right - bottom_right.0 * (1.0 - KAPPA), bottom),
            (right - bottom_right.0, bottom),
        );
        sink.line_to(x + bottom_left.0, bottom);
        sink.curve_to(
            (x + bottom_left.0 * (1.0 - KAPPA), bottom),
            (x, bottom - bottom_left.1 * (1.0 - KAPPA)),
            (x, bottom - bottom_left.1),
        );
        sink.line_to(x, y + top_left.1);
        sink.curve_to(
            (x, y + top_left.1 * (1.0 - KAPPA)),
            (x + top_left.0 * (1.0 - KAPPA), y),
            (x + top_left.0, y),
        );
        sink.close();
    }

    /// Adds the outline to the current path of `context`.
    pub fn add_to_context(&self, context: &CGContext) {
        self.add_to(&mut ContextPath(context));
    }

    pub fn path(&self) -> CFRetained<CGPath> {
        let mut sink = ObjectPath(CGMutablePath::new());
        self.add_to(&mut sink);
        CFRetained::from(&**sink.0)
    }

    /// `frame` with this outline cut out of it (fill with the even-odd rule).
    pub fn cutout_path(&self, frame: Rect) -> CFRetained<CGPath> {
        let mut sink = ObjectPath(CGMutablePath::new());
        add_rect(&mut sink, frame);
        self.add_to(&mut sink);
        CFRetained::from(&**sink.0)
    }
}

fn add_rect(sink: &mut impl PathSink, rect: Rect) {
    sink.move_to(rect.x, rect.y);
    sink.line_to(rect.right(), rect.y);
    sink.line_to(rect.right(), rect.bottom());
    sink.line_to(rect.x, rect.bottom());
    sink.close();
}

fn set_fill(context: &CGContext, color: Color) {
    CGContext::set_fill_color_with_color(Some(context), Some(&cg_color(color)));
}

fn set_stroke(context: &CGContext, color: Color) {
    CGContext::set_stroke_color_with_color(Some(context), Some(&cg_color(color)));
}

fn srgb() -> Option<CFRetained<CGColorSpace>> {
    // SAFETY: the name is an immutable string exported by Core Graphics.
    CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
}

fn cg_gradient(stops: &[GradientStop]) -> Option<CFRetained<CGGradient>> {
    if stops.is_empty() {
        return None;
    }
    let mut components: Vec<CGFloat> = Vec::with_capacity(stops.len() * 4);
    let mut locations: Vec<CGFloat> = Vec::with_capacity(stops.len());
    for stop in stops {
        components.extend(stop.color.to_f32().map(CGFloat::from));
        locations.push(CGFloat::from(stop.offset.clamp(0.0, 1.0)));
    }
    // SAFETY: `components` holds four values per location and both buffers
    // outlive the call, which copies them.
    unsafe {
        CGGradient::with_color_components(
            srgb().as_deref(),
            components.as_ptr(),
            locations.as_ptr(),
            locations.len(),
        )
    }
}

/// Stops of a repeating gradient unrolled over `from..to` (in units of the
/// gradient line), renormalised to `0..1` over that range.
fn repeated_stops(stops: &[GradientStop], from: f32, to: f32) -> Vec<GradientStop> {
    let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
        return Vec::new();
    };
    let period = last.offset - first.offset;
    let span = to - from;
    if period <= 1e-4 || span <= 0.0 {
        return stops.to_vec();
    }
    let start = ((from - first.offset) / period).floor() as i32;
    let end = ((to - first.offset) / period).ceil() as i32;
    let mut repeated = Vec::new();
    for cycle in start..end.max(start.saturating_add(1)) {
        for stop in stops {
            if repeated.len() >= MAX_REPEATED_STOPS {
                return repeated;
            }
            let offset = stop.offset + cycle as f32 * period;
            repeated.push(GradientStop {
                color: stop.color,
                offset: ((offset - from) / span).clamp(0.0, 1.0),
            });
        }
    }
    repeated
}

/// Fills the current clip with `gradient`, laid out for a box of `size`.
fn paint_gradient(context: &CGContext, size: Size, gradient: &Gradient) {
    if gradient.stops.is_empty() || size.is_empty() {
        return;
    }
    let extend = CGGradientDrawingOptions::DrawsBeforeStartLocation
        | CGGradientDrawingOptions::DrawsAfterEndLocation;
    let corners = [
        Point::ZERO,
        Point::new(size.width, 0.0),
        Point::new(size.width, size.height),
        Point::new(0.0, size.height),
    ];
    match &gradient.shape {
        GradientShape::Linear { .. } | GradientShape::LinearPoints { .. } => {
            let Some((start, end)) = gradient.line(size) else {
                return;
            };
            let (dx, dy) = (end.x - start.x, end.y - start.y);
            let length_squared = dx * dx + dy * dy;
            if gradient.repeating && length_squared > 0.0 {
                // Cover the box: project its corners on the gradient line.
                let project = |corner: Point| {
                    ((corner.x - start.x) * dx + (corner.y - start.y) * dy) / length_squared
                };
                let from = corners
                    .iter()
                    .map(|corner| project(*corner))
                    .fold(f32::MAX, f32::min);
                let to = corners
                    .iter()
                    .map(|corner| project(*corner))
                    .fold(f32::MIN, f32::max);
                let Some(native) = cg_gradient(&repeated_stops(&gradient.stops, from, to)) else {
                    return;
                };
                let at = |t: f32| cg_point(Point::new(start.x + dx * t, start.y + dy * t));
                CGContext::draw_linear_gradient(
                    Some(context),
                    Some(&native),
                    at(from),
                    at(to),
                    extend,
                );
            } else if let Some(native) = cg_gradient(&gradient.stops) {
                CGContext::draw_linear_gradient(
                    Some(context),
                    Some(&native),
                    cg_point(start),
                    cg_point(end),
                    extend,
                );
            }
        }
        GradientShape::Radial {
            center,
            radius_x,
            radius_y,
        } => {
            let center = Point::new(center.x * size.width, center.y * size.height);
            // Farthest-corner sizing, like the CSS default.
            let horizontal = (radius_x * size.width * std::f32::consts::SQRT_2).max(1.0);
            let vertical = (radius_y * size.height * std::f32::consts::SQRT_2).max(1.0);
            // How many radii the farthest corner is away, in the space
            // where the ellipse is the unit circle.
            let reach = if gradient.repeating {
                corners
                    .iter()
                    .map(|corner| {
                        (((corner.x - center.x) / horizontal).powi(2)
                            + ((corner.y - center.y) / vertical).powi(2))
                        .sqrt()
                    })
                    .fold(1.0, f32::max)
            } else {
                1.0
            };
            let stops = if gradient.repeating {
                repeated_stops(&gradient.stops, 0.0, reach)
            } else {
                gradient.stops.clone()
            };
            let Some(native) = cg_gradient(&stops) else {
                return;
            };
            CGContext::save_g_state(Some(context));
            // Core Graphics draws circles: squash the space into the ellipse.
            CGContext::translate_ctm(
                Some(context),
                CGFloat::from(center.x),
                CGFloat::from(center.y),
            );
            CGContext::scale_ctm(Some(context), 1.0, CGFloat::from(vertical / horizontal));
            let origin = CGPoint::new(0.0, 0.0);
            CGContext::draw_radial_gradient(
                Some(context),
                Some(&native),
                origin,
                0.0,
                origin,
                CGFloat::from(horizontal * reach),
                extend,
            );
            CGContext::restore_g_state(Some(context));
        }
        GradientShape::Conic { center, angle } => {
            let center = Point::new(center.x * size.width, center.y * size.height);
            let radius = corners
                .iter()
                .map(|corner| corner.distance(center))
                .fold(0.0, f32::max)
                + 1.0;
            // Core Graphics has no conic gradient before macOS 14: a fan of
            // one-degree wedges, unsmoothed so their edges do not show.
            const WEDGES: u32 = 360;
            CGContext::save_g_state(Some(context));
            CGContext::set_should_antialias(Some(context), false);
            for wedge in 0..WEDGES {
                let turn =
                    |index: u32| (angle - 90.0 + index as f32 * 360.0 / WEDGES as f32).to_radians();
                let (from, to) = (turn(wedge), turn(wedge + 1) + 0.005);
                set_fill(
                    context,
                    gradient.color_at((wedge as f32 + 0.5) / WEDGES as f32),
                );
                let mut path = ContextPath(context);
                CGContext::begin_path(Some(context));
                path.move_to(center.x, center.y);
                path.line_to(
                    center.x + radius * from.cos(),
                    center.y + radius * from.sin(),
                );
                path.line_to(center.x + radius * to.cos(), center.y + radius * to.sin());
                path.close();
                CGContext::fill_path(Some(context));
            }
            CGContext::restore_g_state(Some(context));
        }
    }
}

/// An inset shadow: the blurred edge of a frame around the box, seen
/// through the box. The frame itself lies outside the clip.
fn paint_inset_shadow(context: &CGContext, outline: &RoundedRect, shadow: &Shadow) {
    let hole = outline.outset(-shadow.spread);
    let reach = shadow.blur * 2.0
        + shadow.offset.x.abs()
        + shadow.offset.y.abs()
        + shadow.spread.abs()
        + 8.0;
    CGContext::save_g_state(Some(context));
    CGContext::begin_path(Some(context));
    outline.add_to_context(context);
    CGContext::clip(Some(context));
    CGContext::set_shadow_with_color(
        Some(context),
        shadow_offset(shadow.offset),
        CGFloat::from(shadow.blur.max(0.0)),
        Some(&cg_color(shadow.color)),
    );
    // The frame casts at full strength; the shadow color carries the alpha.
    set_fill(
        context,
        Color::rgba(shadow.color.r, shadow.color.g, shadow.color.b, 255),
    );
    CGContext::begin_path(Some(context));
    let mut path = ContextPath(context);
    add_rect(&mut path, outline.rect.outset(reach));
    hole.add_to(&mut path);
    CGContext::eo_fill_path(Some(context));
    CGContext::restore_g_state(Some(context));
}

/// Background color and gradients, and inset shadows. Outer shadows are
/// composited by the view's shadow layers, never painted here.
pub fn paint_background(context: &CGContext, size: Size, decoration: &Decoration) {
    if size.is_empty() {
        return;
    }
    let outline = RoundedRect::new(size, decoration.radii, Insets::ZERO);
    if !decoration.background.is_transparent() || !decoration.gradients.is_empty() {
        CGContext::save_g_state(Some(context));
        CGContext::begin_path(Some(context));
        outline.add_to_context(context);
        CGContext::clip(Some(context));
        if !decoration.background.is_transparent() {
            set_fill(context, decoration.background);
            CGContext::fill_rect(Some(context), cg_rect(Rect::from_size(size)));
        }
        // The first gradient layer is the topmost one.
        for gradient in decoration.gradients.iter().rev() {
            paint_gradient(context, size, gradient);
        }
        CGContext::restore_g_state(Some(context));
    }
    for shadow in decoration
        .shadows
        .iter()
        .filter(|shadow| shadow.inset && !shadow.color.is_transparent())
    {
        paint_inset_shadow(context, &outline, shadow);
    }
}

pub fn paint_border(context: &CGContext, size: Size, decoration: &Decoration) {
    let border = &decoration.border;
    if size.is_empty() || !border.has_width() {
        return;
    }
    let [left, top, right, bottom] = border.widths;
    let outline = RoundedRect::new(size, decoration.radii, Insets::ZERO);
    let inner = RoundedRect::new(
        size,
        decoration.radii,
        Insets::new(left, top, right, bottom),
    );
    // The ring between the border box and the padding box.
    let clip_to_ring = || {
        CGContext::begin_path(Some(context));
        outline.add_to_context(context);
        inner.add_to_context(context);
        CGContext::eo_clip(Some(context));
    };

    if let Some(gradient) = &decoration.border_gradient {
        CGContext::save_g_state(Some(context));
        clip_to_ring();
        paint_gradient(context, size, gradient);
        CGContext::restore_g_state(Some(context));
        return;
    }
    if !border.is_visible() {
        return;
    }

    if border.style == BorderStyle::Solid {
        CGContext::save_g_state(Some(context));
        clip_to_ring();
        let bounds = Rect::from_size(size);
        if border.colors.iter().all(|color| *color == border.colors[0]) {
            set_fill(context, border.colors[0]);
            CGContext::fill_rect(Some(context), cg_rect(bounds));
        } else {
            // Each side owns the trapezoid between its outer edge and the
            // matching edge of the padding box (corners split diagonally).
            let (inner_left, inner_top, inner_right, inner_bottom) = (
                inner.rect.x,
                inner.rect.y,
                inner.rect.right(),
                inner.rect.bottom(),
            );
            let sides = [
                (
                    border.colors[0],
                    left,
                    [
                        (0.0, 0.0),
                        (inner_left, inner_top),
                        (inner_left, inner_bottom),
                        (0.0, size.height),
                    ],
                ),
                (
                    border.colors[1],
                    top,
                    [
                        (0.0, 0.0),
                        (size.width, 0.0),
                        (inner_right, inner_top),
                        (inner_left, inner_top),
                    ],
                ),
                (
                    border.colors[2],
                    right,
                    [
                        (size.width, 0.0),
                        (size.width, size.height),
                        (inner_right, inner_bottom),
                        (inner_right, inner_top),
                    ],
                ),
                (
                    border.colors[3],
                    bottom,
                    [
                        (0.0, size.height),
                        (inner_left, inner_bottom),
                        (inner_right, inner_bottom),
                        (size.width, size.height),
                    ],
                ),
            ];
            for (color, width, corners) in sides {
                if width <= 0.0 || color.is_transparent() {
                    continue;
                }
                set_fill(context, color);
                CGContext::begin_path(Some(context));
                let mut path = ContextPath(context);
                path.move_to(corners[0].0, corners[0].1);
                for corner in &corners[1..] {
                    path.line_to(corner.0, corner.1);
                }
                path.close();
                CGContext::fill_path(Some(context));
            }
        }
        CGContext::restore_g_state(Some(context));
        return;
    }

    // Dashed and dotted borders follow the center line of the ring.
    let width = left.max(top).max(right).max(bottom);
    let center_line = RoundedRect::new(size, decoration.radii, Insets::all(width / 2.0));
    let color = border
        .colors
        .iter()
        .copied()
        .find(|color| !color.is_transparent())
        .unwrap_or(Color::TRANSPARENT);
    let dashes: [CGFloat; 2] = if border.style == BorderStyle::Dotted {
        [0.0, CGFloat::from(width * 2.0)]
    } else {
        [CGFloat::from(width * 3.0), CGFloat::from(width * 2.0)]
    };
    CGContext::save_g_state(Some(context));
    set_stroke(context, color);
    CGContext::set_line_width(Some(context), CGFloat::from(width));
    if border.style == BorderStyle::Dotted {
        CGContext::set_line_cap(Some(context), CGLineCap::Round);
    }
    // SAFETY: `dashes` holds the two lengths the count announces.
    unsafe { CGContext::set_line_dash(Some(context), 0.0, dashes.as_ptr(), dashes.len()) };
    CGContext::begin_path(Some(context));
    center_line.add_to_context(context);
    CGContext::stroke_path(Some(context));
    CGContext::restore_g_state(Some(context));
}

/// Clips the context to the rounded padding box of a decoration.
pub fn clip_to_padding_box(context: &CGContext, size: Size, decoration: &Decoration) {
    let [left, top, right, bottom] = decoration.border.widths;
    CGContext::begin_path(Some(context));
    RoundedRect::new(
        size,
        decoration.radii,
        Insets::new(left, top, right, bottom),
    )
    .add_to_context(context);
    CGContext::clip(Some(context));
}

/// Draws `image` upright in `target` (Core Graphics images have their
/// origin at the bottom-left, the view at the top-left).
fn draw_upright(context: &CGContext, image: &CGImage, target: Rect, tiled: bool) {
    CGContext::save_g_state(Some(context));
    CGContext::translate_ctm(
        Some(context),
        CGFloat::from(target.x),
        CGFloat::from(target.y + target.height),
    );
    CGContext::scale_ctm(Some(context), 1.0, -1.0);
    let rect = CGRect::new(
        CGPoint::new(0.0, 0.0),
        CGSize::new(CGFloat::from(target.width), CGFloat::from(target.height)),
    );
    if tiled {
        CGContext::draw_tiled_image(Some(context), rect, Some(image));
    } else {
        CGContext::draw_image(Some(context), rect, Some(image));
    }
    CGContext::restore_g_state(Some(context));
}

pub fn paint_image(
    context: &CGContext,
    size: Size,
    decoration: &Decoration,
    content: &ImageContent,
    image: &CGImage,
) {
    let insets = content.insets;
    let area = Rect::new(
        insets.left,
        insets.top,
        (size.width - insets.horizontal()).max(0.0),
        (size.height - insets.vertical()).max(0.0),
    );
    let natural = Size::new(
        CGImage::width(Some(image)) as f32,
        CGImage::height(Some(image)) as f32,
    );
    if area.is_empty() || natural.is_empty() || content.opacity <= 0.0 {
        return;
    }
    let scale = match content.fit {
        ImageFit::Cover => (area.width / natural.width).max(area.height / natural.height),
        ImageFit::Contain => (area.width / natural.width).min(area.height / natural.height),
        _ => 1.0,
    };
    let target = match content.fit {
        ImageFit::Fill => area,
        ImageFit::Repeat => Rect::new(area.x, area.y, natural.width, natural.height),
        _ => {
            let (width, height) = (natural.width * scale, natural.height * scale);
            Rect::new(
                area.x + (area.width - width) / 2.0,
                area.y + (area.height - height) / 2.0,
                width,
                height,
            )
        }
    };
    let tiled = content.fit == ImageFit::Repeat;

    CGContext::save_g_state(Some(context));
    // Images never paint outside their (rounded) box.
    CGContext::begin_path(Some(context));
    RoundedRect::new(size, decoration.radii, insets).add_to_context(context);
    CGContext::clip(Some(context));
    let grouped = content.opacity < 1.0 && (content.tint.is_some() || content.overlay.is_some());
    if content.opacity < 1.0 {
        CGContext::set_alpha(Some(context), CGFloat::from(content.opacity));
    }
    if grouped {
        // The image, its tint and its overlay fade as one.
        // SAFETY: no auxiliary dictionary is passed.
        unsafe { CGContext::begin_transparency_layer(Some(context), None) };
    }
    match content.tint {
        Some(tint) => {
            // The image is only a shape: its opaque pixels take the tint.
            // SAFETY: no auxiliary dictionary is passed.
            unsafe { CGContext::begin_transparency_layer(Some(context), None) };
            draw_upright(context, image, target, tiled);
            CGContext::set_blend_mode(Some(context), CGBlendMode::SourceIn);
            set_fill(context, tint);
            CGContext::fill_rect(Some(context), cg_rect(area));
            CGContext::end_transparency_layer(Some(context));
        }
        None => draw_upright(context, image, target, tiled),
    }
    if let Some(overlay) = content.overlay {
        CGContext::set_blend_mode(Some(context), CGBlendMode::Normal);
        set_fill(context, overlay);
        CGContext::fill_rect(Some(context), cg_rect(area));
    }
    if grouped {
        CGContext::end_transparency_layer(Some(context));
    }
    CGContext::restore_g_state(Some(context));
}

/// Rasterises a drawing surface. `hairline` is the width of one device
/// pixel in logical units.
pub fn paint_drawing(context: &CGContext, commands: &[DrawCommand], hairline: f32) {
    let mut groups = 0_u32;
    let close_group = |context: &CGContext| {
        CGContext::end_transparency_layer(Some(context));
        CGContext::restore_g_state(Some(context));
    };
    for command in commands {
        match command {
            DrawCommand::Rect {
                rect,
                radius,
                color,
            } => {
                set_fill(context, *color);
                let radius = radius.min(rect.width / 2.0).min(rect.height / 2.0).max(0.0);
                CGContext::begin_path(Some(context));
                RoundedRect {
                    rect: *rect,
                    corners: [(radius, radius); 4],
                }
                .add_to_context(context);
                CGContext::fill_path(Some(context));
            }
            DrawCommand::Circle {
                center,
                radius,
                color,
            } => {
                set_fill(context, *color);
                let bounds = Rect::new(
                    center.x - radius,
                    center.y - radius,
                    radius * 2.0,
                    radius * 2.0,
                );
                CGContext::begin_path(Some(context));
                CGContext::add_ellipse_in_rect(Some(context), cg_rect(bounds));
                CGContext::fill_path(Some(context));
            }
            DrawCommand::Line {
                from,
                to,
                width,
                color,
            } => {
                set_stroke(context, *color);
                CGContext::set_line_cap(Some(context), CGLineCap::Round);
                CGContext::set_line_width(
                    Some(context),
                    CGFloat::from(if *width > 0.0 { *width } else { hairline }),
                );
                CGContext::begin_path(Some(context));
                CGContext::move_to_point(
                    Some(context),
                    CGFloat::from(from.x),
                    CGFloat::from(from.y),
                );
                CGContext::add_line_to_point(
                    Some(context),
                    CGFloat::from(to.x),
                    CGFloat::from(to.y),
                );
                CGContext::stroke_path(Some(context));
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
                // Erasing clears whatever the current group (or the view) holds.
                let color = if *erase { Color::BLACK } else { *color };
                CGContext::set_blend_mode(
                    Some(context),
                    if *erase {
                        CGBlendMode::Clear
                    } else {
                        CGBlendMode::Normal
                    },
                );
                CGContext::begin_path(Some(context));
                if points.len() == 1 {
                    set_fill(context, color);
                    let bounds =
                        Rect::new(first.x - width / 2.0, first.y - width / 2.0, *width, *width);
                    CGContext::add_ellipse_in_rect(Some(context), cg_rect(bounds));
                    CGContext::fill_path(Some(context));
                } else {
                    set_stroke(context, color);
                    CGContext::set_line_cap(Some(context), CGLineCap::Round);
                    CGContext::set_line_join(Some(context), CGLineJoin::Round);
                    CGContext::set_line_width(Some(context), CGFloat::from(*width));
                    CGContext::move_to_point(
                        Some(context),
                        CGFloat::from(first.x),
                        CGFloat::from(first.y),
                    );
                    for point in &points[1..] {
                        CGContext::add_line_to_point(
                            Some(context),
                            CGFloat::from(point.x),
                            CGFloat::from(point.y),
                        );
                    }
                    CGContext::stroke_path(Some(context));
                }
                CGContext::set_blend_mode(Some(context), CGBlendMode::Normal);
            }
            DrawCommand::PushGroup { clip } => {
                CGContext::save_g_state(Some(context));
                CGContext::clip_to_rect(Some(context), cg_rect(*clip));
                // SAFETY: no auxiliary dictionary is passed.
                unsafe { CGContext::begin_transparency_layer(Some(context), None) };
                groups += 1;
            }
            DrawCommand::PopGroup => {
                if groups > 0 {
                    groups -= 1;
                    close_group(context);
                }
            }
        }
    }
    // An unbalanced list must not leave groups open on the view's context.
    for _ in 0..groups {
        close_group(context);
    }
}

/// The affine transform Core Graphics and Core Animation use.
pub fn cg_affine(transform: &pam_desktop_ui::geometry::Affine) -> CGAffineTransform {
    CGAffineTransform {
        a: CGFloat::from(transform.a),
        b: CGFloat::from(transform.b),
        c: CGFloat::from(transform.c),
        d: CGFloat::from(transform.d),
        tx: CGFloat::from(transform.tx),
        ty: CGFloat::from(transform.ty),
    }
}
