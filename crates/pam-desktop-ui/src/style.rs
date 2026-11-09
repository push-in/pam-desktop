//! Box decoration resolved from node properties: everything a backend paints
//! for a view besides its text and image.

use pam_desktop_wire::{NodeKind, PropKey};

use crate::color::Color;
use crate::geometry::{Affine, Point, Rect, Size};
use crate::props::Props;

/// Corner radii in logical units: top-left, top-right, bottom-right, bottom-left.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Radii {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl Radii {
    pub const ZERO: Self = Self {
        top_left: 0.0,
        top_right: 0.0,
        bottom_right: 0.0,
        bottom_left: 0.0,
    };

    #[must_use]
    pub const fn all(radius: f32) -> Self {
        Self {
            top_left: radius,
            top_right: radius,
            bottom_right: radius,
            bottom_left: radius,
        }
    }

    #[must_use]
    pub fn is_zero(&self) -> bool {
        *self == Self::ZERO
    }

    #[must_use]
    pub fn is_uniform(&self) -> bool {
        self.top_left == self.top_right
            && self.top_right == self.bottom_right
            && self.bottom_right == self.bottom_left
    }

    #[must_use]
    pub fn as_array(&self) -> [f32; 4] {
        [
            self.top_left,
            self.top_right,
            self.bottom_right,
            self.bottom_left,
        ]
    }

    /// CSS corner clamping: adjacent radii never overlap inside `size`.
    #[must_use]
    pub fn clamped(&self, size: Size) -> Self {
        let mut factor = 1.0_f32;
        let mut limit = |extent: f32, sum: f32| {
            if sum > extent && sum > 0.0 {
                factor = factor.min(extent / sum);
            }
        };
        limit(size.width, self.top_left + self.top_right);
        limit(size.width, self.bottom_left + self.bottom_right);
        limit(size.height, self.top_left + self.bottom_left);
        limit(size.height, self.top_right + self.bottom_right);
        let factor = factor.max(0.0);
        Self {
            top_left: self.top_left * factor,
            top_right: self.top_right * factor,
            bottom_right: self.bottom_right * factor,
            bottom_left: self.bottom_left * factor,
        }
    }

    /// Radii of a box grown (positive) or shrunk (negative) by `amount`.
    #[must_use]
    pub fn outset(&self, amount: f32) -> Self {
        let grow = |radius: f32| {
            if radius <= 0.0 && amount > 0.0 {
                0.0
            } else {
                (radius + amount).max(0.0)
            }
        };
        Self {
            top_left: grow(self.top_left),
            top_right: grow(self.top_right),
            bottom_right: grow(self.bottom_right),
            bottom_left: grow(self.bottom_left),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BorderStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

/// Border edges in the wire order: left, top, right, bottom.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Border {
    pub widths: [f32; 4],
    pub colors: [Color; 4],
    pub style: BorderStyle,
}

impl Border {
    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.widths
            .iter()
            .zip(&self.colors)
            .any(|(width, color)| *width > 0.0 && !color.is_transparent())
    }

    #[must_use]
    pub fn is_uniform(&self) -> bool {
        self.widths.iter().all(|width| *width == self.widths[0])
            && self.colors.iter().all(|color| *color == self.colors[0])
    }

    #[must_use]
    pub fn has_width(&self) -> bool {
        self.widths.iter().any(|width| *width > 0.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    pub color: Color,
    /// Position along the gradient line in `0.0..=1.0`.
    pub offset: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GradientShape {
    /// CSS angle in degrees: 0 points up, 180 (the default) points down.
    Linear {
        angle: f32,
    },
    /// Explicit line in fractions of the box (`0..1`).
    LinearPoints {
        start: Point,
        end: Point,
    },
    /// Center in fractions of the box; the ending shape is resolved
    /// against the painted box by [`Gradient::radii`].
    Radial {
        center: Point,
        circle: bool,
        extent: RadialExtent,
    },
    Conic {
        center: Point,
        angle: f32,
    },
}

/// CSS `radial-gradient()` ending-shape size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RadialExtent {
    ClosestSide,
    ClosestCorner,
    FarthestSide,
    FarthestCorner,
    /// Explicit radii: `(value, is_fraction)`; a fraction is of the box
    /// width (horizontal) or height (vertical).
    Explicit {
        horizontal: (f32, bool),
        vertical: (f32, bool),
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub shape: GradientShape,
    pub stops: Vec<GradientStop>,
    pub repeating: bool,
}

impl Gradient {
    /// Start and end of a linear gradient line for a box of `size`, in the
    /// box's coordinates (CSS `linear-gradient` geometry).
    #[must_use]
    pub fn line(&self, size: Size) -> Option<(Point, Point)> {
        match &self.shape {
            GradientShape::Linear { angle } => {
                let radians = angle.to_radians();
                let (sin, cos) = radians.sin_cos();
                let half = (size.width * sin).abs() / 2.0 + (size.height * cos).abs() / 2.0;
                let center = Point::new(size.width / 2.0, size.height / 2.0);
                Some((
                    Point::new(center.x - sin * half, center.y + cos * half),
                    Point::new(center.x + sin * half, center.y - cos * half),
                ))
            }
            GradientShape::LinearPoints { start, end } => Some((
                Point::new(start.x * size.width, start.y * size.height),
                Point::new(end.x * size.width, end.y * size.height),
            )),
            _ => None,
        }
    }

    /// Horizontal and vertical radii of a radial gradient's ending shape
    /// for a box of `size` (CSS sizing: `closest-side` … `farthest-corner`
    /// measured from the center, or explicit radii), at least 1.
    #[must_use]
    pub fn radii(&self, size: Size) -> Option<(f32, f32)> {
        let GradientShape::Radial {
            center,
            circle,
            extent,
        } = &self.shape
        else {
            return None;
        };
        let (cx, cy) = (center.x * size.width, center.y * size.height);
        let (left, right) = (cx.abs(), (size.width - cx).abs());
        let (top, bottom) = (cy.abs(), (size.height - cy).abs());
        let (near_x, far_x) = (left.min(right), left.max(right));
        let (near_y, far_y) = (top.min(bottom), top.max(bottom));
        let (horizontal, vertical) = match (*circle, *extent) {
            (
                _,
                RadialExtent::Explicit {
                    horizontal,
                    vertical,
                },
            ) => {
                let resolve = |(value, fraction): (f32, bool), side: f32| {
                    if fraction { value * side } else { value }
                };
                let x = resolve(horizontal, size.width);
                (
                    x,
                    if *circle {
                        x
                    } else {
                        resolve(vertical, size.height)
                    },
                )
            }
            (true, RadialExtent::ClosestSide) => (near_x.min(near_y), near_x.min(near_y)),
            (true, RadialExtent::FarthestSide) => (far_x.max(far_y), far_x.max(far_y)),
            (true, RadialExtent::ClosestCorner) => {
                let radius = near_x.hypot(near_y);
                (radius, radius)
            }
            (true, RadialExtent::FarthestCorner) => {
                let radius = far_x.hypot(far_y);
                (radius, radius)
            }
            (false, RadialExtent::ClosestSide) => (near_x, near_y),
            (false, RadialExtent::FarthestSide) => (far_x, far_y),
            // An ellipse through the corner with the aspect of the sides.
            (false, RadialExtent::ClosestCorner) => (
                near_x * std::f32::consts::SQRT_2,
                near_y * std::f32::consts::SQRT_2,
            ),
            (false, RadialExtent::FarthestCorner) => (
                far_x * std::f32::consts::SQRT_2,
                far_y * std::f32::consts::SQRT_2,
            ),
        };
        Some((horizontal.max(1.0), vertical.max(1.0)))
    }

    /// Color at `offset` (used by hosts without native gradients and by tests).
    #[must_use]
    pub fn color_at(&self, offset: f32) -> Color {
        let Some(first) = self.stops.first() else {
            return Color::TRANSPARENT;
        };
        if offset <= first.offset {
            return first.color;
        }
        for pair in self.stops.windows(2) {
            if offset <= pair[1].offset {
                let span = pair[1].offset - pair[0].offset;
                let t = if span > 0.0 {
                    (offset - pair[0].offset) / span
                } else {
                    1.0
                };
                return pair[0].color.lerp(pair[1].color, t);
            }
        }
        self.stops[self.stops.len() - 1].color
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub offset: Point,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
    pub inset: bool,
}

impl Shadow {
    /// Area an outer shadow may paint around a box.
    #[must_use]
    pub fn bounds(&self, rect: Rect) -> Rect {
        rect.translate(self.offset.x, self.offset.y)
            .outset(self.spread.max(0.0) + self.blur * 2.0)
    }
}

/// Everything painted for a box behind and around its content.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decoration {
    pub background: Color,
    /// Painted over `background`, last layer first (CSS order: first is topmost).
    pub gradients: Vec<Gradient>,
    pub radii: Radii,
    pub border: Border,
    pub border_gradient: Option<Gradient>,
    pub shadows: Vec<Shadow>,
    /// `overflow: hidden`: children are clipped to the rounded padding box.
    pub clip: bool,
    /// Gaussian blur applied to the view and its children.
    pub blur: f32,
    /// 4x5 color matrix applied to the view and its children.
    pub color_matrix: Option<[f32; 20]>,
    /// Blur of whatever is behind the view (frosted glass).
    pub backdrop_blur: f32,
    /// Boxes painted right over this backdrop view whose outer shadow and
    /// translucent background a backend that composites the backdrop
    /// itself paints into it (the boxes then paint neither); see
    /// `presenter::modal`.
    pub backdrop_boxes: Vec<BackdropBox>,
    /// The translucent background is composited by the backdrop under the
    /// box (`backdrop_boxes`): the backend paints no fill (everything else
    /// still reads `background`).
    pub fill_composited: bool,
}

/// A box painted right over a backdrop (`rect`, rounded by `radii`, in the
/// backdrop view's coordinates): its outer shadow and translucent
/// background, composited into the backdrop. The first box of a list is
/// the dialog; the others, boxes inside it, add their fills to its layer,
/// each cut to `clip` (its clipping ancestors).
#[derive(Clone, Debug, PartialEq)]
pub struct BackdropBox {
    pub rect: Rect,
    pub radii: Radii,
    pub shadow: Option<Shadow>,
    pub fill: Option<Color>,
    pub clip: Option<Rect>,
}

impl Decoration {
    /// True when nothing is painted and nothing is clipped.
    #[must_use]
    pub fn is_plain(&self) -> bool {
        self.background.is_transparent()
            && self.gradients.is_empty()
            && !self.border.is_visible()
            && self.shadows.is_empty()
            && !self.clip
            && self.blur <= 0.0
            && self.color_matrix.is_none()
            && self.backdrop_blur <= 0.0
    }

    #[must_use]
    pub fn has_outer_shadow(&self) -> bool {
        self.shadows
            .iter()
            .any(|shadow| !shadow.inset && !shadow.color.is_transparent())
    }

    /// How far paint may extend beyond the box (shadows).
    #[must_use]
    pub fn overflow_bounds(&self, rect: Rect) -> Rect {
        self.shadows
            .iter()
            .filter(|shadow| !shadow.inset)
            .fold(rect, |bounds, shadow| bounds.union(shadow.bounds(rect)))
    }

    #[must_use]
    pub fn resolve(kind: NodeKind, props: Props<'_>) -> Self {
        let uniform_radius = props.number(PropKey::BorderRadius).unwrap_or(0.0).max(0.0);
        let radius = |key| props.number(key).unwrap_or(uniform_radius).max(0.0);
        let radii = Radii {
            top_left: radius(PropKey::BorderTopLeftRadius),
            top_right: radius(PropKey::BorderTopRightRadius),
            bottom_right: radius(PropKey::BorderBottomRightRadius),
            bottom_left: radius(PropKey::BorderBottomLeftRadius),
        };

        let uniform_width = props.number(PropKey::BorderWidth).unwrap_or(0.0);
        let width = |key| props.number(key).unwrap_or(uniform_width).max(0.0);
        let base_color = props
            .color(PropKey::BorderColor)
            .unwrap_or(Color::TRANSPARENT);
        let color = |key| props.color(key).unwrap_or(base_color);
        let border = Border {
            widths: [
                width(PropKey::BorderLeftWidth),
                width(PropKey::BorderTopWidth),
                width(PropKey::BorderRightWidth),
                width(PropKey::BorderBottomWidth),
            ],
            colors: [
                color(PropKey::BorderLeftColor),
                color(PropKey::BorderTopColor),
                color(PropKey::BorderRightColor),
                color(PropKey::BorderBottomColor),
            ],
            style: match props.integer(PropKey::BorderStyle) {
                Some(2) => BorderStyle::Dashed,
                Some(3) => BorderStyle::Dotted,
                _ => BorderStyle::Solid,
            },
        };

        let image_host = matches!(
            kind,
            NodeKind::Image | NodeKind::ImageBackground | NodeKind::DrawingCanvas
        );
        let gradients = if image_host {
            Vec::new()
        } else {
            parse_gradients(props.text_or_empty(PropKey::BackgroundGradient))
        };
        let border_gradient = if image_host || !border.has_width() {
            None
        } else {
            parse_gradients(props.text_or_empty(PropKey::BorderGradient))
                .into_iter()
                .next()
        };

        let shadows = match props.text(PropKey::BoxShadows) {
            Some(list) if !list.is_empty() => parse_shadows(list),
            _ => match props.color(PropKey::ShadowColor) {
                Some(color) => vec![Shadow {
                    offset: Point::new(
                        props.number_or(PropKey::ShadowOffsetX, 0.0),
                        props.number_or(PropKey::ShadowOffsetY, 0.0),
                    ),
                    blur: props.number_or(PropKey::ShadowBlurRadius, 0.0).max(0.0),
                    spread: props.number_or(PropKey::ShadowSpreadRadius, 0.0),
                    color,
                    inset: false,
                }],
                // Material elevation without a CSS shadow.
                None => elevation_shadow(props.number_or(PropKey::Elevation, 0.0))
                    .into_iter()
                    .collect(),
            },
        };

        Self {
            background: props
                .color(PropKey::BackgroundColor)
                .unwrap_or(Color::TRANSPARENT),
            gradients,
            radii,
            border,
            border_gradient,
            shadows,
            clip: props.integer(PropKey::Overflow) == Some(2)
                || (matches!(kind, NodeKind::Image | NodeKind::Media | NodeKind::WebView)
                    && !radii.is_zero()),
            blur: props.number_or(PropKey::BlurRadius, 0.0).max(0.0),
            color_matrix: parse_color_matrix(props.text_or_empty(PropKey::FilterColorMatrix)),
            // A Modal's `backdropBlurRadius` frosts its backdrop (the
            // presenter's mask), not the full-window modal box as well.
            backdrop_blur: if kind == NodeKind::Modal {
                0.0
            } else {
                props.number_or(PropKey::BackdropBlurRadius, 0.0).max(0.0)
            },
            backdrop_boxes: Vec::new(),
            fill_composited: false,
        }
    }
}

fn elevation_shadow(elevation: f32) -> Option<Shadow> {
    (elevation > 0.0).then(|| Shadow {
        offset: Point::new(0.0, (elevation * 0.45).max(1.0)),
        blur: (elevation * 0.72).max(1.0) * 2.0,
        spread: 0.0,
        color: Color::BLACK.with_alpha((0.10 + elevation * 0.008).min(0.24)),
        inset: false,
    })
}

/// `[[x, y, blur, spread, argb, inset], …]` in logical units.
#[must_use]
pub fn parse_shadows(wire: &str) -> Vec<Shadow> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(wire)
    else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let fields = item.as_array()?;
            if fields.len() < 5 {
                return None;
            }
            let number = |index: usize| {
                fields
                    .get(index)
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0) as f32
            };
            let color = fields[4]
                .as_i64()
                .or_else(|| fields[4].as_f64().map(|value| value as i64))?;
            Some(Shadow {
                offset: Point::new(number(0), number(1)),
                blur: number(2).max(0.0),
                spread: number(3),
                color: Color::from_wire(color),
                inset: fields.get(5).and_then(serde_json::Value::as_i64) == Some(1),
            })
        })
        .collect()
}

