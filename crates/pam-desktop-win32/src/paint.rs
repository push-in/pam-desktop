//! Box decoration painted with Direct2D: shadows, backgrounds, gradients,
//! per-side borders with per-corner radii, images and drawing lists.

use std::collections::HashMap;
use std::mem::ManuallyDrop;

use pam_desktop_ui::backend::{DrawCommand, ImageContent, ImageFit};
use pam_desktop_ui::color::Color;
use pam_desktop_ui::geometry::{Insets, Point, Rect, Size};
use pam_desktop_ui::style::{BorderStyle, Decoration, Gradient, GradientShape, Radii, Shadow};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_F, D2D1_COMPOSITE_MODE_SOURCE_OVER, D2D1_FIGURE_BEGIN_FILLED,
    D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN,
    D2D1_FILL_MODE_ALTERNATE, D2D1_GRADIENT_STOP,
};
use windows::Win32::Graphics::Direct2D::{
    CLSID_D2D1ColorMatrix, CLSID_D2D1GaussianBlur, CLSID_D2D1Shadow, D2D1_ANTIALIAS_MODE_ALIASED,
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_SMALL,
    D2D1_BITMAP_BRUSH_PROPERTIES1, D2D1_BRUSH_PROPERTIES, D2D1_BUFFER_PRECISION_8BPC_UNORM,
    D2D1_CAP_STYLE_FLAT, D2D1_CAP_STYLE_ROUND, D2D1_COLOR_INTERPOLATION_MODE_PREMULTIPLIED,
    D2D1_COLOR_SPACE_SRGB, D2D1_COLORMATRIX_PROP_COLOR_MATRIX, D2D1_COMBINE_MODE_INTERSECT,
    D2D1_DASH_STYLE_CUSTOM, D2D1_ELLIPSE, D2D1_EXTEND_MODE_CLAMP, D2D1_EXTEND_MODE_WRAP,
    D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
    D2D1_INTERPOLATION_MODE_LINEAR, D2D1_LAYER_OPTIONS1_NONE, D2D1_LAYER_PARAMETERS1,
    D2D1_LINE_JOIN_MITER, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_PRIMITIVE_BLEND_COPY,
    D2D1_PRIMITIVE_BLEND_SOURCE_OVER, D2D1_PROPERTY_TYPE_FLOAT, D2D1_PROPERTY_TYPE_MATRIX_5X4,
    D2D1_PROPERTY_TYPE_VECTOR4, D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES, D2D1_ROUNDED_RECT,
    D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION, D2D1_SHADOW_PROP_COLOR, D2D1_STROKE_STYLE_PROPERTIES,
    D2D1_SWEEP_DIRECTION_CLOCKWISE, ID2D1Bitmap1, ID2D1Brush, ID2D1DeviceContext, ID2D1Factory1,
    ID2D1Geometry, ID2D1Image, ID2D1Layer, ID2D1StrokeStyle,
};
use windows::core::{Interface, Result};
use windows_numerics::Matrix3x2;

use crate::gfx::{Device, Graphics};
use crate::util::{color_f, matrix, point_vec, rect_f, vec2};

/// Everything Direct2D calls "infinite" for layer bounds.
pub const INFINITE: D2D_RECT_F = D2D_RECT_F {
    left: -f32::MAX,
    top: -f32::MAX,
    right: f32::MAX,
    bottom: f32::MAX,
};

/// Radii of the four corners of a box, each an ellipse (x, y).
type Corners = [(f32, f32); 4];

/// An outline Direct2D can fill: the cheap primitives when they fit, a
/// path geometry for per-corner radii.
pub enum Shape {
    Rect(Rect),
    Rounded(D2D1_ROUNDED_RECT),
    Path(ID2D1Geometry),
}

/// Corner radii of a box of `size` inset by `insets` (corners shrink with it).
fn corners(size: Size, radii: Radii, insets: Insets) -> (Rect, Corners) {
    let rect = Rect::new(
        insets.left,
        insets.top,
        (size.width - insets.horizontal()).max(0.0),
        (size.height - insets.vertical()).max(0.0),
    );
    let radii = radii.clamped(size);
    let corner = |radius: f32, horizontal: f32, vertical: f32| {
        let (x, y) = ((radius - horizontal).max(0.0), (radius - vertical).max(0.0));
        // A corner flattened on one axis is square. Adjacent radii were
        // already fitted to the box by `clamped`, and shrink with the insets.
        if x <= 0.0 || y <= 0.0 {
            (0.0, 0.0)
        } else {
            (x.min(rect.width), y.min(rect.height))
        }
    };
    let corners = [
        corner(radii.top_left, insets.left, insets.top),
        corner(radii.top_right, insets.right, insets.top),
        corner(radii.bottom_right, insets.right, insets.bottom),
        corner(radii.bottom_left, insets.left, insets.bottom),
    ];
    (rect, corners)
}

fn path_geometry(factory: &ID2D1Factory1, rect: Rect, corners: Corners) -> Result<ID2D1Geometry> {
    // SAFETY: COM calls on live objects; the sink is closed before the geometry is used.
    unsafe {
        let path = factory.CreatePathGeometry()?;
        let sink = path.Open()?;
        let [top_left, top_right, bottom_right, bottom_left] = corners;
        let arc = |to: Point, radius: (f32, f32)| {
            if radius.0 > 0.0 && radius.1 > 0.0 {
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: point_vec(to),
                    size: D2D_SIZE_F {
                        width: radius.0,
                        height: radius.1,
                    },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                    arcSize: D2D1_ARC_SIZE_SMALL,
                });
            }
        };
        let (left, top, right, bottom) = (rect.x, rect.y, rect.right(), rect.bottom());
        sink.BeginFigure(vec2(left + top_left.0, top), D2D1_FIGURE_BEGIN_FILLED);
        sink.AddLine(vec2(right - top_right.0, top));
        arc(Point::new(right, top + top_right.1), top_right);
        sink.AddLine(vec2(right, bottom - bottom_right.1));
        arc(Point::new(right - bottom_right.0, bottom), bottom_right);
        sink.AddLine(vec2(left + bottom_left.0, bottom));
        arc(Point::new(left, bottom - bottom_left.1), bottom_left);
        sink.AddLine(vec2(left, top + top_left.1));
        arc(Point::new(left + top_left.0, top), top_left);
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close()?;
        path.cast()
    }
}

/// Rounded outline of a box inset by `insets`.
pub fn rounded(factory: &ID2D1Factory1, size: Size, radii: Radii, insets: Insets) -> Shape {
    let (rect, corners) = corners(size, radii, insets);
    if corners.iter().all(|corner| *corner == (0.0, 0.0)) {
        return Shape::Rect(rect);
    }
    let first = corners[0];
    if first.0 == first.1 && corners.iter().all(|corner| *corner == first) {
        return Shape::Rounded(D2D1_ROUNDED_RECT {
            rect: rect_f(rect),
            radiusX: first.0,
            radiusY: first.1,
        });
    }
    match path_geometry(factory, rect, corners) {
        Ok(path) => Shape::Path(path),
        Err(_) => Shape::Rect(rect),
    }
}

