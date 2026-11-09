//! Skia's analytic anti-aliased path filler (`SkScan_AAAPath.cpp`, as in
//! Chromium 138), reduced to what a glyph mask needs: a closed path of lines
//! and quadratic curves filled with the non-zero rule into an A8 mask.
//!
//! Chromium on Linux renders glyph outlines with this rasterizer, not with
//! FreeType's: curve segments and edge ends are snapped to quarter pixels
//! vertically, coverage is computed per trapezoid with 8-bit fixed-point
//! approximations, and narrow rectangles use Skia's "fat rect" blit. The
//! arithmetic below follows Skia's integer code step by step (16.16 fixed
//! point, the same truncations and lookup tables), so the masks match
//! Chromium's to the level.

#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]

type Fixed = i32;
type FDot6 = i32;

const FIXED1: Fixed = 1 << 16;
const FIXED_HALF: Fixed = 1 << 15;
const MAX_S32: i32 = i32::MAX;
const MIN_S32: i32 = -i32::MAX;
/// `kDefaultAccuracy`: y is snapped to 1/4 pixel.
const ACCURACY: i32 = 2;
const INVERSE_TABLE_SIZE: i32 = 1024;
const MAX_COEFF_SHIFT: i32 = 6;
const NONE: usize = usize::MAX;

/// One path verb, in device pixels (y down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Verb {
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Close,
}

/// A filled A8 mask: `width * height` coverage bytes at (`left`, `top`).
#[derive(Debug, PartialEq, Eq)]
pub struct Mask {
    pub left: i32,
    pub top: i32,
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

fn fixed_mul(a: Fixed, b: Fixed) -> Fixed {
    ((i64::from(a) * i64::from(b)) >> 16) as Fixed
}

fn fixed_div(numer: i32, denom: i32) -> Fixed {
    ((i64::from(numer) << 16) / i64::from(denom)).clamp(i64::from(MIN_S32), i64::from(MAX_S32))
        as Fixed
}

fn fdot6_div(a: FDot6, b: FDot6) -> Fixed {
    if i16::try_from(a).is_ok() {
        a.wrapping_shl(16) / b
    } else {
        fixed_div(a, b)
    }
}

fn fdot6_to_fixed(x: FDot6) -> Fixed {
    x.wrapping_shl(10)
}

fn fixed_to_fdot6(x: Fixed) -> FDot6 {
    x >> 10
}

fn fixed_round_to_int(x: Fixed) -> i32 {
    x.wrapping_add(FIXED_HALF) >> 16
}

fn fixed_ceil_to_int(x: Fixed) -> i32 {
    x.wrapping_add(FIXED1 - 1) >> 16
}

fn fixed_floor_to_int(x: Fixed) -> i32 {
    x >> 16
}

fn fixed_round_to_fixed(x: Fixed) -> Fixed {
    ((x.wrapping_add(FIXED_HALF) as u32) & 0xFFFF_0000) as i32
}

fn fixed_ceil_to_fixed(x: Fixed) -> Fixed {
    ((x.wrapping_add(FIXED1 - 1) as u32) & 0xFFFF_0000) as i32
}

fn fixed_floor_to_fixed(x: Fixed) -> Fixed {
    ((x as u32) & 0xFFFF_0000) as i32
}

fn snap_y(y: Fixed) -> Fixed {
    let shift = 16 - ACCURACY;
    ((((y as u32).wrapping_add((FIXED1 >> (ACCURACY + 1)) as u32)) >> shift) << shift) as i32
}

fn quick_inverse(x: FDot6) -> Fixed {
    // Skia's table holds -(2^22 / n), truncated, for n = 1024..1 and 0.
    match x {
        0 => 0,
        x if x > 0 => (1 << 22) / x,
        x => -((1 << 22) / -x),
    }
}

fn quick_div(a: FDot6, b: FDot6) -> Fixed {
    const MIN_BITS: i32 = 3;
    const MAX_ABS_A: i32 = 1 << (31 - (22 - MIN_BITS));
    let (abs_a, abs_b) = (a.wrapping_abs(), b.wrapping_abs());
    if ((1 << MIN_BITS)..INVERSE_TABLE_SIZE).contains(&abs_b) && abs_a < MAX_ABS_A {
        a.wrapping_mul(quick_inverse(b)) >> 6
    } else {
        fdot6_div(a, b)
    }
}

fn scalar_to_fdot6(value: f32) -> FDot6 {
    (value * 64.0) as FDot6
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Line,
    Quad,
}

#[derive(Clone, Copy)]
struct Edge {
    prev: usize,
    next: usize,
    x: Fixed,
    dx: Fixed,
    upper_x: Fixed,
    y: Fixed,
    upper_y: Fixed,
    lower_y: Fixed,
    dy: Fixed,
    kind: Kind,
    curve_count: i8,
    curve_shift: u8,
    winding: i8,
    qx: Fixed,
    qy: Fixed,
    qdx: Fixed,
    qdy: Fixed,
    qddx: Fixed,
    qddy: Fixed,
    qlast_x: Fixed,
    qlast_y: Fixed,
    snapped_x: Fixed,
    snapped_y: Fixed,
}

impl Edge {
    const fn sentinel(x: Fixed, y: Fixed) -> Self {
        Self {
            prev: NONE,
            next: NONE,
            x,
            dx: 0,
            upper_x: x,
            y: 0,
            upper_y: y,
            lower_y: y,
            dy: MAX_S32,
            kind: Kind::Line,
            curve_count: 0,
            curve_shift: 0,
            winding: 1,
            qx: 0,
            qy: 0,
            qdx: 0,
            qdy: 0,
            qddx: 0,
            qddy: 0,
            qlast_x: 0,
            qlast_y: 0,
            snapped_x: 0,
            snapped_y: 0,
        }
    }

    fn set_line(p0: (f32, f32), p1: (f32, f32)) -> Option<Self> {
        let multiplier = (1 << ACCURACY) as f32;
        let mut x0 = fdot6_to_fixed(scalar_to_fdot6(p0.0 * multiplier)) >> ACCURACY;
        let mut y0 = snap_y(fdot6_to_fixed(scalar_to_fdot6(p0.1 * multiplier)) >> ACCURACY);
        let mut x1 = fdot6_to_fixed(scalar_to_fdot6(p1.0 * multiplier)) >> ACCURACY;
        let mut y1 = snap_y(fdot6_to_fixed(scalar_to_fdot6(p1.1 * multiplier)) >> ACCURACY);
        let mut winding = 1;
        if y0 > y1 {
            std::mem::swap(&mut x0, &mut x1);
            std::mem::swap(&mut y0, &mut y1);
            winding = -1;
        }
        let dy = fixed_to_fdot6(y1 - y0);
        if dy == 0 {
            return None;
        }
        let dx = fixed_to_fdot6(x1 - x0);
        let slope = quick_div(dx, dy);
        let abs_slope = slope.wrapping_abs();
        let mut edge = Self::sentinel(x0, y0);
        edge.dx = slope;
        edge.y = y0;
        edge.lower_y = y1;
        edge.dy = if dx == 0 || slope == 0 {
            MAX_S32
        } else if abs_slope < INVERSE_TABLE_SIZE {
            quick_inverse(abs_slope)
        } else {
            quick_div(dy, dx).wrapping_abs()
        };
        edge.winding = winding;
        Some(edge)
    }

    fn update_line(
        &mut self,
        mut x0: Fixed,
        mut y0: Fixed,
        mut x1: Fixed,
        mut y1: Fixed,
        slope: Fixed,
    ) -> bool {
        if y0 > y1 {
            std::mem::swap(&mut x0, &mut x1);
            std::mem::swap(&mut y0, &mut y1);
            self.winding = -self.winding;
        }
        let dx = fixed_to_fdot6(x1 - x0);
        let dy = fixed_to_fdot6(y1 - y0);
        if dy == 0 {
            return false;
        }
        let abs_slope = fixed_to_fdot6(slope).wrapping_abs();
        self.x = x0;
        self.dx = slope;
        self.upper_x = x0;
        self.y = y0;
        self.upper_y = y0;
        self.lower_y = y1;
        self.dy = if dx == 0 || slope == 0 {
            MAX_S32
        } else if abs_slope < INVERSE_TABLE_SIZE {
            quick_inverse(abs_slope)
        } else {
            quick_div(dy, dx).wrapping_abs()
        };
        true
    }

