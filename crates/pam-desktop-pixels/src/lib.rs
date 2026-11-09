//! Chromium's (Skia's) 8-bit compositing of a `backdrop-filter: blur()`
//! box, its translucent fill and the box shadows painted over it, on the
//! CPU. Kept apart from `pam-desktop-ui` so development builds compile it
//! optimised (it touches every pixel of a window).
//!
//! Measured on Electron 37 (Linux and macOS, identical pixels): the blur is
//! Skia's three-box approximation of a Gaussian (box size
//! `d = ⌊σ·3·√(2π)/4 + ½⌋`, three boxes of `d`, or `d`, `d`, `d + 1` when
//! `d` is even), one axis at a time (x, then y), each rounded to 8 bits,
//! with transparent edges; the blurred result is drawn over the unblurred
//! backdrop. A box shadow is the antialiased (spread, offset) rounded
//! rectangle blurred the same way with σ = blur / 2, its alpha
//! `(α·(mask + 1)) >> 8`, not painted under its box. Every blend is
//! `source premultiplied + ⌊dst·(256 − α)/256⌋`.

/// A box painted right over the backdrop (a dialog over its mask), in
/// device pixels relative to the image: its outer shadow, painted in the
/// backdrop's layer, and its translucent background, its own layer over it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintedBox {
    /// Left, top, right, bottom.
    pub rect: [f32; 4],
    /// Corner radii: top left, top right, bottom right, bottom left.
    pub radii: [f32; 4],
    pub shadow: Option<BoxShadow>,
    /// Translucent background, in the pixels' channel order, alpha last.
    pub fill: Option<[u8; 4]>,
    /// What the fill is cut to (left, top, right, bottom).
    pub clip: Option<[f32; 4]>,
}

/// An outer `box-shadow`, in device pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxShadow {
    pub offset: [f32; 2],
    /// CSS blur radius (twice the deviation).
    pub blur: f32,
    pub spread: f32,
    /// In the pixels' channel order, alpha last.
    pub color: [u8; 4],
}

/// A premultiplied channel `source` with `alpha` over the channel `under`.
pub fn over_channel(source: u32, alpha: u32, under: u8) -> u8 {
    (source + ((u32::from(under) * (256 - alpha)) >> 8)).min(255) as u8
}

/// Box sizes of Skia's blur for `sigma` (pixels); empty below one pixel.
#[must_use]
pub fn box_sizes(sigma: f32) -> Vec<usize> {
    let d = (f64::from(sigma) * 3.0 * (2.0 * std::f64::consts::PI).sqrt() / 4.0 + 0.5).floor();
    if d.is_nan() || d < 2.0 {
        return Vec::new();
    }
    let d = d as usize;
    if d % 2 == 1 {
        vec![d; 3]
    } else {
        vec![d, d, d + 1]
    }
}

/// Blurs one line of `len` pixels of `N` bytes (`step` bytes apart from
/// `start`) into `out` (transparent beyond both ends): the boxes as
/// successive running sums, rounded once. `scratch` is reused.
fn blur_line<const N: usize>(
    src: &[u8],
    start: usize,
    step: usize,
    len: usize,
    boxes: &[usize],
    [line, next]: &mut [Vec<[u32; N]>; 2],
    out: &mut [[u8; N]],
) {
    let radius = (boxes.iter().sum::<usize>() - boxes.len()) / 2;
    let divisor: u32 = boxes.iter().map(|size| *size as u32).product();
    line.clear();
    line.resize(radius, [0; N]);
    line.extend((0..len).map(|i| {
        let at = start + i * step;
        std::array::from_fn(|lane| u32::from(src[at + lane]))
    }));
    line.resize(len + 2 * radius, [0; N]);
    // Each box ("valid" convolution) shortens the line by its size minus
    // one, the boxes together by 2·radius.
    for &size in boxes {
        next.clear();
        let mut sum = [0_u32; N];
        for value in &line[..size] {
            for lane in 0..N {
                sum[lane] += value[lane];
            }
        }
        next.push(sum);
        for i in size..line.len() {
            let (add, sub) = (line[i], line[i - size]);
            for lane in 0..N {
                sum[lane] = sum[lane] + add[lane] - sub[lane];
            }
            next.push(sum);
        }
        std::mem::swap(line, next);
    }
    for (pixel, sum) in out.iter_mut().zip(line.iter()) {
        *pixel = sum.map(|value| ((value + divisor / 2) / divisor) as u8);
    }
}