impl Shape {
    pub fn fill(&self, context: &ID2D1DeviceContext, brush: &ID2D1Brush) {
        // SAFETY: COM calls on live objects; the structures passed outlive the calls.
        unsafe {
            match self {
                Self::Rect(rect) => context.FillRectangle(&rect_f(*rect), brush),
                Self::Rounded(rounded) => context.FillRoundedRectangle(rounded, brush),
                Self::Path(path) => context.FillGeometry(path, brush, None::<&ID2D1Brush>),
            }
        }
    }

    pub fn stroke(
        &self,
        context: &ID2D1DeviceContext,
        brush: &ID2D1Brush,
        width: f32,
        style: Option<&ID2D1StrokeStyle>,
    ) {
        // SAFETY: as in `fill`.
        unsafe {
            match self {
                Self::Rect(rect) => context.DrawRectangle(&rect_f(*rect), brush, width, style),
                Self::Rounded(rounded) => {
                    context.DrawRoundedRectangle(rounded, brush, width, style)
                }
                Self::Path(path) => context.DrawGeometry(path, brush, width, style),
            }
        }
    }

    pub fn geometry(&self, factory: &ID2D1Factory1) -> Option<ID2D1Geometry> {
        // SAFETY: COM calls on the live factory; the structures passed outlive the calls.
        unsafe {
            match self {
                Self::Rect(rect) => factory
                    .CreateRectangleGeometry(&rect_f(*rect))
                    .ok()?
                    .cast()
                    .ok(),
                Self::Rounded(rounded) => factory
                    .CreateRoundedRectangleGeometry(rounded)
                    .ok()?
                    .cast()
                    .ok(),
                Self::Path(path) => Some(path.clone()),
            }
        }
    }

    pub fn is_rect(&self) -> Option<Rect> {
        match self {
            Self::Rect(rect) => Some(*rect),
            _ => None,
        }
    }
}

/// Opens a layer; the caller pops it with `PopLayer`.
pub fn push_layer(
    context: &ID2D1DeviceContext,
    bounds: D2D_RECT_F,
    mask: Option<&ID2D1Geometry>,
    opacity: f32,
) {
    let mut parameters = D2D1_LAYER_PARAMETERS1 {
        contentBounds: bounds,
        geometricMask: ManuallyDrop::new(mask.cloned()),
        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
        maskTransform: Matrix3x2::identity(),
        opacity,
        opacityBrush: ManuallyDrop::new(None),
        layerOptions: D2D1_LAYER_OPTIONS1_NONE,
    };
    // SAFETY: the parameters outlive the call; Direct2D takes its own
    // reference to the mask, so ours is released right after.
    unsafe {
        context.PushLayer(&parameters, None::<&ID2D1Layer>);
        ManuallyDrop::drop(&mut parameters.geometricMask);
    }
}

/// Clips to `shape`: an axis-aligned clip when it is a rectangle, a layer
/// otherwise. Returned so the matching pop can be issued.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Clip {
    Axis,
    Layer,
}