    fn set_quadratic(pts: [(f32, f32); 3]) -> Option<Self> {
        let scale = (1 << (ACCURACY + 6)) as f32;
        let fd = |value: f32| (value * scale) as FDot6;
        let (mut x0, mut y0) = (fd(pts[0].0), fd(pts[0].1));
        let (x1, y1) = (fd(pts[1].0), fd(pts[1].1));
        let (mut x2, mut y2) = (fd(pts[2].0), fd(pts[2].1));
        let mut winding = 1;
        if y0 > y2 {
            std::mem::swap(&mut x0, &mut x2);
            std::mem::swap(&mut y0, &mut y2);
            winding = -1;
        }
        let top = (y0 + 32) >> 6;
        let bottom = (y2 + 32) >> 6;
        if top == bottom {
            return None;
        }
        let mut shift = {
            let dx = (x1.wrapping_shl(1) - x0 - x2) >> 2;
            let dy = (y1.wrapping_shl(1) - y0 - y2) >> 2;
            diff_to_shift(dx, dy, ACCURACY)
        };
        if shift == 0 {
            shift = 1;
        } else if shift > MAX_COEFF_SHIFT {
            shift = MAX_COEFF_SHIFT;
        }
        let div2 = |value: FDot6| value.wrapping_shl(16 - 6 - 1);
        let mut edge = Self::sentinel(0, 0);
        edge.winding = winding;
        edge.kind = Kind::Quad;
        edge.curve_count = (1 << shift) as i8;
        edge.curve_shift = (shift - 1) as u8;
        let a = div2(x0 - x1 - x1 + x2);
        let b = fdot6_to_fixed(x1 - x0);
        edge.qx = fdot6_to_fixed(x0);
        edge.qdx = b + (a >> shift);
        edge.qddx = a >> (shift - 1);
        let a = div2(y0 - y1 - y1 + y2);
        let b = fdot6_to_fixed(y1 - y0);
        edge.qy = fdot6_to_fixed(y0);
        edge.qdy = b + (a >> shift);
        edge.qddy = a >> (shift - 1);
        edge.qlast_x = fdot6_to_fixed(x2);
        edge.qlast_y = fdot6_to_fixed(y2);
        edge.qx >>= ACCURACY;
        edge.qy >>= ACCURACY;
        edge.qdx >>= ACCURACY;
        edge.qdy >>= ACCURACY;
        edge.qddx >>= ACCURACY;
        edge.qddy >>= ACCURACY;
        edge.qlast_x >>= ACCURACY;
        edge.qlast_y >>= ACCURACY;
        edge.qy = snap_y(edge.qy);
        edge.qlast_y = snap_y(edge.qlast_y);
        edge.snapped_x = edge.qx;
        edge.snapped_y = edge.qy;
        edge.update_quadratic().then_some(edge)
    }

    fn update_quadratic(&mut self) -> bool {
        let mut success = false;
        let mut count = i32::from(self.curve_count);
        let mut oldx = self.qx;
        let mut oldy = self.qy;
        let mut dx = self.qdx;
        let mut dy = self.qdy;
        let shift = i32::from(self.curve_shift);
        let (mut newx, mut newy, mut new_snapped_x, mut new_snapped_y);
        loop {
            let slope;
            count -= 1;
            if count > 0 {
                newx = oldx + (dx >> shift);
                newy = oldy + (dy >> shift);
                if (dy >> shift).wrapping_abs() >= FIXED1 * 2
                    && (i64::from(dy.wrapping_abs()) << 6) > i64::from(dx.wrapping_abs())
                {
                    let diff_y = fixed_to_fdot6(newy - self.snapped_y);
                    slope = if diff_y == 0 {
                        MAX_S32
                    } else {
                        quick_div(fixed_to_fdot6(newx - self.snapped_x), diff_y)
                    };
                    new_snapped_y = self.qlast_y.min(fixed_round_to_fixed(newy));
                    new_snapped_x = newx - fixed_mul(slope, newy - new_snapped_y);
                } else {
                    new_snapped_y = self.qlast_y.min(snap_y(newy));
                    new_snapped_x = newx;
                    let diff_y = fixed_to_fdot6(new_snapped_y - self.snapped_y);
                    slope = if diff_y == 0 {
                        MAX_S32
                    } else {
                        quick_div(fixed_to_fdot6(newx - self.snapped_x), diff_y)
                    };
                }
                dx += self.qddx;
                dy += self.qddy;
            } else {
                newx = self.qlast_x;
                newy = self.qlast_y;
                new_snapped_y = newy;
                new_snapped_x = newx;
                let diff_y = fixed_to_fdot6(newy - self.snapped_y);
                slope = if diff_y == 0 {
                    MAX_S32
                } else {
                    quick_div(fixed_to_fdot6(newx - self.snapped_x), diff_y)
                };
            }
            if slope < MAX_S32 {
                success = self.update_line(
                    self.snapped_x,
                    self.snapped_y,
                    new_snapped_x,
                    new_snapped_y,
                    slope,
                );
            }
            oldx = newx;
            oldy = newy;
            if count <= 0 || success {
                break;
            }
        }
        self.qx = newx;
        self.qy = newy;
        self.qdx = dx;
        self.qdy = dy;
        self.snapped_x = new_snapped_x;
        self.snapped_y = new_snapped_y;
        self.curve_count = count as i8;
        success
    }

    fn update(&mut self) -> bool {
        self.curve_count > 0 && self.update_quadratic()
    }

    fn keep_continuous(&mut self) {
        self.snapped_x = self.x;
        self.snapped_y = self.y;
    }

    fn go_y(&mut self, y: Fixed) {
        if y == self.y.wrapping_add(FIXED1) {
            self.x = self.x.wrapping_add(self.dx);
            self.y = y;
        } else if y != self.y {
            self.x = self
                .upper_x
                .wrapping_add(fixed_mul(self.dx, y.wrapping_sub(self.upper_y)));
            self.y = y;
        }
    }

    fn go_y_shift(&mut self, y: Fixed, shift: i32) {
        self.y = y;
        self.x = self.x.wrapping_add(self.dx >> shift);
    }
}

fn cheap_distance(dx: FDot6, dy: FDot6) -> FDot6 {
    let (dx, dy) = (dx.wrapping_abs(), dy.wrapping_abs());
    if dx > dy {
        dx + (dy >> 1)
    } else {
        dy + (dx >> 1)
    }
}

fn diff_to_shift(dx: FDot6, dy: FDot6, shift_aa: i32) -> i32 {
    let dist = cheap_distance(dx, dy);
    let dist = (dist + (1 << (2 + shift_aa))) >> (3 + shift_aa);
    (32 - (dist as u32).leading_zeros() as i32) >> 1
}

/// The A8 mask being filled (`MaskAdditiveBlitter`; the RLE blitters Skia
/// uses for larger masks accumulate the same way).
struct Coverage {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    data: Vec<u8>,
}

impl Coverage {
    fn index(&self, x: i32, y: i32) -> Option<usize> {
        let (x, y) = (x - self.left, y - self.top);
        (x >= 0 && y >= 0 && x < self.width && y < self.height)
            .then(|| (y * self.width + x) as usize)
    }

    /// `add_alpha`: wraps 256 to 255 like `SkAlphaRuns::CatchOverflow`.
    fn add(&mut self, x: i32, y: i32, delta: u8) {
        if let Some(index) = self.index(x, y) {
            let sum = u32::from(self.data[index]) + u32::from(delta);
            self.data[index] = (sum - (sum >> 8)) as u8;
        }
    }

    fn add_saturating(&mut self, x: i32, y: i32, delta: u8) {
        if let Some(index) = self.index(x, y) {
            self.data[index] = self.data[index].saturating_add(delta);
        }
    }

    fn set(&mut self, x: i32, y: i32, alpha: u8) {
        if let Some(index) = self.index(x, y) {
            self.data[index] = alpha;
        }
    }

    fn blit_anti_h(&mut self, x: i32, y: i32, width: i32, alpha: u8) {
        for i in 0..width {
            self.add(x + i, y, alpha);
        }
    }

    fn blit_v(&mut self, x: i32, y: i32, height: i32, alpha: u8) {
        if alpha == 0 {
            return;
        }
        for i in 0..height {
            self.set(x, y + i, alpha);
        }
    }

    fn blit_rect(&mut self, x: i32, y: i32, width: i32, height: i32) {
        for j in 0..height {
            for i in 0..width {
                self.set(x + i, y + j, 0xFF);
            }
        }
    }