/// Separable blur of a `width` × `height` image of `N`-byte pixels
/// (`stride` bytes per row): x pass, then y pass, each rounded.
fn blur_image<const N: usize>(
    pixels: &[u8],
    width: usize,
    height: usize,
    stride: usize,
    boxes: &[usize],
) -> Vec<[u8; N]> {
    let mut scratch = [Vec::new(), Vec::new()];
    // The x pass, stored column by column so the y pass reads lines.
    let mut columns = vec![0_u8; width * height * N];
    let mut row = vec![[0_u8; N]; width];
    for y in 0..height {
        blur_line(pixels, y * stride, N, width, boxes, &mut scratch, &mut row);
        for (x, pixel) in row.iter().enumerate() {
            let at = (x * height + y) * N;
            columns[at..at + N].copy_from_slice(pixel);
        }
    }
    let mut blurred = vec![[0_u8; N]; width * height];
    let mut column = vec![[0_u8; N]; height];
    for x in 0..width {
        blur_line(
            &columns,
            x * height * N,
            N,
            height,
            boxes,
            &mut scratch,
            &mut column,
        );
        for (y, pixel) in column.iter().enumerate() {
            blurred[y * width + x] = *pixel;
        }
    }
    blurred
}

/// Coverage (0..=255) of the pixel at (`x`, `y`) by a rounded rectangle
/// (`left`, `top`, `right`, `bottom`, corner radii TL, TR, BR, BL).
fn rrect_coverage(x: f32, y: f32, rect: [f32; 4], radii: [f32; 4]) -> u8 {
    let [left, top, right, bottom] = rect;
    let span =
        |from: f32, to: f32, start: f32| (to.min(start + 1.0) - from.max(start)).clamp(0.0, 1.0);
    let straight = span(left, right, x) * span(top, bottom, y);
    if straight <= 0.0 {
        return 0;
    }
    let corner = |px: f32, py: f32| -> Option<(f32, f32, f32)> {
        let r = if px < left + radii[0].max(radii[3]) && py < top + radii[0] {
            Some((left + radii[0], top + radii[0], radii[0]))
        } else if px > right - radii[1] && py < top + radii[1] {
            Some((right - radii[1], top + radii[1], radii[1]))
        } else if px > right - radii[2] && py > bottom - radii[2] {
            Some((right - radii[2], bottom - radii[2], radii[2]))
        } else if px < left + radii[3] && py > bottom - radii[3] {
            Some((left + radii[3], bottom - radii[3], radii[3]))
        } else {
            None
        };
        r.filter(|(_, _, r)| *r > 0.0)
    };
    let near_corner = [(x, y), (x + 1.0, y), (x, y + 1.0), (x + 1.0, y + 1.0)]
        .iter()
        .any(|(px, py)| corner(*px, *py).is_some());
    if !near_corner {
        return (straight * 255.0 + 0.5) as u8;
    }
    // Corners: 4×4 samples.
    let mut inside = 0;
    for sy in 0..4 {
        for sx in 0..4 {
            let (px, py) = (x + (sx as f32 + 0.5) / 4.0, y + (sy as f32 + 0.5) / 4.0);
            if px < left || px > right || py < top || py > bottom {
                continue;
            }
            match corner(px, py) {
                Some((cx, cy, r)) if (px - cx).powi(2) + (py - cy).powi(2) > r * r => {}
                _ => inside += 1,
            }
        }
    }
    (inside as f32 / 16.0 * 255.0 + 0.5) as u8
}