pub fn push_clip(context: &ID2D1DeviceContext, factory: &ID2D1Factory1, shape: &Shape) -> Clip {
    let mut transform = Matrix3x2::identity();
    // SAFETY: the out pointer refers to a live local.
    unsafe { context.GetTransform(&mut transform) };
    // An axis-aligned clip under a rotation or skew would clip to the
    // bounding box of the rectangle: that case takes the layer path.
    let upright = transform.M12 == 0.0 && transform.M21 == 0.0;
    if let Some(rect) = shape.is_rect().filter(|_| upright) {
        // SAFETY: the rectangle outlives the call.
        unsafe { context.PushAxisAlignedClip(&rect_f(rect), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
        return Clip::Axis;
    }
    match shape.geometry(factory) {
        Some(geometry) => {
            push_layer(context, INFINITE, Some(&geometry), 1.0);
            Clip::Layer
        }
        None => {
            // SAFETY: as above.
            unsafe { context.PushAxisAlignedClip(&INFINITE, D2D1_ANTIALIAS_MODE_ALIASED) };
            Clip::Axis
        }
    }
}

pub fn pop_clip(context: &ID2D1DeviceContext, clip: Clip) {
    // SAFETY: pops exactly what `push_clip` pushed.
    unsafe {
        match clip {
            Clip::Axis => context.PopAxisAlignedClip(),
            Clip::Layer => context.PopLayer(),
        }
    }
}

/// Direct2D resources derived from a view's decoration and drawing list,
/// valid for one size and one device generation.
#[derive(Default)]
pub struct PaintCache {
    size: Size,
    generation: u64,
    valid: bool,
    gradients: Vec<Option<ID2D1Brush>>,
    border_gradient: Option<ID2D1Brush>,
    /// The border ring, or one geometry per side when their colors differ.
    ring: Option<ID2D1Geometry>,
    sides: [Option<ID2D1Geometry>; 4],
    dash: Option<ID2D1StrokeStyle>,
    /// A drawing list that needed its own surface (it erases), rasterized,
    /// with the size, scale and device generation it was made for.
    pub drawing: Option<(ID2D1Bitmap1, Size, f32, u64)>,
}

impl PaintCache {
    /// Forgets what depends on the decoration.
    pub fn invalidate(&mut self) {
        self.valid = false;
    }

    pub fn invalidate_drawing(&mut self) {
        self.drawing = None;
    }

    fn prepare(
        &mut self,
        graphics: &Graphics,
        device: &Device,
        size: Size,
        decoration: &Decoration,
    ) {
        if self.valid && self.size == size && self.generation == graphics.generation {
            return;
        }
        self.size = size;
        self.generation = graphics.generation;
        self.valid = true;
        self.gradients.clear();
        self.gradients.extend(
            decoration
                .gradients
                .iter()
                .map(|gradient| gradient_brush(&device.context, size, gradient)),
        );
        self.border_gradient = decoration
            .border_gradient
            .as_ref()
            .and_then(|gradient| gradient_brush(&device.context, size, gradient));
        self.ring = None;
        self.sides = [None, None, None, None];
        self.dash = None;
        let border = &decoration.border;
        if !border.has_width() {
            return;
        }
        let [left, top, right, bottom] = border.widths;
        if border.style != BorderStyle::Solid && decoration.border_gradient.is_none() {
            let dotted = border.style == BorderStyle::Dotted;
            let properties = D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_FLAT,
                endCap: D2D1_CAP_STYLE_FLAT,
                dashCap: if dotted {
                    D2D1_CAP_STYLE_ROUND
                } else {
                    D2D1_CAP_STYLE_FLAT
                },
                lineJoin: D2D1_LINE_JOIN_MITER,
                miterLimit: 10.0,
                dashStyle: D2D1_DASH_STYLE_CUSTOM,
                dashOffset: 0.0,
            };
            // Dash lengths are in multiples of the stroke width.
            let dashes: [f32; 2] = if dotted { [0.0, 2.0] } else { [3.0, 2.0] };
            // SAFETY: the properties and the dash array outlive the call.
            let base: &windows::Win32::Graphics::Direct2D::ID2D1Factory = &graphics.factory;
            self.dash = unsafe { base.CreateStrokeStyle(&properties, Some(&dashes)).ok() };
            return;
        }
        let outer = rounded(&graphics.factory, size, decoration.radii, Insets::ZERO)
            .geometry(&graphics.factory);
        let inner = rounded(
            &graphics.factory,
            size,
            decoration.radii,
            Insets::new(left, top, right, bottom),
        )
        .geometry(&graphics.factory);
        let (Some(outer), Some(inner)) = (outer, inner) else {
            return;
        };
        // SAFETY: COM calls on live geometries created above.
        let ring: Option<ID2D1Geometry> = unsafe {
            graphics
                .factory
                .CreateGeometryGroup(D2D1_FILL_MODE_ALTERNATE, &[Some(outer), Some(inner)])
                .ok()
                .and_then(|group| group.cast().ok())
        };
        let Some(ring) = ring else { return };
        let uniform_color = border
            .widths
            .iter()
            .zip(&border.colors)
            .filter(|(width, _)| **width > 0.0)
            .map(|(_, color)| *color)
            .collect::<Vec<_>>();
        let single = uniform_color.windows(2).all(|pair| pair[0] == pair[1]);
        if single || decoration.border_gradient.is_some() {
            self.ring = Some(ring);
            return;
        }
        // Each side is the part of the ring inside the trapezoid between the
        // outer edge and the inner (padding box) edge, mitered at the corners.
        let (width, height) = (size.width, size.height);
        let inner_rect = Rect::new(
            left,
            top,
            (width - left - right).max(0.0),
            (height - top - bottom).max(0.0),
        );
        let outer_points = [
            Point::new(0.0, 0.0),
            Point::new(width, 0.0),
            Point::new(width, height),
            Point::new(0.0, height),
        ];
        let inner_points = [
            Point::new(inner_rect.x, inner_rect.y),
            Point::new(inner_rect.right(), inner_rect.y),
            Point::new(inner_rect.right(), inner_rect.bottom()),
            Point::new(inner_rect.x, inner_rect.bottom()),
        ];
        // Wire order of the sides: left, top, right, bottom.
        let quads = [(3, 0), (0, 1), (1, 2), (2, 3)];
        for (side, (from, to)) in quads.into_iter().enumerate() {
            if border.widths[side] <= 0.0 || border.colors[side].is_transparent() {
                continue;
            }
            let quad = [
                outer_points[from],
                outer_points[to],
                inner_points[to],
                inner_points[from],
            ];
            self.sides[side] = side_geometry(&graphics.factory, &ring, quad);
        }
    }
}

fn side_geometry(
    factory: &ID2D1Factory1,
    ring: &ID2D1Geometry,
    quad: [Point; 4],
) -> Option<ID2D1Geometry> {
    // SAFETY: COM calls on live objects; sinks are closed before the geometries are used.
    unsafe {
        let trapezoid = factory.CreatePathGeometry().ok()?;
        let sink = trapezoid.Open().ok()?;
        sink.BeginFigure(point_vec(quad[0]), D2D1_FIGURE_BEGIN_FILLED);
        sink.AddLine(point_vec(quad[1]));
        sink.AddLine(point_vec(quad[2]));
        sink.AddLine(point_vec(quad[3]));
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close().ok()?;
        let side = factory.CreatePathGeometry().ok()?;
        let sink = side.Open().ok()?;
        ring.CombineWithGeometry(&trapezoid, D2D1_COMBINE_MODE_INTERSECT, None, 0.1, &sink)
            .ok()?;
        sink.Close().ok()?;
        side.cast().ok()
    }
}

fn gradient_stops(gradient: &Gradient, first: f32, span: f32) -> Vec<D2D1_GRADIENT_STOP> {
    let mut stops: Vec<D2D1_GRADIENT_STOP> = gradient
        .stops
        .iter()
        .map(|stop| D2D1_GRADIENT_STOP {
            position: ((stop.offset - first) / span).clamp(0.0, 1.0),
            color: color_f(stop.color),
        })
        .collect();
    if stops.len() == 1 {
        let color = stops[0].color;
        stops.push(D2D1_GRADIENT_STOP {
            position: 1.0,
            color,
        });
    }
    stops
}

/// A brush for a linear or radial gradient over a box of `size`. Conic
/// gradients have no Direct2D brush and are painted as wedges.
fn gradient_brush(
    context: &ID2D1DeviceContext,
    size: Size,
    gradient: &Gradient,
) -> Option<ID2D1Brush> {
    if gradient.stops.is_empty() || matches!(gradient.shape, GradientShape::Conic { .. }) {
        return None;
    }
    let (first, last) = (
        gradient.stops[0].offset,
        gradient.stops[gradient.stops.len() - 1].offset,
    );
    let extend = if gradient.repeating {
        D2D1_EXTEND_MODE_WRAP
    } else {
        D2D1_EXTEND_MODE_CLAMP
    };
    // Stops are interpolated in sRGB with premultiplied alpha, like CSS
    // gradients (a fade to transparent never darkens on the way).
    let collection = |stops: &[D2D1_GRADIENT_STOP]| {
        // SAFETY: COM call on the live context; the stops outlive the call.
        unsafe {
            context
                .CreateGradientStopCollection(
                    stops,
                    D2D1_COLOR_SPACE_SRGB,
                    D2D1_COLOR_SPACE_SRGB,
                    D2D1_BUFFER_PRECISION_8BPC_UNORM,
                    extend,
                    D2D1_COLOR_INTERPOLATION_MODE_PREMULTIPLIED,
                )
                .ok()
        }
    };
    // SAFETY: COM calls on the live context; every structure passed outlives its call.
    unsafe {
        match &gradient.shape {
            GradientShape::Linear { .. } | GradientShape::LinearPoints { .. } => {
                // A repeating gradient tiles the span between its first and last stop.
                let (first, span) = if gradient.repeating && last - first > 0.0001 {
                    (first, last - first)
                } else {
                    (0.0, 1.0)
                };
                let collection = collection(&gradient_stops(gradient, first, span))?;
                let (start, end) = gradient.line(size)?;
                let at = |offset: f32| {
                    Point::new(
                        start.x + (end.x - start.x) * offset,
                        start.y + (end.y - start.y) * offset,
                    )
                };
                let properties = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                    startPoint: point_vec(at(first)),
                    endPoint: point_vec(at(first + span)),
                };
                context
                    .CreateLinearGradientBrush(&properties, None, &collection)
                    .ok()?
                    .cast()
                    .ok()
            }
            GradientShape::Radial {
                center,
                radius_x,
                radius_y,
            } => {
                // A repeating radial gradient tiles rings as wide as its last stop.
                let span = if gradient.repeating && last > 0.0001 {
                    last
                } else {
                    1.0
                };
                let collection = collection(&gradient_stops(gradient, 0.0, span))?;
                // Farthest-corner sizing, like the CSS default.
                let horizontal = (radius_x * size.width * std::f32::consts::SQRT_2).max(1.0);
                let vertical = (radius_y * size.height * std::f32::consts::SQRT_2).max(1.0);
                let properties = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
                    center: vec2(center.x * size.width, center.y * size.height),
                    gradientOriginOffset: vec2(0.0, 0.0),
                    radiusX: horizontal * span,
                    radiusY: vertical * span,
                };
                context
                    .CreateRadialGradientBrush(&properties, None, &collection)
                    .ok()?
                    .cast()
                    .ok()
            }
            GradientShape::Conic { .. } => None,
        }
    }
}