    fn blit_anti_rect(&mut self, x: i32, y: i32, width: i32, height: i32, left: u8, right: u8) {
        self.blit_v(x, y, height, left);
        self.blit_v(x + 1 + width, y, height, right);
        self.blit_rect(x + 1, y, width, height);
    }
}

fn trapezoid_to_alpha(l1: Fixed, l2: Fixed) -> u8 {
    ((l1.wrapping_add(l2) / 2) >> 8) as u8
}

fn partial_triangle_to_alpha(a: Fixed, b: Fixed) -> u8 {
    let area = (a >> 11).wrapping_mul(a >> 11).wrapping_mul(b >> 11);
    ((area >> 8) & 0xFF) as u8
}

fn partial_alpha_fixed(alpha: u8, height: Fixed) -> u8 {
    fixed_round_to_int(i32::from(alpha).wrapping_mul(height)) as u8
}

fn partial_alpha(alpha: u8, full: u8) -> u8 {
    ((u32::from(alpha) * u32::from(full)) >> 8) as u8
}

fn fixed_to_alpha(f: Fixed) -> u8 {
    partial_alpha_fixed(0xFF, f)
}

fn approximate_intersection(mut l1: Fixed, mut r1: Fixed, mut l2: Fixed, mut r2: Fixed) -> Fixed {
    if l1 > r1 {
        std::mem::swap(&mut l1, &mut r1);
    }
    if l2 > r2 {
        std::mem::swap(&mut l2, &mut r2);
    }
    (l1.max(l2).wrapping_add(r1.min(r2))) / 2
}

fn compute_alpha_above_line(alphas: &mut [u8], l: Fixed, r: Fixed, dy: Fixed, full: u8) {
    let big_r = fixed_ceil_to_int(r);
    if big_r == 0 {
    } else if big_r == 1 {
        alphas[0] = partial_alpha((((big_r << 17) - l - r) >> 9) as u8, full);
    } else {
        let first = FIXED1 - l;
        let last = r - ((big_r - 1) << 16);
        let first_h = fixed_mul(first, dy);
        alphas[0] = (fixed_mul(first, first_h) >> 9) as u8;
        let mut alpha16 = first_h.saturating_add(dy >> 1);
        for alpha in alphas.iter_mut().take((big_r - 1) as usize).skip(1) {
            *alpha = (alpha16 >> 8) as u8;
            alpha16 = alpha16.saturating_add(dy);
        }
        alphas[(big_r - 1) as usize] = full.wrapping_sub(partial_triangle_to_alpha(last, dy));
    }
}

fn compute_alpha_below_line(alphas: &mut [u8], l: Fixed, r: Fixed, dy: Fixed, full: u8) {
    let big_r = fixed_ceil_to_int(r);
    if big_r == 0 {
    } else if big_r == 1 {
        alphas[0] = partial_alpha(trapezoid_to_alpha(l, r), full);
    } else {
        let first = FIXED1 - l;
        let last = r - ((big_r - 1) << 16);
        let last_h = fixed_mul(last, dy);
        alphas[(big_r - 1) as usize] = (fixed_mul(last, last_h) >> 9) as u8;
        let mut alpha16 = last_h.saturating_add(dy >> 1);
        let mut i = big_r - 2;
        while i > 0 {
            alphas[i as usize] = ((alpha16 >> 8) & 0xFF) as u8;
            alpha16 = alpha16.saturating_add(dy);
            i -= 1;
        }
        alphas[0] = full.wrapping_sub(partial_triangle_to_alpha(first, dy));
    }
}

fn blit_single_alpha(mask: &mut Coverage, y: i32, x: i32, alpha: u8, full: u8, no_real: bool) {
    if full == 0xFF && !no_real {
        mask.set(x, y, alpha);
    } else {
        mask.add_saturating(x, y, partial_alpha(alpha, full));
    }
}

fn blit_two_alphas(mask: &mut Coverage, y: i32, x: i32, a1: u8, a2: u8) {
    mask.add_saturating(x, y, a1);
    mask.add_saturating(x + 1, y, a2);
}

fn blit_full_alpha(mask: &mut Coverage, y: i32, x: i32, len: i32, full: u8) {
    for i in 0..len {
        mask.add_saturating(x + i, y, full);
    }
}

#[allow(clippy::too_many_arguments)]
fn blit_aaa_trapezoid_row(
    mask: &mut Coverage,
    y: i32,
    ul: Fixed,
    ur: Fixed,
    ll: Fixed,
    lr: Fixed,
    l_dy: Fixed,
    r_dy: Fixed,
    full: u8,
    no_real: bool,
) {
    let big_l = fixed_floor_to_int(ul);
    let big_r = fixed_ceil_to_int(lr);
    let len = big_r - big_l;
    if len == 1 {
        let alpha = trapezoid_to_alpha(ur - ul, lr - ll);
        blit_single_alpha(mask, y, big_l, alpha, full, no_real);
        return;
    }
    if len <= 0 {
        return;
    }
    let len_u = len as usize;
    let mut alphas = vec![full; len_u + 1];
    let mut temp = vec![0u8; len_u + 2];
    let u_l = fixed_floor_to_int(ul);
    let l_l = fixed_ceil_to_int(ll);
    if u_l + 2 == l_l {
        let first = (u_l << 16) + FIXED1 - ul;
        let second = ll - ul - first;
        let a1 = full.wrapping_sub(partial_triangle_to_alpha(first, l_dy));
        let a2 = partial_triangle_to_alpha(second, l_dy);
        alphas[0] = alphas[0].saturating_sub(a1);
        alphas[1] = alphas[1].saturating_sub(a2);
    } else {
        let start = (u_l - big_l) as usize;
        compute_alpha_below_line(
            &mut temp[start..],
            ul - (u_l << 16),
            ll - (u_l << 16),
            l_dy,
            full,
        );
        for i in u_l..l_l {
            let index = (i - big_l) as usize;
            if index < len_u {
                alphas[index] = alphas[index].saturating_sub(temp[index]);
            }
        }
    }
    let u_r = fixed_floor_to_int(ur);
    let l_r = fixed_ceil_to_int(lr);
    if u_r + 2 == l_r {
        let first = (u_r << 16) + FIXED1 - ur;
        let second = lr - ur - first;
        let a1 = partial_triangle_to_alpha(first, r_dy);
        let a2 = full.wrapping_sub(partial_triangle_to_alpha(second, r_dy));
        alphas[len_u - 2] = alphas[len_u - 2].saturating_sub(a1);
        alphas[len_u - 1] = alphas[len_u - 1].saturating_sub(a2);
    } else {
        temp.iter_mut().for_each(|value| *value = 0);
        let start = (u_r - big_l) as usize;
        if start < temp.len() {
            compute_alpha_above_line(
                &mut temp[start..],
                ur - (u_r << 16),
                lr - (u_r << 16),
                r_dy,
                full,
            );
        }
        for i in u_r..l_r {
            let index = (i - big_l) as usize;
            if index < len_u {
                alphas[index] = alphas[index].saturating_sub(temp[index]);
            }
        }
    }
    for (i, alpha) in alphas.iter().take(len_u).enumerate() {
        mask.add_saturating(big_l + i as i32, y, *alpha);
    }
}

#[allow(clippy::too_many_arguments)]
fn blit_trapezoid_row(
    mask: &mut Coverage,
    y: i32,
    mut ul: Fixed,
    mut ur: Fixed,
    mut ll: Fixed,
    mut lr: Fixed,
    l_dy: Fixed,
    r_dy: Fixed,
    full: u8,
    no_real: bool,
) {
    if ul > ur {
        return;
    }
    if ll > lr {
        let x = approximate_intersection(ul, ll, ur, lr);
        ll = x;
        lr = x;
    }
    if ul == ur && ll == lr {
        return;
    }
    if ul > ll {
        std::mem::swap(&mut ul, &mut ll);
    }
    if ur > lr {
        std::mem::swap(&mut ur, &mut lr);
    }
    let join_left = fixed_ceil_to_fixed(ll);
    let join_rite = fixed_floor_to_fixed(ur);
    if join_left <= join_rite {
        if ul < join_left {
            let len = fixed_ceil_to_int(join_left - ul);
            if len == 1 {
                let alpha = trapezoid_to_alpha(join_left - ul, join_left - ll);
                blit_single_alpha(mask, y, ul >> 16, alpha, full, no_real);
            } else if len == 2 {
                let first = join_left - FIXED1 - ul;
                let second = ll - ul - first;
                let a1 = partial_triangle_to_alpha(first, l_dy);
                let a2 = full.wrapping_sub(partial_triangle_to_alpha(second, l_dy));
                blit_two_alphas(mask, y, ul >> 16, a1, a2);
            } else {
                blit_aaa_trapezoid_row(
                    mask, y, ul, join_left, ll, join_left, l_dy, MAX_S32, full, no_real,
                );
            }
        }
        if join_left < join_rite {
            blit_full_alpha(
                mask,
                y,
                fixed_floor_to_int(join_left),
                fixed_floor_to_int(join_rite - join_left),
                full,
            );
        }
        if lr > join_rite {
            let len = fixed_ceil_to_int(lr - join_rite);
            if len == 1 {
                let alpha = trapezoid_to_alpha(ur - join_rite, lr - join_rite);
                blit_single_alpha(mask, y, join_rite >> 16, alpha, full, no_real);
            } else if len == 2 {
                let first = join_rite + FIXED1 - ur;
                let second = lr - ur - first;
                let a1 = full.wrapping_sub(partial_triangle_to_alpha(first, r_dy));
                let a2 = partial_triangle_to_alpha(second, r_dy);
                blit_two_alphas(mask, y, join_rite >> 16, a1, a2);
            } else {
                blit_aaa_trapezoid_row(
                    mask, y, join_rite, ur, join_rite, lr, MAX_S32, r_dy, full, no_real,
                );
            }
        }
    } else {
        blit_aaa_trapezoid_row(mask, y, ul, ur, ll, lr, l_dy, r_dy, full, no_real);
    }
}

/// The edge list: index 0 is the head sentinel, 1 the tail sentinel.
struct Edges {
    list: Vec<Edge>,
}

const HEAD: usize = 0;
const TAIL: usize = 1;

impl Edges {
    fn remove(&mut self, edge: usize) {
        let (prev, next) = (self.list[edge].prev, self.list[edge].next);
        self.list[prev].next = next;
        self.list[next].prev = prev;
    }