/// `filter: blur(sigma)` of a box painted in one colour, for a renderer
/// without GPU filters: the antialiased rounded rectangle `rect` (left,
/// top, right, bottom; corner radii TL, TR, BR, BL; device pixels) in the
/// straight RGBA `color`, on a transparent `width` × `height` image,
/// blurred like Skia (three boxes, transparent beyond the edges).
/// Premultiplied RGBA, `width * 4` bytes per row.
#[must_use]
pub fn blurred_box(
    width: usize,
    height: usize,
    rect: [f32; 4],
    radii: [f32; 4],
    color: [u8; 4],
    sigma: f32,
) -> Vec<u8> {
    let mut pixels = vec![0_u8; width * height * 4];
    for y in 0..height {
        for x in 0..width {
            let coverage = u32::from(rrect_coverage(x as f32, y as f32, rect, radii));
            if coverage == 0 {
                continue;
            }
            let alpha = (u32::from(color[3]) * coverage + 127) / 255;
            let at = (y * width + x) * 4;
            for channel in 0..3 {
                pixels[at + channel] = ((u32::from(color[channel]) * alpha + 127) / 255) as u8;
            }
            pixels[at + 3] = alpha as u8;
        }
    }
    let kernel = box_sizes(sigma);
    if kernel.is_empty() || width == 0 || height == 0 {
        return pixels;
    }
    blur_image::<4>(&pixels, width, height, width * 4, &kernel)
        .into_iter()
        .flatten()
        .collect()
}

/// How far a Skia blur of `sigma` spreads past the blurred shape, in pixels.
#[must_use]
pub fn blur_reach(sigma: f32) -> usize {
    let kernel = box_sizes(sigma);
    kernel.iter().map(|size| size / 2 + 1).sum()
}

/// The 8-bit alpha (paint alpha included) of an outer `box-shadow` of the
/// box `rect` (device pixels, relative to the image) over a `width` ×
/// `height` image, like Skia draws it: the spread/offset rounded rectangle
/// as an antialiased A8 mask, blurred by the three boxes of σ = blur / 2,
/// times the colour's alpha as `(α·(mask + 1)) >> 8`.
fn shadow_alpha(width: usize, height: usize, owner: &PaintedBox, shadow: &BoxShadow) -> Vec<u8> {
    let spread = shadow.spread;
    let [left, top, right, bottom] = owner.rect;
    let rect = [
        left + shadow.offset[0] - spread,
        top + shadow.offset[1] - spread,
        right + shadow.offset[0] + spread,
        bottom + shadow.offset[1] + spread,
    ];
    let radii = owner
        .radii
        .map(|r| if r > 0.0 { (r + spread).max(0.0) } else { 0.0 });
    let boxes = box_sizes(shadow.blur / 2.0);
    let reach = boxes.iter().sum::<usize>() as f32 / 2.0 + 2.0;
    // The mask covers the whole blurred shadow, also beyond the image.
    let x0 = (rect[0] - reach).floor() as i64;
    let y0 = (rect[1] - reach).floor() as i64;
    let x1 = (rect[2] + reach).ceil() as i64;
    let y1 = (rect[3] + reach).ceil() as i64;
    let mut alpha = vec![0_u8; width * height];
    if x1 <= x0 || y1 <= y0 || x1 < 0 || y1 < 0 || x0 > width as i64 || y0 > height as i64 {
        return alpha;
    }
    let (mw, mh) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut mask = vec![0_u8; mw * mh];
    for my in 0..mh {
        for mx in 0..mw {
            mask[my * mw + mx] = rrect_coverage(
                (x0 + mx as i64) as f32,
                (y0 + my as i64) as f32,
                rect,
                radii,
            );
        }
    }
    let blurred: Vec<[u8; 1]> = if boxes.is_empty() {
        mask.iter().map(|value| [*value]).collect()
    } else {
        blur_image::<1>(&mask, mw, mh, mw, &boxes)
    };
    let paint = u32::from(shadow.color[3]);
    for y in 0..height {
        let my = y as i64 - y0;
        if my < 0 || my >= mh as i64 {
            continue;
        }
        for x in 0..width {
            let mx = x as i64 - x0;
            if mx < 0 || mx >= mw as i64 {
                continue;
            }
            let coverage = u32::from(blurred[my as usize * mw + mx as usize][0]);
            alpha[y * width + x] = ((paint * (coverage + 1)) >> 8) as u8;
        }
    }
    // Outer shadows are not painted under their box.
    let (inside, box_radii) = (owner.rect, owner.radii);
    let ys = (inside[1].floor().max(0.0) as usize)
        ..(inside[3].ceil().min(height as f32).max(0.0) as usize);
    let xs = (inside[0].floor().max(0.0) as usize)
        ..(inside[2].ceil().min(width as f32).max(0.0) as usize);
    for y in ys {
        for x in xs.clone() {
            let covered = u32::from(rrect_coverage(x as f32, y as f32, inside, box_radii));
            let at = y * width + x;
            alpha[at] = ((u32::from(alpha[at]) * (255 - covered) + 127) / 255) as u8;
        }
    }
    alpha
}