const GRADIENT_LINEAR: i64 = 1;
const GRADIENT_RADIAL: i64 = 2;
const GRADIENT_CONIC: i64 = 3;
const MODE_ANGLE: i64 = 1;
const MODE_CORNER: i64 = 2;
const MODE_POINTS: i64 = 3;

/// The compiler's gradient list: a JSON array of layers
/// `{t: type, r: repeating, m: mode, a: angle, x/y: corner, p: [x0,y0,x1,y1],
///   c: [[x,unit],[y,unit]], s: [[argb, position, unit], …]}`.
#[must_use]
pub fn parse_gradients(wire: &str) -> Vec<Gradient> {
    if wire.is_empty() {
        return Vec::new();
    }
    let Ok(serde_json::Value::Array(layers)) = serde_json::from_str::<serde_json::Value>(wire)
    else {
        return Vec::new();
    };
    layers.iter().filter_map(parse_gradient).collect()
}

fn parse_gradient(layer: &serde_json::Value) -> Option<Gradient> {
    let raw_stops = layer.get("s")?.as_array()?;
    if raw_stops.is_empty() {
        return None;
    }
    let integer = |key: &str| layer.get(key).and_then(serde_json::Value::as_i64);
    let number = |key: &str| {
        layer
            .get(key)
            .and_then(serde_json::Value::as_f64)
            .map(|value| value as f32)
    };

    // Stops: explicit fractions are kept; `auto` positions are spread evenly
    // between their explicit neighbours (CSS color stop fix-up).
    let mut colors = Vec::with_capacity(raw_stops.len());
    let mut positions: Vec<Option<f32>> = Vec::with_capacity(raw_stops.len());
    for stop in raw_stops {
        let fields = stop.as_array()?;
        let color = fields
            .first()
            .and_then(|value| value.as_i64().or_else(|| value.as_f64().map(|v| v as i64)))?;
        colors.push(Color::from_wire(color));
        let unit = fields
            .get(2)
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        let position = fields
            .get(1)
            .and_then(serde_json::Value::as_f64)
            .map(|value| value as f32);
        // unit 0 = auto, 1 = fraction, 2 = percent.
        positions.push(match (unit, position) {
            (0, _) | (_, None) => None,
            (2, Some(value)) => Some(value / 100.0),
            (_, Some(value)) => Some(value),
        });
    }
    let count = positions.len();
    if positions[0].is_none() {
        positions[0] = Some(0.0);
    }
    if positions[count - 1].is_none() {
        positions[count - 1] = Some(1.0);
    }
    let mut index = 0;
    while index < count {
        if positions[index].is_some() {
            index += 1;
            continue;
        }
        let start = index - 1;
        let mut end = index;
        while positions[end].is_none() {
            end += 1;
        }
        let from = positions[start].unwrap_or(0.0);
        let to = positions[end].unwrap_or(1.0);
        for (step, position) in positions.iter_mut().enumerate().take(end).skip(index) {
            *position = Some(from + (to - from) * (step - start) as f32 / (end - start) as f32);
        }
        index = end;
    }
    let mut last = f32::MIN;
    let stops = colors
        .into_iter()
        .zip(positions)
        .map(|(color, position)| {
            let offset = position.unwrap_or(0.0).max(last);
            last = offset;
            GradientStop { color, offset }
        })
        .collect();

    let center = || {
        let axis = |index: usize| {
            let pair = layer
                .get("c")
                .and_then(|center| center.get(index))
                .and_then(serde_json::Value::as_array);
            let value = pair
                .and_then(|pair| pair.first())
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.5) as f32;
            let unit = pair
                .and_then(|pair| pair.get(1))
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(1);
            if unit == 2 { value / 100.0 } else { value }
        };
        Point::new(axis(0), axis(1))
    };

    let shape = match integer("t").unwrap_or(GRADIENT_LINEAR) {
        GRADIENT_RADIAL => GradientShape::Radial {
            center: center(),
            circle: integer("e") == Some(0),
            extent: match integer("z") {
                Some(1) => RadialExtent::ClosestSide,
                Some(2) => RadialExtent::ClosestCorner,
                Some(3) => RadialExtent::FarthestSide,
                Some(5) => {
                    let radius = |key: &str| {
                        let pair = layer.get(key).and_then(serde_json::Value::as_array);
                        let value = pair
                            .and_then(|pair| pair.first())
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or(0.5) as f32;
                        let fraction = pair
                            .and_then(|pair| pair.get(1))
                            .and_then(serde_json::Value::as_i64)
                            .unwrap_or(1)
                            == 1;
                        (value, fraction)
                    };
                    RadialExtent::Explicit {
                        horizontal: radius("w"),
                        vertical: radius("h"),
                    }
                }
                _ => RadialExtent::FarthestCorner,
            },
        },
        GRADIENT_CONIC => GradientShape::Conic {
            center: center(),
            angle: number("a").unwrap_or(0.0),
        },
        _ => match integer("m").unwrap_or(MODE_ANGLE) {
            MODE_POINTS => {
                let points = layer.get("p").and_then(serde_json::Value::as_array);
                let point = |index: usize| {
                    points
                        .and_then(|points| points.get(index))
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0) as f32
                };
                GradientShape::LinearPoints {
                    start: Point::new(point(0), point(1)),
                    end: Point::new(point(2), point(3)),
                }
            }
            MODE_CORNER => {
                // `to <corner>`: x/y are -1, 0 or 1 towards the target corner.
                let x = number("x").unwrap_or(0.0);
                let y = number("y").unwrap_or(0.0);
                let start = Point::new(
                    if x > 0.0 {
                        0.0
                    } else if x < 0.0 {
                        1.0
                    } else {
                        0.5
                    },
                    if y > 0.0 {
                        0.0
                    } else if y < 0.0 {
                        1.0
                    } else {
                        0.5
                    },
                );
                let end = Point::new(1.0 - start.x, 1.0 - start.y);
                GradientShape::LinearPoints { start, end }
            }
            _ => GradientShape::Linear {
                angle: number("a").unwrap_or(180.0),
            },
        },
    };
    Some(Gradient {
        shape,
        stops,
        repeating: integer("r") == Some(1),
    })
}