    fn insert_after(&mut self, edge: usize, after: usize) {
        let next = self.list[after].next;
        self.list[edge].prev = after;
        self.list[edge].next = next;
        self.list[next].prev = edge;
        self.list[after].next = edge;
    }

    fn backward_insert_based_on_x(&mut self, edge: usize) {
        let x = self.list[edge].x;
        let mut prev = self.list[edge].prev;
        while self.list[prev].prev != NONE && self.list[prev].x > x {
            prev = self.list[prev].prev;
        }
        if self.list[prev].next != edge {
            self.remove(edge);
            self.insert_after(edge, prev);
        }
    }

    fn backward_insert_start(&self, mut prev: usize, x: Fixed) -> usize {
        while self.list[prev].prev != NONE && self.list[prev].x > x {
            prev = self.list[prev].prev;
        }
        prev
    }

    fn check_intersection(&self, edge: usize, next_y: Fixed, next_next_y: &mut Fixed) {
        let prev = self.list[edge].prev;
        let e = &self.list[edge];
        let p = &self.list[prev];
        if p.prev != NONE && p.x.wrapping_add(p.dx) > e.x.wrapping_add(e.dx) {
            *next_next_y = next_y + (FIXED1 >> ACCURACY);
        }
    }

    fn check_intersection_fwd(&self, edge: usize, next_y: Fixed, next_next_y: &mut Fixed) {
        let next = self.list[edge].next;
        let e = &self.list[edge];
        let n = &self.list[next];
        if n.next != NONE && e.x.wrapping_add(e.dx) > n.x.wrapping_add(n.dx) {
            *next_next_y = next_y + (FIXED1 >> ACCURACY);
        }
    }

    fn insert_new_edges(&mut self, mut new_edge: usize, y: Fixed, next_next_y: &mut Fixed) {
        if self.list[new_edge].upper_y > y {
            update_next_next_y(self.list[new_edge].upper_y, y, next_next_y);
            return;
        }
        let prev = self.list[new_edge].prev;
        if self.list[prev].x <= self.list[new_edge].x {
            while self.list[new_edge].upper_y <= y {
                self.check_intersection(new_edge, y, next_next_y);
                update_next_next_y(self.list[new_edge].lower_y, y, next_next_y);
                new_edge = self.list[new_edge].next;
            }
            update_next_next_y(self.list[new_edge].upper_y, y, next_next_y);
            return;
        }
        let mut start = self.backward_insert_start(prev, self.list[new_edge].x);
        loop {
            let next = self.list[new_edge].next;
            let mut in_place = false;
            loop {
                if self.list[start].next == new_edge {
                    in_place = true;
                    break;
                }
                let after = self.list[start].next;
                if self.list[after].x >= self.list[new_edge].x {
                    break;
                }
                start = after;
            }
            if !in_place {
                self.remove(new_edge);
                self.insert_after(new_edge, start);
            }
            self.check_intersection(new_edge, y, next_next_y);
            self.check_intersection_fwd(new_edge, y, next_next_y);
            update_next_next_y(self.list[new_edge].lower_y, y, next_next_y);
            start = new_edge;
            new_edge = next;
            if self.list[new_edge].upper_y > y {
                break;
            }
        }
        update_next_next_y(self.list[new_edge].upper_y, y, next_next_y);
    }

    fn edges_too_close(&self, prev: usize, next: usize, lower_y: Fixed) -> bool {
        if prev == NONE || next == NONE {
            return false;
        }
        let (p, n) = (&self.list[prev], &self.list[next]);
        n.upper_y < lower_y && p.x.saturating_add(FIXED1) >= n.x.wrapping_sub(n.dx.wrapping_abs())
    }

    fn is_smooth_enough_one(&self, this: usize, next: usize) -> bool {
        let e = &self.list[this];
        if e.curve_count > 0 {
            return e.qdx.wrapping_abs() >> 1 >= e.qddx.wrapping_abs()
                && e.qdy.wrapping_abs() >> 1 >= e.qddy.wrapping_abs()
                && (e.qdy - e.qddy) >> e.curve_shift >= FIXED1;
        }
        let n = &self.list[next];
        n.dx.saturating_sub(e.dx).wrapping_abs() <= FIXED1
            && n.lower_y.wrapping_sub(n.upper_y) >= FIXED1
    }