/// `backdrop-filter: blur(sigma)`, an optional translucent RGBA `fill`
/// and the `boxes` painted over it, applied in place to opaque 4-byte
/// pixels (`stride` bytes per row; the channel order of the colours is the
/// pixels') the way Chromium paints them: the fill and the boxes' shadows
/// make one layer (shadows over the fill), drawn over the blur, which is
/// drawn over the unblurred backdrop; then each box's translucent
/// background (its own layer). Everything in device pixels.
#[allow(clippy::too_many_arguments)]
pub fn composite(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    stride: usize,
    sigma: f32,
    fill: Option<[u8; 4]>,
    boxes: &[PaintedBox],
) {
    if width == 0 || height == 0 || pixels.len() < stride * (height - 1) + width * 4 {
        return;
    }
    let kernel = box_sizes(sigma);
    if !kernel.is_empty() {
        let blurred = blur_image::<4>(pixels, width, height, stride, &kernel);
        // The blur over the unblurred backdrop.
        for y in 0..height {
            for x in 0..width {
                let at = y * stride + x * 4;
                let pixel = blurred[y * width + x];
                let alpha = u32::from(pixel[3]);
                for channel in 0..3 {
                    pixels[at + channel] =
                        over_channel(u32::from(pixel[channel]), alpha, pixels[at + channel]);
                }
                pixels[at + 3] = 255;
            }
        }
    }
    let fill = fill.filter(|fill| fill[3] > 0);
    let shadows: Vec<(Vec<u8>, [u8; 4])> = boxes
        .iter()
        .rev()
        .filter_map(|owner| owner.shadow.map(|shadow| (owner, shadow)))
        .filter(|(_, shadow)| shadow.color[3] > 0)
        .map(|(owner, shadow)| (shadow_alpha(width, height, owner, &shadow), shadow.color))
        .collect();
    if fill.is_some() || !shadows.is_empty() {
        mask_layer(pixels, width, height, stride, fill, &shadows);
    }
    box_fills(pixels, width, height, stride, boxes);
}