/// 20 comma or space separated decimals (rows of R, G, B, A with an offset column).
#[must_use]
pub fn parse_color_matrix(wire: &str) -> Option<[f32; 20]> {
    if wire.is_empty() {
        return None;
    }
    let mut matrix = [0.0_f32; 20];
    let mut count = 0;
    for field in wire.split(|character: char| {
        character == ',' || character.is_whitespace() || character == '[' || character == ']'
    }) {
        if field.is_empty() {
            continue;
        }
        if count == 20 {
            return None;
        }
        matrix[count] = field
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite())?;
        count += 1;
    }
    (count == 20).then_some(matrix)
}

/// Compositing state that changes during animations: cheap for backends to
/// update without repainting the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerState {
    pub opacity: f32,
    /// Around the view's own origin (its frame's top-left corner).
    pub transform: Affine,
    pub visible: bool,
}

impl Default for LayerState {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            transform: Affine::IDENTITY,
            visible: true,
        }
    }
}

/// The node's authored transform for a box of `size`.
#[must_use]
pub fn resolve_transform(props: Props<'_>, size: Size) -> Affine {
    let translate_x = props.number_or(PropKey::TranslationX, 0.0)
        + props.number_or(PropKey::TranslationXPercent, 0.0) / 100.0 * size.width;
    let translate_y = props.number_or(PropKey::TranslationY, 0.0)
        + props.number_or(PropKey::TranslationYPercent, 0.0) / 100.0 * size.height;
    let scale_x = props.number_or(PropKey::ScaleX, 1.0);
    let scale_y = props.number_or(PropKey::ScaleY, 1.0);
    let rotation = props.number_or(PropKey::Rotation, 0.0);
    compose_transform(
        translate_x,
        translate_y,
        scale_x,
        scale_y,
        rotation,
        transform_origin(props, size),
    )
}