    fn is_smooth_enough(&self, left: usize, rite: usize, curr: usize, stop_y: i32) -> bool {
        if self.list[curr].upper_y >= stop_y.wrapping_shl(16) {
            return false;
        }
        let (l, r) = (&self.list[left], &self.list[rite]);
        if l.lower_y.saturating_add(FIXED1) < r.lower_y {
            return self.is_smooth_enough_one(left, curr);
        } else if l.lower_y > r.lower_y.saturating_add(FIXED1) {
            return self.is_smooth_enough_one(rite, curr);
        }
        let mut curr = curr;
        let mut next_curr = self.list[curr].next;
        if next_curr == NONE || self.list[next_curr].upper_y >= stop_y.wrapping_shl(16) {
            return false;
        }
        if self.list[next_curr].upper_x < self.list[curr].upper_x {
            std::mem::swap(&mut curr, &mut next_curr);
        }
        self.is_smooth_enough_one(left, curr) && self.is_smooth_enough_one(rite, next_curr)
    }
}

fn update_next_next_y(y: Fixed, next_y: Fixed, next_next_y: &mut Fixed) {
    if y > next_y && y < *next_next_y {
        *next_next_y = y;
    }
}

fn edges_too_close_rite(prev_rite: i32, ul: Fixed, ll: Fixed) -> bool {
    prev_rite > fixed_floor_to_int(ul) || prev_rite > fixed_floor_to_int(ll)
}

#[allow(clippy::too_many_lines)]
fn walk_edges(
    edges: &mut Edges,
    mask: &mut Coverage,
    start_y: i32,
    stop_y: i32,
    left_clip: Fixed,
    right_clip: Fixed,
    skip_intersect: bool,
) {
    edges.list[HEAD].x = left_clip;
    edges.list[HEAD].upper_x = left_clip;
    edges.list[TAIL].x = right_clip;
    edges.list[TAIL].upper_x = right_clip;
    let first = edges.list[HEAD].next;
    let mut y = edges.list[first].upper_y.max(start_y << 16);
    let mut next_next_y = MAX_S32;
    {
        let mut edge = first;
        while edges.list[edge].upper_y <= y {
            edges.list[edge].go_y(y);
            update_next_next_y(edges.list[edge].lower_y, y, &mut next_next_y);
            edge = edges.list[edge].next;
        }
        update_next_next_y(edges.list[edge].upper_y, y, &mut next_next_y);
    }
    loop {
        let mut w: i32 = 0;
        let mut in_interval = false;
        let mut prev_x = edges.list[HEAD].x;
        let mut next_y = next_next_y.min(fixed_ceil_to_fixed(y + 1));
        let mut curr = edges.list[HEAD].next;
        let mut left_e = HEAD;
        let mut left = left_clip;
        let mut left_dy = 0;
        let mut prev_rite = fixed_floor_to_int(left_clip);
        next_next_y = MAX_S32;
        let mut y_shift = 0;
        if (next_y - y) & (FIXED1 >> 2) != 0 {
            y_shift = 2;
            next_y = y + (FIXED1 >> 2);
        } else if (next_y - y) & (FIXED1 >> 1) != 0 {
            y_shift = 1;
        }
        let full = fixed_to_alpha(next_y - y);
        while edges.list[curr].upper_y <= y {
            w += i32::from(edges.list[curr].winding);
            let prev_in_interval = in_interval;
            // Non-zero winding.
            in_interval = w != 0;
            let is_left = in_interval && !prev_in_interval;
            let is_rite = !in_interval && prev_in_interval;
            if is_rite {
                let mut rite = edges.list[curr].x;
                edges.list[curr].go_y_shift(next_y, y_shift);
                let next_left = left_clip.max(edges.list[left_e].x);
                rite = right_clip.min(rite);
                let next_rite = right_clip.min(edges.list[curr].x);
                let no_real = full == 0xFF
                    && (edges_too_close_rite(prev_rite, left, edges.list[left_e].x)
                        || edges.edges_too_close(curr, edges.list[curr].next, next_y));
                blit_trapezoid_row(
                    mask,
                    y >> 16,
                    left,
                    rite,
                    next_left,
                    next_rite,
                    left_dy,
                    edges.list[curr].dy,
                    full,
                    no_real,
                );
                prev_rite = fixed_ceil_to_int(rite.max(edges.list[curr].x));
            } else {
                if is_left {
                    left = edges.list[curr].x.max(left_clip);
                    left_dy = edges.list[curr].dy;
                    left_e = curr;
                }
                edges.list[curr].go_y_shift(next_y, y_shift);
            }
            let next = edges.list[curr].next;
            while edges.list[curr].lower_y <= next_y {
                if edges.list[curr].curve_count > 0 {
                    edges.list[curr].keep_continuous();
                    if !edges.list[curr].update_quadratic() {
                        break;
                    }
                } else {
                    break;
                }
            }
            if edges.list[curr].lower_y <= next_y {
                edges.remove(curr);
            } else {
                update_next_next_y(edges.list[curr].lower_y, next_y, &mut next_next_y);
                let new_x = edges.list[curr].x;
                if new_x < prev_x {
                    edges.backward_insert_based_on_x(curr);
                } else {
                    prev_x = new_x;
                }
                if !skip_intersect {
                    edges.check_intersection(curr, next_y, &mut next_next_y);
                }
            }
            curr = next;
        }
        if in_interval {
            let no_real =
                full == 0xFF && edges.edges_too_close(edges.list[left_e].prev, left_e, next_y);
            let next_left = left_clip.max(edges.list[left_e].x);
            blit_trapezoid_row(
                mask,
                y >> 16,
                left,
                right_clip,
                next_left,
                right_clip,
                left_dy,
                0,
                full,
                no_real,
            );
        }
        y = next_y;
        if y >= stop_y << 16 {
            break;
        }
        edges.insert_new_edges(curr, y, &mut next_next_y);
    }
}

#[allow(clippy::too_many_lines)]
fn walk_convex_edges(
    edges: &mut Edges,
    mask: &mut Coverage,
    stop_y: i32,
    left_bound: Fixed,
    rite_bound: Fixed,
) {
    let mut left_e = edges.list[HEAD].next;
    let mut rite_e = edges.list[left_e].next;
    let mut curr = edges.list[rite_e].next;
    let mut y = edges.list[left_e].upper_y.max(edges.list[rite_e].upper_y);
    loop {
        while edges.list[left_e].lower_y <= y {
            if !edges.list[left_e].update() {
                if fixed_floor_to_int(edges.list[curr].upper_y) >= stop_y {
                    return;
                }
                left_e = curr;
                curr = edges.list[curr].next;
            }
        }
        while edges.list[rite_e].lower_y <= y {
            if !edges.list[rite_e].update() {
                if fixed_floor_to_int(edges.list[curr].upper_y) >= stop_y {
                    return;
                }
                rite_e = curr;
                curr = edges.list[curr].next;
            }
        }
        if fixed_floor_to_int(y) >= stop_y {
            return;
        }
        edges.list[left_e].go_y(y);
        edges.list[rite_e].go_y(y);
        {
            let (l, r) = (&edges.list[left_e], &edges.list[rite_e]);
            if l.x > r.x || (l.x == r.x && l.dx > r.dx) {
                std::mem::swap(&mut left_e, &mut rite_e);
            }
        }
        let mut local_bot = edges.list[left_e].lower_y.min(edges.list[rite_e].lower_y);
        if edges.is_smooth_enough(left_e, rite_e, curr, stop_y) {
            local_bot = fixed_ceil_to_fixed(local_bot);
        }
        local_bot = local_bot.min(stop_y << 16);
        let mut left = left_bound.max(edges.list[left_e].x);
        let d_left = edges.list[left_e].dx;
        let mut rite = rite_bound.min(edges.list[rite_e].x);
        let d_rite = edges.list[rite_e].dx;
        if (d_left | d_rite) == 0 {
            let full_left = fixed_ceil_to_int(left);
            let full_rite = fixed_floor_to_int(rite);
            let partial_left = (full_left << 16) - left;
            let partial_rite = rite - (full_rite << 16);
            let full_top = fixed_ceil_to_int(y);
            let full_bot = fixed_floor_to_int(local_bot);
            let mut partial_top = (full_top << 16) - y;
            let mut partial_bot = local_bot - (full_bot << 16);
            if full_top > full_bot {
                partial_top -= FIXED1 - partial_bot;
                partial_bot = 0;
            }
            if full_rite >= full_left {
                if partial_top > 0 {
                    if partial_left > 0 {
                        mask.blit_anti_h(
                            full_left - 1,
                            full_top - 1,
                            1,
                            fixed_to_alpha(fixed_mul(partial_top, partial_left)),
                        );
                    }
                    mask.blit_anti_h(
                        full_left,
                        full_top - 1,
                        full_rite - full_left,
                        fixed_to_alpha(partial_top),
                    );
                    if partial_rite > 0 {
                        mask.blit_anti_h(
                            full_rite,
                            full_top - 1,
                            1,
                            fixed_to_alpha(fixed_mul(partial_top, partial_rite)),
                        );
                    }
                }
                if full_bot > full_top
                    && (full_rite > full_left
                        || fixed_to_alpha(partial_left) > 0
                        || fixed_to_alpha(partial_rite) > 0)
                {
                    mask.blit_anti_rect(
                        full_left - 1,
                        full_top,
                        full_rite - full_left,
                        full_bot - full_top,
                        fixed_to_alpha(partial_left),
                        fixed_to_alpha(partial_rite),
                    );
                }
                if partial_bot > 0 {
                    if partial_left > 0 {
                        mask.blit_anti_h(
                            full_left - 1,
                            full_bot,
                            1,
                            fixed_to_alpha(fixed_mul(partial_bot, partial_left)),
                        );
                    }
                    mask.blit_anti_h(
                        full_left,
                        full_bot,
                        full_rite - full_left,
                        fixed_to_alpha(partial_bot),
                    );
                    if partial_rite > 0 {
                        mask.blit_anti_h(
                            full_rite,
                            full_bot,
                            1,
                            fixed_to_alpha(fixed_mul(partial_bot, partial_rite)),
                        );
                    }
                }
            } else {
                let width = rite - left;
                if width > 0 {
                    if partial_top > 0 {
                        mask.blit_anti_h(
                            full_left - 1,
                            full_top - 1,
                            1,
                            fixed_to_alpha(fixed_mul(partial_top, width)),
                        );
                    }
                    if full_bot > full_top {
                        mask.blit_v(
                            full_left - 1,
                            full_top,
                            full_bot - full_top,
                            fixed_to_alpha(width),
                        );
                    }
                    if partial_bot > 0 {
                        mask.blit_anti_h(
                            full_left - 1,
                            full_bot,
                            1,
                            fixed_to_alpha(fixed_mul(partial_bot, width)),
                        );
                    }
                }
            }
            y = local_bot;
        } else {
            const SNAP_DIGIT: Fixed = FIXED1 >> 4;
            const SNAP_HALF: Fixed = SNAP_DIGIT >> 1;
            const SNAP_MASK: Fixed = -1 ^ (SNAP_DIGIT - 1);
            left += SNAP_HALF;
            rite += SNAP_HALF;
            let mut count = fixed_ceil_to_int(local_bot) - fixed_floor_to_int(y);
            let (l_dy, r_dy) = (edges.list[left_e].dy, edges.list[rite_e].dy);
            if count > 1 {
                if (y as u32 & 0xFFFF_0000) as i32 != y {
                    count -= 1;
                    let next_y = fixed_ceil_to_fixed(y + 1);
                    let dy = next_y - y;
                    let next_left = left + fixed_mul(d_left, dy);
                    let next_rite = rite + fixed_mul(d_rite, dy);
                    blit_trapezoid_row(
                        mask,
                        y >> 16,
                        left & SNAP_MASK,
                        rite & SNAP_MASK,
                        next_left & SNAP_MASK,
                        next_rite & SNAP_MASK,
                        l_dy,
                        r_dy,
                        partial_alpha_fixed(0xFF, dy),
                        false,
                    );
                    left = next_left;
                    rite = next_rite;
                    y = next_y;
                }
                while count > 1 {
                    count -= 1;
                    let next_y = y + FIXED1;
                    let next_left = left + d_left;
                    let next_rite = rite + d_rite;
                    blit_trapezoid_row(
                        mask,
                        y >> 16,
                        left & SNAP_MASK,
                        rite & SNAP_MASK,
                        next_left & SNAP_MASK,
                        next_rite & SNAP_MASK,
                        l_dy,
                        r_dy,
                        0xFF,
                        false,
                    );
                    left = next_left;
                    rite = next_rite;
                    y = next_y;
                }
            }
            let dy = local_bot - y;
            let next_left = (left + fixed_mul(d_left, dy)).max(left_bound + SNAP_HALF);
            let next_rite = (rite + fixed_mul(d_rite, dy)).min(rite_bound + SNAP_HALF);
            blit_trapezoid_row(
                mask,
                y >> 16,
                left & SNAP_MASK,
                rite & SNAP_MASK,
                next_left & SNAP_MASK,
                next_rite & SNAP_MASK,
                l_dy,
                r_dy,
                partial_alpha_fixed(0xFF, dy),
                false,
            );
            left = next_left - SNAP_HALF;
            rite = next_rite - SNAP_HALF;
            y = local_bot;
        }
        edges.list[left_e].x = left;
        edges.list[rite_e].x = rite;
        edges.list[left_e].y = y;
        edges.list[rite_e].y = y;
    }
}

/// `SkChopQuadAtYExtrema`: one or two y-monotonic quads.
fn chop_quad_at_y_extrema(src: [(f32, f32); 3]) -> Vec<[(f32, f32); 3]> {
    let (a, mut b, c) = (src[0].1, src[1].1, src[2].1);
    let ab = a - b;
    let bc = if ab < 0.0 { -(b - c) } else { b - c };
    if ab == 0.0 || bc < 0.0 {
        // Not monotonic: chop at the extremum.
        let (mut numer, mut denom) = (a - b, a - b - b + c);
        if numer < 0.0 {
            numer = -numer;
            denom = -denom;
        }
        if denom != 0.0 && numer != 0.0 && numer < denom {
            let t = numer / denom;
            if t.is_finite() && t != 0.0 {
                let lerp =
                    |p: (f32, f32), q: (f32, f32)| (p.0 + (q.0 - p.0) * t, p.1 + (q.1 - p.1) * t);
                let p01 = lerp(src[0], src[1]);
                let p12 = lerp(src[1], src[2]);
                let mid = lerp(p01, p12);
                return vec![[src[0], (p01.0, mid.1), mid], [mid, (p12.0, mid.1), src[2]]];
            }
        }
        b = if (a - b).abs() < (b - c).abs() { a } else { c };
    }
    vec![[src[0], (src[1].0, b), src[2]]]
}

/// Single closed contour of lines whose corners are axis aligned:
/// `SkPath::isRect` for the shapes glyph outlines have.
fn as_rect(contours: &[Vec<Segment>]) -> Option<(f32, f32, f32, f32)> {
    let [contour] = contours else { return None };
    let mut points: Vec<(f32, f32)> = Vec::new();
    for segment in contour {
        let Segment::Line(_, p1) = segment else {
            return None;
        };
        if points.last() != Some(p1) {
            points.push(*p1);
        }
    }
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    // Drop collinear points.
    let mut corners = Vec::new();
    let n = points.len();
    for i in 0..n {
        let (p, q, r) = (points[(i + n - 1) % n], points[i], points[(i + 1) % n]);
        let cross = (q.0 - p.0) * (r.1 - q.1) - (q.1 - p.1) * (r.0 - q.0);
        if cross != 0.0 {
            corners.push(q);
        }
    }
    if corners.len() != 4 {
        return None;
    }
    let xs: Vec<f32> = corners.iter().map(|p| p.0).collect();
    let ys: Vec<f32> = corners.iter().map(|p| p.1).collect();
    for i in 0..4 {
        let (p, q) = (corners[i], corners[(i + 1) % 4]);
        if p.0 != q.0 && p.1 != q.1 {
            return None;
        }
    }
    let left = xs.iter().copied().fold(f32::MAX, f32::min);
    let right = xs.iter().copied().fold(f32::MIN, f32::max);
    let top = ys.iter().copied().fold(f32::MAX, f32::min);
    let bottom = ys.iter().copied().fold(f32::MIN, f32::max);
    Some((left, top, right, bottom))
}

/// `SkPath::isConvex` for one contour: every turn has the same direction,
/// it never doubles back and each axis changes direction at most 3 times.
fn is_convex(contours: &[Vec<Segment>]) -> bool {
    let [contour] = contours else { return false };
    let mut points: Vec<(f32, f32)> = Vec::new();
    for segment in contour {
        let (control, end) = match segment {
            Segment::Line(_, p1) => (None, *p1),
            Segment::Quad(_, c, p2) => (Some(*c), *p2),
        };
        for p in control.into_iter().chain(std::iter::once(end)) {
            if points.last() != Some(&p) {
                points.push(p);
            }
        }
    }
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    let n = points.len();
    if n <= 3 {
        return true;
    }
    let vectors: Vec<(f32, f32)> = (0..n)
        .map(|i| {
            (
                points[(i + 1) % n].0 - points[i].0,
                points[(i + 1) % n].1 - points[i].1,
            )
        })
        .collect();
    let mut sign = 0.0f32;
    let (mut x_changes, mut y_changes) = (0, 0);
    let (mut last_x, mut last_y) = (0.0f32, 0.0f32);
    for i in 0..n {
        let (a, b) = (vectors[(i + n - 1) % n], vectors[i]);
        let cross = a.0 * b.1 - a.1 * b.0;
        if cross == 0.0 {
            if a.0 * b.0 + a.1 * b.1 < 0.0 {
                return false;
            }
        } else if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
        if b.0 != 0.0 {
            if last_x != 0.0 && b.0.signum() != last_x {
                x_changes += 1;
            }
            last_x = b.0.signum();
        }
        if b.1 != 0.0 {
            if last_y != 0.0 && b.1.signum() != last_y {
                y_changes += 1;
            }
            last_y = b.1.signum();
        }
    }
    x_changes <= 3 && y_changes <= 3
}

#[derive(Clone, Copy)]
enum Segment {
    Line((f32, f32), (f32, f32)),
    Quad((f32, f32), (f32, f32), (f32, f32)),
}

fn contours(path: &[Verb]) -> (Vec<Vec<Segment>>, usize) {
    let mut all = Vec::new();
    let mut points = 0;
    let mut current: Vec<Segment> = Vec::new();
    let (mut start, mut last) = ((0.0f32, 0.0f32), (0.0f32, 0.0f32));
    let mut open = false;
    let close = |current: &mut Vec<Segment>,
                 all: &mut Vec<Vec<Segment>>,
                 last: (f32, f32),
                 start: (f32, f32)| {
        if last != start {
            current.push(Segment::Line(last, start));
        }
        if !current.is_empty() {
            all.push(std::mem::take(current));
        }
    };
    for verb in path {
        match *verb {
            Verb::Move(x, y) => {
                if open {
                    close(&mut current, &mut all, last, start);
                }
                start = (x, y);
                last = start;
                open = true;
                points += 1;
            }
            Verb::Line(x, y) => {
                current.push(Segment::Line(last, (x, y)));
                last = (x, y);
                points += 1;
            }
            Verb::Quad(cx, cy, x, y) => {
                current.push(Segment::Quad(last, (cx, cy), (x, y)));
                last = (x, y);
                points += 2;
            }
            Verb::Close => {
                if open {
                    close(&mut current, &mut all, last, start);
                    last = start;
                    open = false;
                }
            }
        }
    }
    if open {
        close(&mut current, &mut all, last, start);
    }
    (all, points)
}

/// `SkBlitter::blitFatAntiRect` of an A8 coverage blitter.
fn fat_rect(mask: &mut Coverage, rect: (f32, f32, f32, f32)) {
    let alpha = |a: f32| {
        let alpha = (a * 255.0) as u8;
        if alpha > 247 {
            0xFF
        } else if alpha < 8 {
            0
        } else {
            alpha
        }
    };
    let (left, top) = (rect.0.floor() as i32, rect.1.floor() as i32);
    let (right, bottom) = (rect.2.ceil() as i32, rect.3.ceil() as i32);
    let (width, height) = (right - left, bottom - top);
    if height == 0 {
        return;
    }
    let partial_l = (left + 1) as f32 - rect.0;
    let partial_r = rect.2 - (right - 1) as f32;
    let mut partial_t = (top + 1) as f32 - rect.1;
    let partial_b = rect.3 - (bottom - 1) as f32;
    if height == 1 {
        partial_t = rect.3 - rect.1;
    }
    let row = |mask: &mut Coverage, y: i32, partial: f32| {
        mask.set(left, y, alpha(partial_l * partial));
        for x in left + 1..right - 1 {
            mask.set(x, y, alpha(partial));
        }
        mask.set(right - 1, y, alpha(partial_r * partial));
    };
    row(mask, top, partial_t);
    if height > 2 {
        mask.blit_anti_rect(
            left,
            top + 1,
            width - 2,
            height - 2,
            alpha(partial_l),
            alpha(partial_r),
        );
    }
    if height > 1 {
        row(mask, bottom - 1, partial_b);
    }
}

/// Fills `path` (non-zero winding) like Skia fills a glyph path into its
/// A8 mask: the mask covers the path bounds rounded out.
#[must_use]
pub fn fill(path: &[Verb]) -> Option<Mask> {
    let (contours, point_count) = contours(path);
    let mut bounds = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for segment in contours.iter().flatten() {
        let points: &[(f32, f32)] = match segment {
            Segment::Line(p0, p1) => &[*p0, *p1],
            Segment::Quad(p0, c, p2) => &[*p0, *c, *p2],
        };
        for p in points {
            bounds = (
                bounds.0.min(p.0),
                bounds.1.min(p.1),
                bounds.2.max(p.0),
                bounds.3.max(p.1),
            );
        }
    }
    if !(bounds.0 < bounds.2 && bounds.1 < bounds.3)
        || !bounds.0.is_finite()
        || !bounds.3.is_finite()
    {
        return None;
    }
    let (left, top) = (bounds.0.floor() as i32, bounds.1.floor() as i32);
    let (right, bottom) = (bounds.2.ceil() as i32, bounds.3.ceil() as i32);
    let (width, height) = (right - left, bottom - top);
    if width <= 0 || height <= 0 || width > 4096 || height > 4096 {
        return None;
    }
    let mut mask = Coverage {
        left: 0,
        top: 0,
        width,
        height,
        data: vec![0; (width * height) as usize],
    };
    // Mask-local coordinates, like `GenerateImageFromPath`.
    let local = |p: (f32, f32)| (p.0 - left as f32, p.1 - top as f32);
    let contours: Vec<Vec<Segment>> = contours
        .iter()
        .map(|contour| {
            contour
                .iter()
                .map(|segment| match *segment {
                    Segment::Line(p0, p1) => Segment::Line(local(p0), local(p1)),
                    Segment::Quad(p0, c, p2) => Segment::Quad(local(p0), local(c), local(p2)),
                })
                .collect()
        })
        .collect();
    let small = width <= 32 && ((width + 3) & !3) * height <= 1024;
    if small
        && let Some(rect) = as_rect(&contours)
        && (rect.2.ceil() - rect.0.floor()) as i32 >= 3
    {
        fat_rect(&mut mask, rect);
        return Some(Mask {
            left,
            top,
            width: width as usize,
            height: height as usize,
            data: mask.data,
        });
    }
    let mut list = vec![
        Edge::sentinel(MIN_S32, MIN_S32),
        Edge::sentinel(MAX_S32, MAX_S32),
    ];
    let push = |list: &mut Vec<Edge>, edge: Edge| {
        // `combineVertical`: a vertical line continuing or cancelling the
        // previous one.
        if edge.kind == Kind::Line && edge.dx == 0 && list.len() > 2 {
            let last = list.last_mut().expect("edge");
            if last.kind == Kind::Line && last.dx == 0 && edge.x == last.x {
                let close = |a: Fixed, b: Fixed| (a - b).abs() < 0x100;
                if edge.winding == last.winding {
                    if edge.lower_y == last.upper_y {
                        last.upper_y = edge.upper_y;
                        last.y = last.upper_y;
                        return;
                    }
                    if close(edge.upper_y, last.lower_y) {
                        last.lower_y = edge.lower_y;
                        return;
                    }
                } else if close(edge.upper_y, last.upper_y) {
                    if close(edge.lower_y, last.lower_y) {
                        list.pop();
                        return;
                    }
                    if edge.lower_y < last.lower_y {
                        last.upper_y = edge.lower_y;
                        last.y = last.upper_y;
                        return;
                    }
                    last.upper_y = last.lower_y;
                    last.y = last.upper_y;
                    last.lower_y = edge.lower_y;
                    last.winding = edge.winding;
                    return;
                } else if close(edge.lower_y, last.lower_y) {
                    if edge.upper_y > last.upper_y {
                        last.lower_y = edge.upper_y;
                        return;
                    }
                    last.lower_y = last.upper_y;
                    last.upper_y = edge.upper_y;
                    last.y = last.upper_y;
                    last.winding = edge.winding;
                    return;
                }
            }
        }
        list.push(edge);
    };
    for segment in contours.iter().flatten() {
        match *segment {
            Segment::Line(p0, p1) => {
                if let Some(edge) = Edge::set_line(p0, p1) {
                    push(&mut list, edge);
                }
            }
            Segment::Quad(p0, c, p2) => {
                for quad in chop_quad_at_y_extrema([p0, c, p2]) {
                    if let Some(edge) = Edge::set_quadratic(quad) {
                        list.push(edge);
                    }
                }
            }
        }
    }
    let count = list.len() - 2;
    if count == 0 {
        return Some(Mask {
            left,
            top,
            width: width as usize,
            height: height as usize,
            data: mask.data,
        });
    }
    let mut order: Vec<usize> = (2..list.len()).collect();
    order.sort_unstable_by(|&a, &b| {
        let (a, b) = (&list[a], &list[b]);
        a.upper_y
            .cmp(&b.upper_y)
            .then(a.x.cmp(&b.x))
            .then(a.dx.cmp(&b.dx))
    });
    let mut chain = vec![HEAD];
    chain.extend(order);
    chain.push(TAIL);
    for pair in chain.windows(2) {
        list[pair[0]].next = pair[1];
        list[pair[1]].prev = pair[0];
    }
    let mut edges = Edges { list };
    let (left_bound, right_bound) = (0, width << 16);
    if is_convex(&contours) && count >= 2 {
        walk_convex_edges(&mut edges, &mut mask, height, left_bound, right_bound);
    } else {
        let skip_intersect = point_count > (height as usize) * 2;
        walk_edges(
            &mut edges,
            &mut mask,
            0,
            height,
            left_bound,
            right_bound,
            skip_intersect,
        );
    }
    Some(Mask {
        left,
        top,
        width: width as usize,
        height: height as usize,
        data: mask.data,
    })
}

#[cfg(test)]
mod tests {
    use super::Verb::{Close, Line, Move, Quad};
    use super::{Verb, fill};