/// The boxes' translucent backgrounds as one layer (the dialog's: later
/// boxes over earlier ones, premultiplied, in 8 bits), antialiased at
/// edges and corners, then that layer over the pixels.
fn box_fills(pixels: &mut [u8], width: usize, height: usize, stride: usize, boxes: &[PaintedBox]) {
    let filled: Vec<(&PaintedBox, [u8; 4], [usize; 4])> = boxes
        .iter()
        .filter_map(|owner| {
            let fill = owner.fill.filter(|fill| fill[3] > 0)?;
            let [mut left, mut top, mut right, mut bottom] = owner.rect;
            if let Some(clip) = owner.clip {
                left = left.max(clip[0]);
                top = top.max(clip[1]);
                right = right.min(clip[2]);
                bottom = bottom.min(clip[3]);
            }
            let x0 = left.floor().max(0.0) as usize;
            let y0 = top.floor().max(0.0) as usize;
            let x1 = (right.ceil().min(width as f32).max(0.0) as usize).max(x0);
            let y1 = (bottom.ceil().min(height as f32).max(0.0) as usize).max(y0);
            (x1 > x0 && y1 > y0).then_some((owner, fill, [x0, y0, x1, y1]))
        })
        .collect();
    let Some(first) = filled.first() else {
        return;
    };
    let mut area = first.2;
    for (_, _, [x0, y0, x1, y1]) in &filled {
        area = [
            area[0].min(*x0),
            area[1].min(*y0),
            area[2].max(*x1),
            area[3].max(*y1),
        ];
    }
    let (aw, ah) = (area[2] - area[0], area[3] - area[1]);
    // Premultiplied layer: R, G, B, A.
    let mut layer = vec![[0_u8; 4]; aw * ah];
    for (owner, fill, [x0, y0, x1, y1]) in &filled {
        let paint = u32::from(fill[3]);
        for y in *y0..*y1 {
            for x in *x0..*x1 {
                let mut coverage =
                    u32::from(rrect_coverage(x as f32, y as f32, owner.rect, owner.radii));
                if let Some(clip) = owner.clip {
                    let span = |from: f32, to: f32, start: f32| {
                        (to.min(start + 1.0) - from.max(start)).clamp(0.0, 1.0)
                    };
                    let inside =
                        span(clip[0], clip[2], x as f32) * span(clip[1], clip[3], y as f32);
                    coverage = (coverage as f32 * inside + 0.5) as u32;
                }
                if coverage == 0 {
                    continue;
                }
                let alpha = if coverage == 255 {
                    paint
                } else {
                    (paint * (coverage + 1)) >> 8
                };
                let cell = &mut layer[(y - area[1]) * aw + (x - area[0])];
                for channel in 0..3 {
                    let source = (u32::from(fill[channel]) * alpha + 127) / 255;
                    cell[channel] = over_channel(source, alpha, cell[channel]);
                }
                cell[3] = over_channel(alpha, alpha, cell[3]);
            }
        }
    }
    for y in area[1]..area[3] {
        for x in area[0]..area[2] {
            let cell = layer[(y - area[1]) * aw + (x - area[0])];
            if cell[3] == 0 {
                continue;
            }
            let at = y * stride + x * 4;
            for channel in 0..3 {
                pixels[at + channel] = over_channel(
                    u32::from(cell[channel]),
                    u32::from(cell[3]),
                    pixels[at + channel],
                );
            }
        }
    }
}

/// The backdrop's layer (its fill, then the shadows over it) over the
/// blurred backdrop.
fn mask_layer(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    stride: usize,
    fill: Option<[u8; 4]>,
    shadows: &[(Vec<u8>, [u8; 4])],
) {
    let (fill_rgb, fill_alpha) = fill.map_or(([0; 3], 0), |fill| {
        let alpha = u32::from(fill[3]);
        (
            [fill[0], fill[1], fill[2]].map(|channel| (u32::from(channel) * alpha + 127) / 255),
            alpha,
        )
    });
    for y in 0..height {
        for x in 0..width {
            let (mut rgb, mut alpha) = (fill_rgb, fill_alpha);
            for (field, color) in shadows {
                let shade = u32::from(field[y * width + x]);
                if shade == 0 {
                    continue;
                }
                let source = [color[0], color[1], color[2]]
                    .map(|channel| (u32::from(channel) * shade + 127) / 255);
                for channel in 0..3 {
                    rgb[channel] = source[channel] + ((rgb[channel] * (256 - shade)) >> 8);
                }
                alpha = shade + ((alpha * (256 - shade)) >> 8);
            }
            if alpha == 0 {
                continue;
            }
            let at = y * stride + x * 4;
            for channel in 0..3 {
                pixels[at + channel] =
                    over_channel(rgb[channel], alpha.min(255), pixels[at + channel]);
            }
            pixels[at + 3] = 255;
        }
    }
}