/// Transform origin in the box's coordinates (percent on the wire, center by default).
#[must_use]
pub fn transform_origin(props: Props<'_>, size: Size) -> Point {
    Point::new(
        props.number_or(PropKey::TransformOriginX, 50.0) / 100.0 * size.width,
        props.number_or(PropKey::TransformOriginY, 50.0) / 100.0 * size.height,
    )
}

#[must_use]
pub fn compose_transform(
    translate_x: f32,
    translate_y: f32,
    scale_x: f32,
    scale_y: f32,
    rotation: f32,
    origin: Point,
) -> Affine {
    let mut transform = Affine::IDENTITY;
    if scale_x != 1.0 || scale_y != 1.0 {
        transform = transform.then(&Affine::scaling(scale_x, scale_y));
    }
    if rotation != 0.0 {
        transform = transform.then(&Affine::rotation(rotation));
    }
    if !transform.is_identity() {
        transform = transform.around(origin);
    }
    if translate_x != 0.0 || translate_y != 0.0 {
        transform = transform.then(&Affine::translation(translate_x, translate_y));
    }
    transform
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::props::PropMap;
    use pam_desktop_wire::PropValue;

    #[test]
    fn radial_gradients_size_their_ending_shape_like_css() {
        let radii = |wire: &str| {
            let gradients = parse_gradients(wire);
            gradients[0].radii(Size::new(200.0, 100.0)).expect("radial")
        };
        let stops = r#""s":[[4294901760,0,1],[0,1,1]]"#;
        // circle at top left, farthest-corner: the box diagonal.
        let (x, y) = radii(&format!(
            r#"[{{"t":2,{stops},"e":0,"z":4,"c":[[0,1],[0,1]]}}]"#
        ));
        assert!((x - 200.0_f32.hypot(100.0)).abs() < 0.01 && x == y);
        // Default ellipse at the center: sqrt(2) times the half sides.
        let (x, y) = radii(&format!(r#"[{{"t":2,{stops},"e":1,"z":4}}]"#));
        assert!((x - 100.0 * std::f32::consts::SQRT_2).abs() < 0.01);
        assert!((y - 50.0 * std::f32::consts::SQRT_2).abs() < 0.01);
        // circle closest-side at 20% bottom: the bottom edge is 0 away.
        let (x, _) = radii(&format!(
            r#"[{{"t":2,{stops},"e":0,"z":1,"c":[[0.2,1],[1,1]]}}]"#
        ));
        assert_eq!(x, 1.0);
        // Explicit 40px 50%.
        let (x, y) = radii(&format!(
            r#"[{{"t":2,{stops},"e":1,"z":5,"w":[40,2],"h":[0.5,1]}}]"#
        ));
        assert_eq!((x, y), (40.0, 50.0));
    }

    #[test]
    fn resolves_radii_borders_and_shadows() {
        let mut map = PropMap::new();
        map.insert(PropKey::BorderRadius, PropValue::Float(8.0));
        map.insert(PropKey::BorderTopLeftRadius, PropValue::Float(2.0));
        map.insert(PropKey::BorderWidth, PropValue::Integer(1));
        map.insert(PropKey::BorderBottomWidth, PropValue::Integer(3));
        map.insert(PropKey::BorderColor, PropValue::Integer(0xFF00_0000));
        map.insert(PropKey::BackgroundColor, PropValue::Integer(0xFFFF_FFFF));
        map.insert(
            PropKey::BoxShadows,
            PropValue::String("[[0,4,12,0,1493172224,0],[0,1,0,0,-1,1]]".into()),
        );
        let decoration = Decoration::resolve(NodeKind::View, Props::new(&map));
        assert_eq!(
            decoration.radii,
            Radii {
                top_left: 2.0,
                top_right: 8.0,
                bottom_right: 8.0,
                bottom_left: 8.0
            }
        );
        assert_eq!(decoration.border.widths, [1.0, 1.0, 1.0, 3.0]);
        assert!(decoration.border.is_visible() && !decoration.border.is_uniform());
        assert_eq!(decoration.shadows.len(), 2);
        assert_eq!(decoration.shadows[0].color, Color::from_argb(0x5900_0000));
        assert!(decoration.shadows[1].inset);
        assert_eq!(decoration.background, Color::WHITE);
    }

    #[test]
    fn elevation_becomes_a_shadow_only_without_css_shadows() {
        let mut map = PropMap::new();
        map.insert(PropKey::Elevation, PropValue::Float(4.0));
        assert_eq!(
            Decoration::resolve(NodeKind::View, Props::new(&map))
                .shadows
                .len(),
            1
        );
        map.insert(PropKey::ShadowColor, PropValue::Integer(0x4000_0000));
        let decoration = Decoration::resolve(NodeKind::View, Props::new(&map));
        assert_eq!(decoration.shadows.len(), 1);
        assert_eq!(decoration.shadows[0].color, Color::from_argb(0x4000_0000));
    }

    #[test]
    fn parses_linear_gradients_and_spreads_auto_stops() {
        let gradients = parse_gradients(
            r#"[{"t":1,"m":1,"a":90,"s":[[-65536,0,0],[-16711936,0,0],[-16776961,0,0]]}]"#,
        );
        assert_eq!(gradients.len(), 1);
        let offsets: Vec<f32> = gradients[0].stops.iter().map(|stop| stop.offset).collect();
        assert_eq!(offsets, vec![0.0, 0.5, 1.0]);
        let (start, end) = gradients[0].line(Size::new(100.0, 50.0)).expect("linear");
        assert!((start.x - 0.0).abs() < 1e-3 && (end.x - 100.0).abs() < 1e-3);
        assert!((start.y - 25.0).abs() < 1e-3 && (end.y - 25.0).abs() < 1e-3);
        assert_eq!(gradients[0].color_at(0.0), Color::from_argb(0xFFFF_0000));
    }

    #[test]
    fn radii_are_clamped_like_css() {
        let clamped = Radii::all(40.0).clamped(Size::new(40.0, 100.0));
        assert_eq!(clamped, Radii::all(20.0));
    }

    #[test]
    fn transform_scales_around_the_center_by_default() {
        let mut map = PropMap::new();
        map.insert(PropKey::ScaleX, PropValue::Float(2.0));
        map.insert(PropKey::ScaleY, PropValue::Float(2.0));
        map.insert(PropKey::TranslationX, PropValue::Float(10.0));
        let transform = resolve_transform(Props::new(&map), Size::new(100.0, 40.0));
        assert_eq!(
            transform.apply(Point::new(50.0, 20.0)),
            Point::new(60.0, 20.0)
        );
        assert_eq!(
            transform.apply(Point::new(0.0, 0.0)),
            Point::new(-40.0, -20.0)
        );
    }

    #[test]
    fn color_matrix_needs_twenty_values() {
        assert!(parse_color_matrix("1 0 0 0 0 0 1 0 0 0 0 0 1 0 0 0 0 0 1 0").is_some());
        assert!(parse_color_matrix("1,0,0").is_none());
    }
}
