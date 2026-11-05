//! Points, rectangles and 2D affine transforms in logical (density
//! independent) units. The engine lays out in the same units.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    #[must_use]
    pub fn offset(self, dx: f32, dy: f32) -> Self {
        Self::new(self.x + dx, self.y + dy)
    }

    #[must_use]
    pub fn distance(self, other: Self) -> f32 {
        (self.x - other.x).hypot(self.y - other.y)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const ZERO: Self = Self { width: 0.0, height: 0.0 };

    #[must_use]
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0, width: 0.0, height: 0.0 };

    #[must_use]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    #[must_use]
    pub const fn from_size(size: Size) -> Self {
        Self::new(0.0, 0.0, size.width, size.height)
    }

    #[must_use]
    pub const fn origin(self) -> Point {
        Point::new(self.x, self.y)
    }

    #[must_use]
    pub const fn size(self) -> Size {
        Size::new(self.width, self.height)
    }

    #[must_use]
    pub fn right(self) -> f32 {
        self.x + self.width
    }

    #[must_use]
    pub fn bottom(self) -> f32 {
        self.y + self.height
    }

    #[must_use]
    pub fn center(self) -> Point {
        Point::new(self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    #[must_use]
    pub fn contains(self, point: Point) -> bool {
        point.x >= self.x && point.y >= self.y && point.x < self.right() && point.y < self.bottom()
    }

    #[must_use]
    pub fn translate(self, dx: f32, dy: f32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.width, self.height)
    }

    /// Grows (positive) or shrinks (negative) every edge.
    #[must_use]
    pub fn outset(self, amount: f32) -> Self {
        Self::new(
            self.x - amount,
            self.y - amount,
            (self.width + amount * 2.0).max(0.0),
            (self.height + amount * 2.0).max(0.0),
        )
    }

    #[must_use]
    pub fn outset_by(self, insets: Insets) -> Self {
        Self::new(
            self.x - insets.left,
            self.y - insets.top,
            (self.width + insets.left + insets.right).max(0.0),
            (self.height + insets.top + insets.bottom).max(0.0),
        )
    }

    #[must_use]
    pub fn inset_by(self, insets: Insets) -> Self {
        self.outset_by(Insets::new(-insets.left, -insets.top, -insets.right, -insets.bottom))
    }

    #[must_use]
    pub fn intersect(self, other: Self) -> Option<Self> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (right > x && bottom > y).then(|| Self::new(x, y, right - x, bottom - y))
    }

    #[must_use]
    pub fn intersects(self, other: Self) -> bool {
        self.intersect(other).is_some()
    }

    #[must_use]
    pub fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self::new(x, y, self.right().max(other.right()) - x, self.bottom().max(other.bottom()) - y)
    }

    /// Snaps the edges to the device pixel grid so 1px borders and text stay crisp.
    #[must_use]
    pub fn snapped(self, scale: f32) -> Self {
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        let snap = |value: f32| (value * scale).round() / scale;
        let x = snap(self.x);
        let y = snap(self.y);
        Self::new(x, y, (snap(self.right()) - x).max(0.0), (snap(self.bottom()) - y).max(0.0))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Insets {
    pub const ZERO: Self = Self { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 };

    #[must_use]
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self { left, top, right, bottom }
    }

    #[must_use]
    pub const fn all(value: f32) -> Self {
        Self::new(value, value, value, value)
    }

    #[must_use]
    pub fn is_zero(self) -> bool {
        self.left == 0.0 && self.top == 0.0 && self.right == 0.0 && self.bottom == 0.0
    }

    #[must_use]
    pub fn horizontal(self) -> f32 {
        self.left + self.right
    }

    #[must_use]
    pub fn vertical(self) -> f32 {
        self.top + self.bottom
    }
}