    type Case = (
        &'static str,
        &'static [Verb],
        (i32, i32, usize, usize),
        &'static str,
    );

    /// DejaVu Sans glyphs (FreeType light hinting) at quarter-pixel phases,
    /// with the A8 masks Skia (m144) fills them into.
    #[rustfmt::skip]
    const SKIA: &[Case] = &[
    ("o_16_regular", &[Move(7.90625, 12.21875), Quad(6.75, 12.21875, 6.078125, 13.09375), Quad(5.40625, 13.96875, 5.40625, 15.5), Quad(5.40625, 17.015625, 6.0625, 17.890625), Quad(6.734375, 18.765625, 7.90625, 18.765625), Quad(9.046875, 18.765625, 9.71875, 17.890625), Quad(10.390625, 17.015625, 10.390625, 15.5), Quad(10.390625, 13.984375, 9.71875, 13.109375), Quad(9.046875, 12.21875, 7.90625, 12.21875), Close, Move(7.90625, 11.0), Quad(9.78125, 11.0, 10.84375, 12.1875), Quad(11.921875, 13.375, 11.921875, 15.5), Quad(11.921875, 17.609375, 10.84375, 18.8125), Quad(9.78125, 20.0, 7.90625, 20.0), Quad(6.015625, 20.0, 4.953125, 18.8125), Quad(3.890625, 17.609375, 3.890625, 15.5), Quad(3.890625, 13.375, 4.953125, 12.1875), Quad(6.015625, 11.0, 7.90625, 11.0), Close], (3, 11, 9, 9), "000035c5f3eab41900002ffabc566ad7e50b00b2d100000014f47d00f88200000000b9c212ff6e00000000a3e101fa8800000000c0c700c7dc00000018fe930036fa974040c4e713000034c5f3eab51a00"),
    ("i_16_regular_rect", &[Move(4.828125, 8.0), Line(6.40625, 8.0), Line(6.40625, 20.0), Line(4.828125, 20.0), Line(4.828125, 8.0), Close], (4, 8, 3, 12), "2bff672bff672bff672bff672bff672bff672bff672bff672bff672bff672bff672bff67"),
    ("period_16_regular", &[Move(5.21875, 18.015625), Line(6.859375, 18.015625), Line(6.859375, 20.0), Line(5.21875, 20.0), Line(5.21875, 18.015625), Close], (5, 18, 2, 2), "c7dbc7db"),
    ("at_14_bold", &[Move(9.4375, 16.0625), Quad(9.4375, 16.9375, 9.796875, 17.453125), Quad(10.15625, 17.953125, 10.765625, 17.953125), Quad(11.375, 17.953125, 11.734375, 17.453125), Quad(12.09375, 16.9375, 12.09375, 16.0625), Quad(12.09375, 15.21875, 11.71875, 14.734375), Quad(11.359375, 14.234375, 10.75, 14.234375), Quad(10.15625, 14.234375, 9.796875, 14.734375), Quad(9.4375, 15.21875, 9.4375, 16.0625), Close, Move(12.234375, 18.203125), Quad(12.03125, 18.6875, 11.578125, 18.96875), Quad(11.125, 19.234375, 10.515625, 19.234375), Quad(9.328125, 19.234375, 8.59375, 18.375), Quad(7.859375, 17.5, 7.859375, 16.09375), Quad(7.859375, 14.6875, 8.59375, 13.828125), Quad(9.34375, 12.953125, 10.515625, 12.953125), Quad(11.125, 12.953125, 11.578125, 13.234375), Quad(12.03125, 13.5, 12.234375, 14.0), Line(12.234375, 12.953125), Line(13.65625, 12.953125), Line(13.65625, 17.953125), Quad(14.515625, 17.8125, 15.0, 17.140625), Quad(15.484375, 16.453125, 15.484375, 15.375), Quad(15.484375, 14.6875, 15.28125, 14.09375), Quad(15.078125, 13.484375, 14.6875, 12.984375), Quad(14.03125, 12.15625, 13.046875, 11.703125), Quad(12.078125, 11.25, 10.953125, 11.25), Quad(10.15625, 11.25, 9.421875, 11.453125), Quad(8.703125, 11.65625, 8.09375, 12.0625), Quad(7.078125, 12.734375, 6.515625, 13.796875), Quad(5.953125, 14.859375, 5.953125, 16.09375), Quad(5.953125, 17.109375, 6.3125, 18.0), Quad(6.6875, 18.890625, 7.375, 19.5625), Quad(8.0625, 20.234375, 8.9375, 20.578125), Quad(9.828125, 20.9375, 10.828125, 20.9375), Quad(11.6875, 20.9375, 12.546875, 20.625), Quad(13.421875, 20.3125, 14.03125, 19.78125), Line(14.75, 20.859375), Quad(13.90625, 21.53125, 12.890625, 21.890625), Quad(11.890625, 22.25, 10.84375, 22.25), Quad(9.578125, 22.25, 8.453125, 21.796875), Quad(7.34375, 21.34375, 6.46875, 20.5), Quad(5.59375, 19.640625, 5.125, 18.515625), Quad(4.671875, 17.390625, 4.671875, 16.09375), Quad(4.671875, 14.828125, 5.140625, 13.703125), Quad(5.609375, 12.578125, 6.46875, 11.71875), Quad(7.328125, 10.875, 8.46875, 10.40625), Quad(9.609375, 9.9375, 10.84375, 9.9375), Quad(12.375, 9.9375, 13.625, 10.53125), Quad(14.875, 11.125, 15.71875, 12.21875), Quad(16.234375, 12.890625, 16.5, 13.6875), Quad(16.765625, 14.46875, 16.765625, 15.328125), Quad(16.765625, 17.171875, 15.640625, 18.203125), Quad(14.53125, 19.234375, 12.5, 19.234375), Line(12.234375, 19.234375), Line(12.234375, 18.203125), Close], (4, 9, 13, 14), "00000000000000000000000000000000137ab8efe4b67e0b0000000044f0d78151649ceede1e000023f481000000000012c7d60600b4b20029b9f3b9d0a810ec6014ff3400d0ec63a8ffa80099a844ff0411ff800004f7a80089b648fc0111ff7e0002f7a800ae8f23ff2a00d9d30752ffac5ef72900d79d0053fffff1dfffdd4a000047ff72002d40152d25090000000078ffc256150346d67c000000000045bbf8ffffff860500000000000000032f401500000000"),
    ("l_14_bold", &[Move(4.671875, 9.0), Line(7.125, 9.0), Line(7.125, 20.0), Line(4.671875, 20.0), Line(4.671875, 9.0), Close], (4, 9, 4, 11), "53ffff1f53ffff1f53ffff1f53ffff1f53ffff1f53ffff1f53ffff1f53ffff1f53ffff1f53ffff1f53ffff1f"),
    ("g_12_47_regular", &[Move(8.90625, 16.5), Quad(8.90625, 15.28125, 8.40625, 14.609375), Quad(7.90625, 13.9375, 7.0, 13.9375), Quad(6.09375, 13.9375, 5.59375, 14.609375), Quad(5.09375, 15.28125, 5.09375, 16.5), Quad(5.09375, 17.71875, 5.59375, 18.390625), Quad(6.09375, 19.0625, 7.0, 19.0625), Quad(7.90625, 19.0625, 8.40625, 18.390625), Quad(8.90625, 17.71875, 8.90625, 16.5), Close, Move(10.03125, 19.359375), Quad(10.03125, 21.1875, 9.25, 22.09375), Quad(8.484375, 23.0, 6.890625, 23.0), Quad(6.296875, 23.0, 5.765625, 22.90625), Quad(5.25, 22.828125, 4.765625, 22.640625), Line(4.765625, 21.421875), Quad(5.25, 21.765625, 5.734375, 21.921875), Quad(6.21875, 22.078125, 6.71875, 22.078125), Quad(7.8125, 22.078125, 8.359375, 21.421875), Quad(8.90625, 20.765625, 8.90625, 19.4375), Line(8.90625, 18.796875), Quad(8.5625, 19.40625, 8.015625, 19.703125), Quad(7.484375, 20.0, 6.71875, 20.0), Quad(5.46875, 20.0, 4.703125, 19.046875), Quad(3.9375, 18.078125, 3.9375, 16.5), Quad(3.9375, 14.921875, 4.703125, 13.96875), Quad(5.46875, 13.0, 6.71875, 13.0), Quad(7.484375, 13.0, 8.015625, 13.296875), Quad(8.5625, 13.59375, 8.90625, 14.203125), Line(8.90625, 13.0), Line(10.03125, 13.0), Line(10.03125, 19.359375), Close], (3, 13, 8, 10), "0009a0f1da5eff080094b42727adff0800f034000034ff0808ff1f00001fff0800f340000040ff080094af0303a9ff080009a0f3f060fa02000000000037d900001b230008b47a000030fcffe48d0600"),
    ("dollar_12_47_regular", &[Move(7.96875, 22.0), Line(7.359375, 22.0), Line(7.34375, 20.0), Quad(6.703125, 20.0, 6.0625, 19.875), Quad(5.4375, 19.734375, 4.78125, 19.453125), Line(4.78125, 18.359375), Quad(5.40625, 18.75, 6.03125, 18.953125), Quad(6.671875, 19.140625, 7.359375, 19.140625), Line(7.359375, 16.390625), Quad(6.015625, 16.171875, 5.390625, 15.65625), Quad(4.78125, 15.125, 4.78125, 14.234375), Quad(4.78125, 13.234375, 5.4375, 12.671875), Quad(6.109375, 12.09375, 7.359375, 12.09375), Line(7.359375, 10.0), Line(7.96875, 10.0), Line(7.96875, 12.09375), Quad(8.53125, 12.09375, 9.0625, 12.1875), Quad(9.59375, 12.265625, 10.09375, 12.40625), Line(10.09375, 13.453125), Quad(9.59375, 13.1875, 9.046875, 13.046875), Quad(8.515625, 12.90625, 7.96875, 12.90625), Line(7.96875, 15.46875), Quad(9.34375, 15.6875, 9.984375, 16.25), Quad(10.640625, 16.796875, 10.640625, 17.765625), Quad(10.640625, 18.796875, 9.953125, 19.40625), Quad(9.265625, 20.0, 7.96875, 20.0), Line(7.96875, 22.0), Close, Move(7.359375, 15.359375), Line(7.359375, 12.890625), Quad(6.640625, 12.96875, 6.265625, 13.296875), Quad(5.90625, 13.609375, 5.90625, 14.140625), Quad(5.90625, 14.671875, 6.25, 14.953125), Quad(6.59375, 15.234375, 7.359375, 15.359375), Close, Move(7.96875, 16.515625), Line(7.96875, 19.125), Quad(8.734375, 19.03125, 9.125, 18.703125), Quad(9.515625, 18.375, 9.515625, 17.828125), Quad(9.515625, 17.28125, 9.140625, 16.984375), Quad(8.765625, 16.671875, 7.96875, 16.515625), Close], (4, 10, 7, 12), "0000009c0000000000009c0000000041cbfde1a80c0eed44a100350c28f3079c00000000bcdee564170000024dccb5f0290000009c0092902864009c07c07d1ec8d6e7efc30f0000009f0000000000009d000000"),

    ];

    #[test]
    fn glyph_masks_match_skia() {
        for (name, path, (left, top, width, height), expected) in SKIA {
            let mask = fill(path).expect(name);
            assert_eq!(
                (mask.left, mask.top, mask.width, mask.height),
                (*left, *top, *width, *height),
                "{name}"
            );
            let expected: Vec<u8> = (0..expected.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(&expected[at..at + 2], 16).expect("hex"))
                .collect();
            assert_eq!(mask.data, expected, "{name}");
        }
    }

    #[test]
    fn empty_and_degenerate_paths_fill_nothing() {
        assert!(fill(&[]).is_none());
        assert!(fill(&[Move(1.0, 1.0), Line(5.0, 1.0), Close]).is_none());
    }

    #[test]
    fn a_square_is_covered_exactly() {
        let mask = fill(&[
            Move(1.0, 1.0),
            Line(4.0, 1.0),
            Line(4.0, 3.0),
            Line(1.0, 3.0),
            Close,
        ])
        .expect("mask");
        assert_eq!((mask.left, mask.top, mask.width, mask.height), (1, 1, 3, 2));
        assert!(mask.data.iter().all(|&value| value == 0xFF));
    }
}