/// A conic gradient as a fan of solid wedges (Direct2D has no sweep brush).
fn paint_conic(
    context: &ID2D1DeviceContext,
    device: &Device,
    factory: &ID2D1Factory1,
    size: Size,
    gradient: &Gradient,
    center: Point,
    angle: f32,
) {
    const WEDGES: usize = 180;
    let center = Point::new(center.x * size.width, center.y * size.height);
    let reach = (size.width + size.height) * 2.0;
    // CSS conic gradients start at the top and turn clockwise.
    let direction = |index: usize| {
        let radians =
            (angle - 90.0).to_radians() + std::f32::consts::TAU * index as f32 / WEDGES as f32;
        Point::new(
            center.x + radians.cos() * reach,
            center.y + radians.sin() * reach,
        )
    };
    // SAFETY: COM calls on live objects; sinks are closed before use and the
    // antialias mode is restored before returning.
    unsafe {
        // Aliased edges leave no seams between neighbouring wedges.
        context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
        for index in 0..WEDGES {
            let color = gradient.color_at((index as f32 + 0.5) / WEDGES as f32);
            if color.is_transparent() {
                continue;
            }
            let Ok(path) = factory.CreatePathGeometry() else {
                break;
            };
            let Ok(sink) = path.Open() else { break };
            sink.BeginFigure(point_vec(center), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLine(point_vec(direction(index)));
            sink.AddLine(point_vec(direction(index + 1)));
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            if sink.Close().is_ok() {
                context.FillGeometry(&path, device.solid(color), None::<&ID2D1Brush>);
            }
        }
        context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ShadowKey {
    width: u32,
    height: u32,
    radii: [u32; 4],
    sigma: u32,
    color: u32,
    scale: u32,
}

struct ShadowEntry {
    bitmap: ID2D1Bitmap1,
    last_used: u64,
}

/// Blurred shadow bitmaps shared by every box with the same shape. Large
/// boxes use a small nine-patch, so a list of cards costs one bitmap.
#[derive(Default)]
pub struct ShadowCache {
    entries: HashMap<ShadowKey, ShadowEntry>,
    frame: u64,
    generation: u64,
}

const SHADOW_CACHE_LIMIT: usize = 256;
/// Frames an unused shadow bitmap survives.
const SHADOW_IDLE_FRAMES: u64 = 240;

impl ShadowCache {
    /// Called once per rendered frame: drops bitmaps nobody drew for a while.
    pub fn begin_frame(&mut self, generation: u64) {
        if self.generation != generation {
            self.entries.clear();
            self.generation = generation;
        }
        self.frame += 1;
        let frame = self.frame;
        if self.entries.len() > SHADOW_CACHE_LIMIT {
            self.entries.retain(|_, entry| frame - entry.last_used < 2);
        } else if frame.is_multiple_of(60) {
            self.entries
                .retain(|_, entry| frame - entry.last_used < SHADOW_IDLE_FRAMES);
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

fn set_float(effect: &windows::Win32::Graphics::Direct2D::ID2D1Effect, index: i32, value: f32) {
    // SAFETY: the byte slice is exactly the float the property expects.
    let _ =
        unsafe { effect.SetValue(index as u32, D2D1_PROPERTY_TYPE_FLOAT, &value.to_ne_bytes()) };
}

fn set_floats(
    effect: &windows::Win32::Graphics::Direct2D::ID2D1Effect,
    index: i32,
    kind: windows::Win32::Graphics::Direct2D::D2D1_PROPERTY_TYPE,
    values: &[f32],
) {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    // SAFETY: the byte slice holds the floats of the property's type.
    let _ = unsafe { effect.SetValue(index as u32, kind, &bytes) };
}

/// The blurred, colored silhouette of whatever `draw` paints, as an image
/// in the same coordinates, to be drawn by `painter`. `draw` records into
/// a command list on the offscreen context of `depth`.
fn shadow_image(
    device: &mut Device,
    painter: &ID2D1DeviceContext,
    depth: usize,
    dpi: f32,
    sigma: f32,
    color: Color,
    draw: impl FnOnce(&ID2D1DeviceContext, &Device),
) -> Option<ID2D1Image> {
    let recorder = device.offscreen(depth).ok()?;
    // SAFETY: COM calls on live objects. The recorder context is not in a
    // BeginDraw (each nesting depth has its own context).
    unsafe {
        let list = recorder.CreateCommandList().ok()?;
        recorder.SetDpi(dpi, dpi);
        recorder.SetTarget(&list);
        recorder.BeginDraw();
        recorder.SetTransform(&Matrix3x2::identity());
        draw(&recorder, device);
        let drawn = recorder.EndDraw(None, None);
        recorder.SetTarget(None::<&ID2D1Image>);
        drawn.ok()?;
        list.Close().ok()?;
        // The effect belongs to the context that will draw its output.
        let effect = painter.CreateEffect(&CLSID_D2D1Shadow).ok()?;
        effect.SetInput(0, &list, true);
        set_float(&effect, D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION.0, sigma);
        set_floats(
            &effect,
            D2D1_SHADOW_PROP_COLOR.0,
            D2D1_PROPERTY_TYPE_VECTOR4,
            &color.to_f32(),
        );
        effect.GetOutput().ok()
    }
}

/// One slice of a nine-patch axis: source span in the bitmap, destination span on screen.
type Slice = (f32, f32, f32, f32);

fn slices(
    source_extent: f32,
    patch: f32,
    start: f32,
    end: f32,
    stretched: bool,
) -> ([Slice; 3], usize) {
    if stretched {
        (
            [
                (0.0, patch, start, start + patch),
                (patch, source_extent - patch, start + patch, end - patch),
                (source_extent - patch, source_extent, end - patch, end),
            ],
            3,
        )
    } else {
        (
            [
                (0.0, source_extent, start, end),
                (0.0, 0.0, 0.0, 0.0),
                (0.0, 0.0, 0.0, 0.0),
            ],
            1,
        )
    }
}

/// What painting needs besides the context it draws on.
pub struct PaintEnv<'a> {
    pub graphics: &'a mut Graphics,
    pub shadows: &'a mut ShadowCache,
    /// Device pixels per logical unit of the window being drawn.
    pub scale: f32,
    /// Nesting depth of offscreen passes (0 = drawing the window).
    pub depth: usize,
}

fn paint_outer_shadow(
    context: &ID2D1DeviceContext,
    env: &mut PaintEnv<'_>,
    size: Size,
    radii: Radii,
    shadow: &Shadow,
) {
    let spread = shadow.spread;
    let shape_size = Size::new(size.width + 2.0 * spread, size.height + 2.0 * spread);
    if shape_size.is_empty() {
        return;
    }
    let origin = Point::new(shadow.offset.x - spread, shadow.offset.y - spread);
    let shape_radii = radii.clamped(size).outset(spread).clamped(shape_size);
    let sigma = shadow.blur / 2.0;
    let factory = env.graphics.factory.clone();
    let Some(device) = env.graphics.device.as_mut() else {
        return;
    };
    if sigma < 0.05 {
        // A sharp shadow is just the shape, moved.
        let shape = rounded(&factory, shape_size, shape_radii, Insets::ZERO);
        // SAFETY: plain transform save/restore around the fill.
        unsafe {
            let mut saved = Matrix3x2::identity();
            context.GetTransform(&mut saved);
            context.SetTransform(&(Matrix3x2::translation(origin.x, origin.y) * saved));
            shape.fill(context, device.solid(shadow.color));
            context.SetTransform(&saved);
        }
        return;
    }
    // Beyond three standard deviations the blur adds nothing visible.
    let margin = (sigma * 3.0).ceil() + 1.0;
    let corner = shape_radii
        .as_array()
        .into_iter()
        .fold(0.0_f32, f32::max)
        .ceil()
        + margin;
    let stretch_x = shape_size.width > 2.0 * corner + 2.0;
    let stretch_y = shape_size.height > 2.0 * corner + 2.0;
    let sample = Size::new(
        if stretch_x {
            2.0 * corner + 2.0
        } else {
            shape_size.width
        },
        if stretch_y {
            2.0 * corner + 2.0
        } else {
            shape_size.height
        },
    );
    let key = ShadowKey {
        width: sample.width.to_bits(),
        height: sample.height.to_bits(),
        radii: shape_radii.as_array().map(f32::to_bits),
        sigma: sigma.to_bits(),
        color: shadow.color.to_argb(),
        scale: env.scale.to_bits(),
    };
    let extent = Size::new(sample.width + 2.0 * margin, sample.height + 2.0 * margin);
    let frame = env.shadows.frame;
    if !env.shadows.entries.contains_key(&key) {
        let dpi = 96.0 * env.scale;
        let shape = rounded(&factory, sample, shape_radii.clamped(sample), Insets::ZERO);
        // The same offscreen context records the shape, then renders the
        // blurred result into the bitmap.
        let Ok(renderer) = device.offscreen(env.depth) else {
            return;
        };
        let image = shadow_image(
            device,
            &renderer,
            env.depth,
            dpi,
            sigma,
            shadow.color,
            |recorder, device| {
                // SAFETY: the recorder is inside its BeginDraw.
                unsafe { recorder.SetTransform(&Matrix3x2::translation(margin, margin)) };
                shape.fill(recorder, device.solid(Color::BLACK));
            },
        );
        let Some(image) = image else { return };
        let (width, height) = (
            (extent.width * env.scale).ceil() as u32,
            (extent.height * env.scale).ceil() as u32,
        );
        let Ok(bitmap) = device.create_target_bitmap(width, height, dpi) else {
            return;
        };
        // SAFETY: COM calls on live objects; the offscreen context is idle
        // (the recording pass above ended its draw).
        unsafe {
            renderer.SetDpi(dpi, dpi);
            renderer.SetTarget(&bitmap);
            renderer.BeginDraw();
            renderer.SetTransform(&Matrix3x2::identity());
            renderer.Clear(None);
            renderer.DrawImage(
                &image,
                None,
                None,
                D2D1_INTERPOLATION_MODE_LINEAR,
                D2D1_COMPOSITE_MODE_SOURCE_OVER,
            );
            let drawn = renderer.EndDraw(None, None);
            renderer.SetTarget(None::<&ID2D1Image>);
            if drawn.is_err() {
                return;
            }
        }
        env.shadows.entries.insert(
            key,
            ShadowEntry {
                bitmap,
                last_used: frame,
            },
        );
    }
    let Some(entry) = env.shadows.entries.get_mut(&key) else {
        return;
    };
    entry.last_used = frame;
    let patch = corner + margin;
    let (columns, column_count) = slices(
        extent.width,
        patch,
        origin.x - margin,
        origin.x + shape_size.width + margin,
        stretch_x,
    );
    let (rows, row_count) = slices(
        extent.height,
        patch,
        origin.y - margin,
        origin.y + shape_size.height + margin,
        stretch_y,
    );
    // SAFETY: COM calls on live objects; the antialias mode is restored.
    unsafe {
        // Aliased edges: neighbouring patches meet without a seam.
        context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
        for row in &rows[..row_count] {
            for column in &columns[..column_count] {
                let source = D2D_RECT_F {
                    left: column.0,
                    top: row.0,
                    right: column.1,
                    bottom: row.1,
                };
                let target = D2D_RECT_F {
                    left: column.2,
                    top: row.2,
                    right: column.3,
                    bottom: row.3,
                };
                context.DrawBitmap(
                    &entry.bitmap,
                    Some(&target),
                    1.0,
                    D2D1_INTERPOLATION_MODE_LINEAR,
                    Some(&source),
                    None,
                );
            }
        }
        context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
    }
}

fn paint_inset_shadow(
    context: &ID2D1DeviceContext,
    env: &mut PaintEnv<'_>,
    size: Size,
    radii: Radii,
    outline: &Shape,
    shadow: &Shadow,
) {
    let factory = env.graphics.factory.clone();
    let Some(device) = env.graphics.device.as_mut() else {
        return;
    };
    let sigma = (shadow.blur / 2.0).max(0.0);
    let spread = shadow.spread;
    // The shadow is cast by everything outside a hole: the box shrunk by
    // the spread and moved by the offset.
    let hole_size = Size::new(
        (size.width - 2.0 * spread).max(0.0),
        (size.height - 2.0 * spread).max(0.0),
    );
    let hole = rounded(
        &factory,
        hole_size,
        radii.clamped(size).outset(-spread).clamped(hole_size),
        Insets::ZERO,
    );
    let reach = sigma * 3.0 + shadow.offset.x.abs().max(shadow.offset.y.abs()) + spread.abs() + 8.0;
    let frame = Rect::new(
        -reach,
        -reach,
        size.width + 2.0 * reach,
        size.height + 2.0 * reach,
    );
    let dpi = 96.0 * env.scale;
    let image = shadow_image(
        device,
        context,
        env.depth,
        dpi,
        sigma,
        shadow.color,
        |recorder, device| {
            // SAFETY: COM calls on live objects inside the recorder's BeginDraw.
            unsafe {
                let Some(hole) = hole.geometry(&factory) else {
                    return;
                };
                let placed: Option<ID2D1Geometry> = factory
                    .CreateTransformedGeometry(
                        &hole,
                        &Matrix3x2::translation(shadow.offset.x + spread, shadow.offset.y + spread),
                    )
                    .ok()
                    .and_then(|geometry| geometry.cast().ok());
                let outer: Option<ID2D1Geometry> = factory
                    .CreateRectangleGeometry(&rect_f(frame))
                    .ok()
                    .and_then(|geometry| geometry.cast().ok());
                if let (Some(placed), Some(outer)) = (placed, outer)
                    && let Ok(group) = factory
                        .CreateGeometryGroup(D2D1_FILL_MODE_ALTERNATE, &[Some(outer), Some(placed)])
                {
                    recorder.FillGeometry(&group, device.solid(Color::BLACK), None::<&ID2D1Brush>);
                }
            }
        },
    );
    let Some(image) = image else { return };
    let clip = push_clip(context, &factory, outline);
    // SAFETY: COM call on the live context, inside its BeginDraw.
    unsafe {
        context.DrawImage(
            &image,
            None,
            None,
            D2D1_INTERPOLATION_MODE_LINEAR,
            D2D1_COMPOSITE_MODE_SOURCE_OVER,
        )
    };
    pop_clip(context, clip);
}

/// Shadows, background color and gradients, inset shadows and borders.
pub fn paint_decoration(
    context: &ID2D1DeviceContext,
    env: &mut PaintEnv<'_>,
    cache: &mut PaintCache,
    size: Size,
    decoration: &Decoration,
) {
    if size.is_empty() {
        return;
    }
    let factory = env.graphics.factory.clone();
    let outline = rounded(&factory, size, decoration.radii, Insets::ZERO);
    let has_fill = !decoration.background.is_transparent() || !decoration.gradients.is_empty();

    if decoration.has_outer_shadow() {
        // A shadow never shows through its own box; only a translucent box
        // needs the mask that guarantees it.
        let covered = decoration.background.is_opaque();
        let mut masked = false;
        if !covered && let Some(inner) = outline.geometry(&factory) {
            let reach = decoration
                .overflow_bounds(Rect::from_size(size))
                .outset(4.0);
            // SAFETY: COM calls on the live factory with live geometries.
            let outside: Option<ID2D1Geometry> = unsafe {
                factory
                    .CreateRectangleGeometry(&rect_f(reach))
                    .ok()
                    .and_then(|geometry| geometry.cast::<ID2D1Geometry>().ok())
                    .and_then(|outer| {
                        factory
                            .CreateGeometryGroup(
                                D2D1_FILL_MODE_ALTERNATE,
                                &[Some(outer), Some(inner)],
                            )
                            .ok()
                            .and_then(|group| group.cast().ok())
                    })
            };
            if let Some(outside) = outside {
                push_layer(context, rect_f(reach), Some(&outside), 1.0);
                masked = true;
            }
        }
        for shadow in decoration
            .shadows
            .iter()
            .filter(|shadow| !shadow.inset && !shadow.color.is_transparent())
        {
            paint_outer_shadow(context, env, size, decoration.radii, shadow);
        }
        if masked {
            // SAFETY: pops the layer pushed above.
            unsafe { context.PopLayer() };
        }
    }

    {
        let graphics = &*env.graphics;
        let Some(device) = graphics.device.as_ref() else {
            return;
        };
        cache.prepare(graphics, device, size, decoration);
        if has_fill {
            if !decoration.background.is_transparent() {
                outline.fill(context, device.solid(decoration.background));
            }
            // The first gradient layer is the topmost one.
            for (index, gradient) in decoration.gradients.iter().enumerate().rev() {
                match (
                    &gradient.shape,
                    cache.gradients.get(index).and_then(Option::as_ref),
                ) {
                    (GradientShape::Conic { center, angle }, _) => {
                        let clip = push_clip(context, &factory, &outline);
                        paint_conic(context, device, &factory, size, gradient, *center, *angle);
                        pop_clip(context, clip);
                    }
                    (_, Some(brush)) => outline.fill(context, brush),
                    _ => {}
                }
            }
        }
    }
    for shadow in decoration
        .shadows
        .iter()
        .filter(|shadow| shadow.inset && !shadow.color.is_transparent())
    {
        paint_inset_shadow(context, env, size, decoration.radii, &outline, shadow);
    }
    let Some(device) = env.graphics.device.as_ref() else {
        return;
    };
    paint_border(context, device, &factory, cache, size, decoration);
}

fn paint_border(
    context: &ID2D1DeviceContext,
    device: &Device,
    factory: &ID2D1Factory1,
    cache: &PaintCache,
    size: Size,
    decoration: &Decoration,
) {
    let border = &decoration.border;
    if !border.has_width() {
        return;
    }
    if let Some(brush) = &cache.border_gradient {
        // The gradient shows only through the border ring.
        if let Some(ring) = &cache.ring {
            // SAFETY: COM call with live objects.
            unsafe { context.FillGeometry(ring, brush, None::<&ID2D1Brush>) };
        }
        return;
    }
    if !border.is_visible() {
        return;
    }
    let [left, top, right, bottom] = border.widths;
    if border.style != BorderStyle::Solid {
        // Dashed and dotted borders follow the center line of the ring.
        let width = left.max(top).max(right).max(bottom);
        let center_line = rounded(factory, size, decoration.radii, Insets::all(width / 2.0));
        let color = border
            .colors
            .iter()
            .copied()
            .find(|color| !color.is_transparent())
            .unwrap_or(Color::TRANSPARENT);
        center_line.stroke(context, device.solid(color), width, cache.dash.as_ref());
        return;
    }
    if let Some(ring) = &cache.ring {
        let color = border
            .widths
            .iter()
            .zip(&border.colors)
            .find(|(width, _)| **width > 0.0)
            .map_or(Color::TRANSPARENT, |(_, color)| *color);
        if !color.is_transparent() {
            // SAFETY: COM call with live objects.
            unsafe { context.FillGeometry(ring, device.solid(color), None::<&ID2D1Brush>) };
        }
        return;
    }
    for (side, geometry) in cache.sides.iter().enumerate() {
        if let Some(geometry) = geometry {
            // SAFETY: COM call with live objects.
            unsafe {
                context.FillGeometry(
                    geometry,
                    device.solid(border.colors[side]),
                    None::<&ID2D1Brush>,
                )
            };
        }
    }
}

/// An image inside its (rounded) box: fit, tint, overlay and opacity.
pub fn paint_image(
    context: &ID2D1DeviceContext,
    device: &Device,
    factory: &ID2D1Factory1,
    size: Size,
    decoration: &Decoration,
    content: &ImageContent,
    bitmap: &ID2D1Bitmap1,
) {
    let insets = content.insets;
    let area = Rect::new(
        insets.left,
        insets.top,
        (size.width - insets.horizontal()).max(0.0),
        (size.height - insets.vertical()).max(0.0),
    );
    // SAFETY: plain getter on a live bitmap.
    let pixels = unsafe { bitmap.GetSize() };
    let natural = Size::new(pixels.width, pixels.height);
    let opacity = content.opacity.clamp(0.0, 1.0);
    if area.is_empty() || natural.is_empty() || opacity <= 0.0 {
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
    // Shrinking by more than half needs prefiltering to stay smooth.
    let shrink = (target.width / natural.width).min(target.height / natural.height);
    let interpolation = if shrink < 0.5 {
        D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC
    } else {
        D2D1_INTERPOLATION_MODE_LINEAR
    };
    let clip_shape = rounded(factory, size, decoration.radii, insets);
    let covers = matches!(content.fit, ImageFit::Cover | ImageFit::Fill);

    // SAFETY: COM calls on live objects inside the context's BeginDraw;
    // every structure passed outlives its call and clips are balanced.
    unsafe {
        // The common case (avatars, cards): the image covers a rounded box.
        // A bitmap brush fills the outline directly, without a layer.
        if covers && content.tint.is_none() && clip_shape.is_rect().is_none() {
            let transform = Matrix3x2 {
                M11: target.width / natural.width,
                M12: 0.0,
                M21: 0.0,
                M22: target.height / natural.height,
                M31: target.x,
                M32: target.y,
            };
            let brush_properties = D2D1_BRUSH_PROPERTIES { opacity, transform };
            let bitmap_properties = D2D1_BITMAP_BRUSH_PROPERTIES1 {
                extendModeX: D2D1_EXTEND_MODE_CLAMP,
                extendModeY: D2D1_EXTEND_MODE_CLAMP,
                interpolationMode: interpolation,
            };
            if let Ok(brush) =
                context.CreateBitmapBrush(bitmap, Some(&bitmap_properties), Some(&brush_properties))
                && let Ok(brush) = brush.cast::<ID2D1Brush>()
            {
                clip_shape.fill(context, &brush);
                if let Some(overlay) = content.overlay {
                    // The overlay fades in with the image.
                    let tint = device.solid(overlay);
                    tint.SetOpacity(opacity);
                    clip_shape.fill(context, tint);
                    tint.SetOpacity(1.0);
                }
                return;
            }
        }
        let clip = push_clip(context, factory, &clip_shape);
        match (content.tint, content.fit) {
            (Some(tint), _) => {
                // The image's alpha is the mask of a solid fill.
                let brush = device.solid(tint);
                brush.SetOpacity(opacity);
                context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
                // A tiny tile over a large area would be thousands of fills
                // per frame: such a pattern is painted as one stretched mask.
                let tiles =
                    (area.width / natural.width).ceil() * (area.height / natural.height).ceil();
                if content.fit == ImageFit::Repeat && tiles <= 1024.0 {
                    let mut y = area.y;
                    while y < area.bottom() {
                        let mut x = area.x;
                        while x < area.right() {
                            let tile = D2D_RECT_F {
                                left: x,
                                top: y,
                                right: x + natural.width,
                                bottom: y + natural.height,
                            };
                            context.FillOpacityMask(bitmap, brush, Some(&tile), None);
                            x += natural.width;
                        }
                        y += natural.height;
                    }
                } else if content.fit == ImageFit::Repeat {
                    context.FillOpacityMask(bitmap, brush, Some(&rect_f(area)), None);
                } else {
                    context.FillOpacityMask(bitmap, brush, Some(&rect_f(target)), None);
                }
                context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                brush.SetOpacity(1.0);
            }
            (None, ImageFit::Repeat) => {
                let brush_properties = D2D1_BRUSH_PROPERTIES {
                    opacity,
                    transform: Matrix3x2::translation(area.x, area.y),
                };
                let bitmap_properties = D2D1_BITMAP_BRUSH_PROPERTIES1 {
                    extendModeX: D2D1_EXTEND_MODE_WRAP,
                    extendModeY: D2D1_EXTEND_MODE_WRAP,
                    interpolationMode: D2D1_INTERPOLATION_MODE_LINEAR,
                };
                if let Ok(brush) = context.CreateBitmapBrush(
                    bitmap,
                    Some(&bitmap_properties),
                    Some(&brush_properties),
                ) {
                    context.FillRectangle(&rect_f(area), &brush);
                }
            }
            (None, _) => context.DrawBitmap(
                bitmap,
                Some(&rect_f(target)),
                opacity,
                interpolation,
                None,
                None,
            ),
        }
        if let Some(overlay) = content.overlay {
            let tint = device.solid(overlay);
            tint.SetOpacity(opacity);
            context.FillRectangle(&rect_f(area), tint);
            tint.SetOpacity(1.0);
        }
        pop_clip(context, clip);
    }
}

fn polyline(factory: &ID2D1Factory1, points: &[Point]) -> Option<ID2D1Geometry> {
    let (first, rest) = points.split_first()?;
    // SAFETY: COM calls on live objects; the sink is closed before use.
    unsafe {
        let path = factory.CreatePathGeometry().ok()?;
        let sink = path.Open().ok()?;
        sink.BeginFigure(point_vec(*first), D2D1_FIGURE_BEGIN_HOLLOW);
        for point in rest {
            sink.AddLine(point_vec(*point));
        }
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
        sink.Close().ok()?;
        path.cast().ok()
    }
}

/// Whether a drawing list erases at its top level, which needs a surface
/// of its own so the erasing never reaches what is under the view.
pub fn drawing_needs_surface(commands: &[DrawCommand]) -> bool {
    commands
        .iter()
        .any(|command| matches!(command, DrawCommand::Stroke { erase: true, .. }))
}

/// Paints a drawing list. `isolated` tells that the context targets a
/// surface holding only this drawing, where erasing strokes clear pixels.
pub fn paint_drawing(
    context: &ID2D1DeviceContext,
    device: &Device,
    graphics: &Graphics,
    commands: &[DrawCommand],
    hairline: f32,
    isolated: bool,
) {
    // Open groups: `true` for a layer, `false` for a plain clip.
    let mut groups: Vec<bool> = Vec::new();
    // SAFETY: COM calls on live objects inside the context's BeginDraw;
    // clips and layers are balanced and the primitive blend is restored
    // after each erase.
    unsafe {
        for (index, command) in commands.iter().enumerate() {
            match command {
                DrawCommand::Rect {
                    rect,
                    radius,
                    color,
                } => {
                    let radius = radius.min(rect.width / 2.0).min(rect.height / 2.0);
                    if radius > 0.0 {
                        context.FillRoundedRectangle(
                            &D2D1_ROUNDED_RECT {
                                rect: rect_f(*rect),
                                radiusX: radius,
                                radiusY: radius,
                            },
                            device.solid(*color),
                        );
                    } else {
                        context.FillRectangle(&rect_f(*rect), device.solid(*color));
                    }
                }
                DrawCommand::Circle {
                    center,
                    radius,
                    color,
                } => {
                    context.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: point_vec(*center),
                            radiusX: *radius,
                            radiusY: *radius,
                        },
                        device.solid(*color),
                    );
                }
                DrawCommand::Line {
                    from,
                    to,
                    width,
                    color,
                } => {
                    let width = if *width > 0.0 { *width } else { hairline };
                    context.DrawLine(
                        point_vec(*from),
                        point_vec(*to),
                        device.solid(*color),
                        width,
                        &graphics.round_stroke,
                    );
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
                    let erasing = *erase;
                    if erasing && !isolated {
                        // Without a surface of its own there is nothing safe to clear.
                        continue;
                    }
                    let color = if erasing { Color::TRANSPARENT } else { *color };
                    if erasing {
                        // Copy blending writes the transparent source over the pixels.
                        context.SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_COPY);
                    }
                    if points.len() == 1 {
                        context.FillEllipse(
                            &D2D1_ELLIPSE {
                                point: point_vec(*first),
                                radiusX: *width / 2.0,
                                radiusY: *width / 2.0,
                            },
                            device.solid(color),
                        );
                    } else if let Some(path) = polyline(&graphics.factory, points) {
                        context.DrawGeometry(
                            &path,
                            device.solid(color),
                            *width,
                            &graphics.round_stroke,
                        );
                    }
                    if erasing {
                        context.SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_SOURCE_OVER);
                    }
                }
                DrawCommand::PushGroup { clip } => {
                    // A group that erases over earlier paint needs its own
                    // intermediate (a layer); otherwise a clip is enough.
                    let painted_before = commands[..index].iter().any(|earlier| {
                        !matches!(
                            earlier,
                            DrawCommand::PushGroup { .. } | DrawCommand::PopGroup
                        )
                    });
                    let layer = isolated && painted_before && group_erases(&commands[index + 1..]);
                    if layer {
                        push_layer(context, rect_f(*clip), None, 1.0);
                    } else {
                        context
                            .PushAxisAlignedClip(&rect_f(*clip), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                    }
                    groups.push(layer);
                }
                DrawCommand::PopGroup => match groups.pop() {
                    Some(true) => context.PopLayer(),
                    Some(false) => context.PopAxisAlignedClip(),
                    None => {}
                },
            }
        }
        // An unbalanced list must not leave clips open on the shared context.
        while let Some(layer) = groups.pop() {
            if layer {
                context.PopLayer();
            } else {
                context.PopAxisAlignedClip();
            }
        }
    }
}

/// Whether the group whose commands start at `commands[0]` holds an erasing stroke.
fn group_erases(commands: &[DrawCommand]) -> bool {
    let mut depth = 0_u32;
    for command in commands {
        match command {
            DrawCommand::PushGroup { .. } => depth += 1,
            DrawCommand::PopGroup => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            DrawCommand::Stroke { erase: true, .. } => return true,
            _ => {}
        }
    }
    false
}

/// Wraps `image` in a Gaussian blur and/or a color matrix, to be drawn by `painter`.
pub fn filter_image(
    painter: &ID2D1DeviceContext,
    image: &ID2D1Image,
    blur: f32,
    color_matrix: Option<&[f32; 20]>,
) -> Option<ID2D1Image> {
    let mut current = image.clone();
    // SAFETY: COM calls on live objects.
    unsafe {
        if let Some(wire) = color_matrix {
            let effect = painter.CreateEffect(&CLSID_D2D1ColorMatrix).ok()?;
            effect.SetInput(0, &current, true);
            // The wire matrix has the Android layout: one row per output
            // channel (R, G, B, A) with five columns (R, G, B, A inputs and
            // an offset in 0..255). Direct2D wants one row per input channel
            // plus the offset row, with offsets in 0..1.
            let mut values = [0.0_f32; 20];
            for output in 0..4 {
                for input in 0..5 {
                    let value = wire[output * 5 + input];
                    values[input * 4 + output] = if input == 4 { value / 255.0 } else { value };
                }
            }
            set_floats(
                &effect,
                D2D1_COLORMATRIX_PROP_COLOR_MATRIX.0,
                D2D1_PROPERTY_TYPE_MATRIX_5X4,
                &values,
            );
            current = effect.GetOutput().ok()?;
        }
        if blur > 0.0 {
            let effect = painter.CreateEffect(&CLSID_D2D1GaussianBlur).ok()?;
            effect.SetInput(0, &current, true);
            set_float(
                &effect,
                D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0,
                blur / 2.0,
            );
            current = effect.GetOutput().ok()?;
        }
    }
    Some(current)
}

/// The blurred silhouette of a text layout, for text shadows.
pub fn text_shadow_image(
    device: &mut Device,
    painter: &ID2D1DeviceContext,
    depth: usize,
    dpi: f32,
    radius: f32,
    color: Color,
    draw: impl FnOnce(&ID2D1DeviceContext, &Device),
) -> Option<ID2D1Image> {
    shadow_image(
        device,
        painter,
        depth,
        dpi,
        (radius / 2.0).max(0.0),
        color,
        draw,
    )
}

/// Applies `matrix(affine) * current` on the context and returns the previous transform.
pub fn concat_transform(
    context: &ID2D1DeviceContext,
    affine: &pam_desktop_ui::geometry::Affine,
) -> Matrix3x2 {
    let mut saved = Matrix3x2::identity();
    // SAFETY: the out pointer refers to a live local.
    unsafe {
        context.GetTransform(&mut saved);
        context.SetTransform(&(matrix(affine) * saved));
    }
    saved
}