/// 2D affine transform `[a c tx; b d ty]`, applied to column vectors.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Default for Affine {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine {
    pub const IDENTITY: Self = Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 };

    #[must_use]
    pub const fn translation(tx: f32, ty: f32) -> Self {
        Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx, ty }
    }

    #[must_use]
    pub const fn scaling(sx: f32, sy: f32) -> Self {
        Self { a: sx, b: 0.0, c: 0.0, d: sy, tx: 0.0, ty: 0.0 }
    }

    /// Clockwise rotation in degrees (y axis points down).
    #[must_use]
    pub fn rotation(degrees: f32) -> Self {
        let (sin, cos) = degrees.to_radians().sin_cos();
        Self { a: cos, b: sin, c: -sin, d: cos, tx: 0.0, ty: 0.0 }
    }

    #[must_use]
    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }

    #[must_use]
    pub fn is_translation(&self) -> bool {
        self.a == 1.0 && self.b == 0.0 && self.c == 0.0 && self.d == 1.0
    }

    /// `self` first, then `next` (`next * self`).
    #[must_use]
    pub fn then(&self, next: &Self) -> Self {
        Self {
            a: next.a * self.a + next.c * self.b,
            b: next.b * self.a + next.d * self.b,
            c: next.a * self.c + next.c * self.d,
            d: next.b * self.c + next.d * self.d,
            tx: next.a * self.tx + next.c * self.ty + next.tx,
            ty: next.b * self.tx + next.d * self.ty + next.ty,
        }
    }

    /// The same transform applied around `origin` instead of (0, 0).
    #[must_use]
    pub fn around(&self, origin: Point) -> Self {
        Self::translation(-origin.x, -origin.y).then(self).then(&Self::translation(origin.x, origin.y))
    }

    #[must_use]
    pub fn apply(&self, point: Point) -> Point {
        Point::new(
            self.a * point.x + self.c * point.y + self.tx,
            self.b * point.x + self.d * point.y + self.ty,
        )
    }

    #[must_use]
    pub fn determinant(&self) -> f32 {
        self.a * self.d - self.b * self.c
    }

    #[must_use]
    pub fn invert(&self) -> Option<Self> {
        let det = self.determinant();
        if !det.is_finite() || det.abs() < 1e-9 {
            return None;
        }
        let a = self.d / det;
        let b = -self.b / det;
        let c = -self.c / det;
        let d = self.a / det;
        Some(Self { a, b, c, d, tx: -(a * self.tx + c * self.ty), ty: -(b * self.tx + d * self.ty) })
    }

    /// Axis-aligned bounds of a transformed rectangle.
    #[must_use]
    pub fn bounds(&self, rect: Rect) -> Rect {
        let corners = [
            self.apply(Point::new(rect.x, rect.y)),
            self.apply(Point::new(rect.right(), rect.y)),
            self.apply(Point::new(rect.right(), rect.bottom())),
            self.apply(Point::new(rect.x, rect.bottom())),
        ];
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for corner in corners {
            min_x = min_x.min(corner.x);
            min_y = min_y.min(corner.y);
            max_x = max_x.max(corner.x);
            max_y = max_y.max(corner.y);
        }
        Rect::new(min_x, min_y, max_x - min_x, max_y - min_y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_intersection_and_union() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        assert_eq!(a.intersect(b), Some(Rect::new(5.0, 5.0, 5.0, 5.0)));
        assert_eq!(a.union(b), Rect::new(0.0, 0.0, 15.0, 15.0));
        assert!(a.intersect(Rect::new(20.0, 0.0, 1.0, 1.0)).is_none());
        assert!(a.contains(Point::new(9.9, 0.0)) && !a.contains(Point::new(10.0, 0.0)));
    }

    #[test]
    fn affine_inverse_round_trips() {
        let transform = Affine::scaling(2.0, 3.0)
            .then(&Affine::rotation(30.0))
            .then(&Affine::translation(7.0, -4.0));
        let inverse = transform.invert().expect("invertible");
        let point = Point::new(3.5, -2.0);
        let back = inverse.apply(transform.apply(point));
        assert!((back.x - point.x).abs() < 1e-4 && (back.y - point.y).abs() < 1e-4);
        assert!(Affine::scaling(0.0, 1.0).invert().is_none());
    }

    #[test]
    fn transform_around_origin_keeps_the_origin_fixed() {
        let origin = Point::new(50.0, 20.0);
        let scaled = Affine::scaling(2.0, 2.0).around(origin);
        assert_eq!(scaled.apply(origin), origin);
    }

    #[test]
    fn snapping_keeps_adjacent_edges_together() {
        let left = Rect::new(0.0, 0.0, 33.3, 10.0).snapped(1.0);
        let right = Rect::new(33.3, 0.0, 33.3, 10.0).snapped(1.0);
        assert_eq!(left.right(), right.x);
    }
}
