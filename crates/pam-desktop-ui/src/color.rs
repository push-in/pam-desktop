//! sRGB colors. The wire carries colors as `0xAARRGGBB` integers.

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);
    pub const BLACK: Self = Self::rgba(0, 0, 0, 255);
    pub const WHITE: Self = Self::rgba(255, 255, 255, 255);

    #[must_use]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    #[must_use]
    pub const fn from_argb(argb: u32) -> Self {
        Self {
            a: (argb >> 24) as u8,
            r: (argb >> 16) as u8,
            g: (argb >> 8) as u8,
            b: argb as u8,
        }
    }

    /// Wire integers are signed 64-bit; only the low 32 bits carry the color.
    #[must_use]
    pub const fn from_wire(value: i64) -> Self {
        Self::from_argb(value as u32)
    }

    #[must_use]
    pub const fn to_argb(self) -> u32 {
        ((self.a as u32) << 24) | ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }

    #[must_use]
    pub const fn is_transparent(self) -> bool {
        self.a == 0
    }

    #[must_use]
    pub const fn is_opaque(self) -> bool {
        self.a == 255
    }

    #[must_use]
    pub fn with_alpha(self, alpha: f32) -> Self {
        Self { a: (f32::from(self.a) * alpha.clamp(0.0, 1.0)).round() as u8, ..self }
    }

    /// Non-premultiplied components in `0.0..=1.0`.
    #[must_use]
    pub fn to_f32(self) -> [f32; 4] {
        [
            f32::from(self.r) / 255.0,
            f32::from(self.g) / 255.0,
            f32::from(self.b) / 255.0,
            f32::from(self.a) / 255.0,
        ]
    }

    #[must_use]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        // Interpolate premultiplied so fading to transparent never darkens.
        let [r0, g0, b0, a0] = self.to_f32();
        let [r1, g1, b1, a1] = other.to_f32();
        let alpha = a0 + (a1 - a0) * t;
        if alpha <= f32::EPSILON {
            return Self::TRANSPARENT;
        }
        let channel = |c0: f32, c1: f32| {
            let value = (c0 * a0 + (c1 * a1 - c0 * a0) * t) / alpha;
            (value.clamp(0.0, 1.0) * 255.0).round() as u8
        };
        Self { r: channel(r0, r1), g: channel(g0, g1), b: channel(b0, b1), a: (alpha * 255.0).round() as u8 }
    }

    /// Perceived brightness in `0.0..=1.0` (used to pick readable foregrounds).
    #[must_use]
    pub fn luminance(self) -> f32 {
        let [r, g, b, _] = self.to_f32();
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argb_round_trip() {
        let color = Color::from_argb(0x80FF_7F00);
        assert_eq!(color, Color::rgba(255, 127, 0, 128));
        assert_eq!(color.to_argb(), 0x80FF_7F00);
        assert_eq!(Color::from_wire(-1), Color::WHITE);
    }

    #[test]
    fn lerp_is_premultiplied() {
        let faded = Color::rgba(255, 0, 0, 255).lerp(Color::TRANSPARENT, 0.5);
        assert_eq!(faded, Color::rgba(255, 0, 0, 128));
        assert_eq!(Color::BLACK.lerp(Color::WHITE, 0.0), Color::BLACK);
        assert_eq!(Color::BLACK.lerp(Color::WHITE, 1.0), Color::WHITE);
    }
}
