# Text rendering (GTK)

How the Linux backend turns a `Text` into pixels, and the glyph atlas that
rasterizes it like Chromium.

## Layout and positions

Every text view owns one Pango layout (`crates/pam-desktop-gtk/src/text.rs`).
Runs carry their font, colour, background, underline, strikethrough and
letter-spacing as Pango attributes; inline boxes are characters shaped as
empty glyphs; ellipsis is Pango's. The view places the layout from the
engine's exact geometry (`TextGeometry`: wrap width, exact left/top offsets
and height, baselines snapped like Blink) in `TextState::place`
(`view.rs`). Glyph x positions are sub-pixel (Pango units, 1/1024 px).

On Linux the font face is sized like Chromium's: Skia sets the FreeType
size in 26.6 fixed point, truncated, so a 12.48px (0.78rem) run is shaped
and drawn from a 12.46875px face (`text::face_size`), about 0.1% narrower
than the exact size; a long line otherwise drifts a pixel from Chromium's.
Font metrics (ascent, descent, `line-height: normal`, the average character
width of fields) keep the exact size, as Skia's do.

Drawing never changes those positions: both paths below draw the same
layout at the same origin.

## How Chromium rasterizes glyphs on Linux

Measured glyph by glyph (probe pages in Electron 37 vs the host, same font
file, size, x phase and colour; FreeType 2.13.2 outlines filled with Skia
m144 through skia-python as the reference):

- **Hinting is the same.** Chromium reads the same fontconfig (here
  `hintslight`, grayscale, no LCD filter) and loads the outline with
  FreeType light hinting, like cairo. Forcing `hintnone` or `hintfull`
  changes Electron's pixels a lot; matching it in both leaves the same
  residual, so hinting is not the difference.
- **The rasterizer is not FreeType's.** Chromium fills the hinted outline
  as a path with Skia's analytic anti-aliasing (`SkScan_AAAPath`): edge
  ends and curve subdivision points snapped to 1/4 pixel vertically,
  coverage per trapezoid in 8-bit fixed point, narrow rectangles through
  the "fat rect" blit. FreeType (cairo, GSK) computes exact area coverage.
  A horizontal stroke edge at y = .2 is .2 in FreeType and .25 in Skia; a
  bowl bottom differs by up to 35 levels. This is the main difference:
  the outline filled by Skia matches Electron within 0.03/255, FreeType's
  coverage with Skia's gamma within 0.33/255.
- **Sub-pixel positions.** x is bucketed to quarter pixels
  (`floor(x + 1/8)`, phase `floor(4(x + 1/8)) mod 4`) and the outline moved
  by the phase; y is rounded to the pixel.
- **Mask gamma.** Each mask goes through Skia's correcting table: gamma
  1.2 and contrast 0.2 are stored as 76/64 = 1.1875 and 51/255, and the
  table is picked by `SkComputeLuminance` of the colour's channels reduced
  to 3 bits. With 1.2 instead of 1.1875 half of the edge pixels are one
  level off.
- **Blending.** Electron's tiles are rasterized on the CPU here: Skia blends
  `(fg * (a + 1) >> 8) + (bg * (256 - a) >> 8)`. GSK blends in float, so
  coloured text can be one level off on edge pixels (black text is exact);
  emulating Skia's floors would need the destination.

Fontconfig settings, LCD filters, embolden and bitmap strikes play no part
for the fonts used (DejaVu, Liberation: outline TrueType fonts).

## Default: Chromium glyph atlas

`crates/pam-desktop-gtk/src/glyphs.rs`, with the rasterizer in
`crates/pam-desktop-gtk/src/aaa.rs` (Skia's `SkScan_AAAPath`,
`SkAnalyticEdge` and the edge builder ported step by step; its tests
compare masks with Skia's, byte for byte):

- A Pango renderer subclass walks the layout, so backgrounds (selection
  included), underline, strikethrough, letter-spacing, ellipsis and
  per-run colours and alphas come from Pango exactly as with GSK's renderer.
- Each glyph's hinted outline comes from cairo (`cairo_glyph_path` with the
  Pango font's own scaled font: same size, hinting and options); cairo's
  cubic elevation of TrueType curves is undone back to the 26.6 quadratic
  control point. The outline is moved by the quarter-pixel x phase, filled
  by `aaa::fill` and corrected with Skia's table for the text colour.
  Outlines with real cubics (CFF fonts) are rasterized by cairo instead.
- The mask is stored premultiplied in the text colour.
- Glyphs live in one atlas per process, shared by every view and window.
  Key: Pango font (face, size, options), glyph id, scale factor, x
  quarter-pixel phase and the RGBA8 text colour. The glyphs rasterized by
  one paragraph draw are packed into one page.
- Drawing places cached glyphs on whole device pixels, never resampled:
  GPU renderers get one texture node per glyph (each glyph texture is a view
  of its page; GSK packs small textures into its own GPU atlas), the Cairo
  renderer gets one cairo node per run that copies the glyphs out of the page
  images. There is no texture per paragraph.
- Colour fonts (COLR, CBDT/sbix, SVG: checked once per font with
  HarfBuzz), glyphs missing from the font (hex boxes) and glyphs larger than
  256 device pixels make their run a GSK text node.
- Pages are least recently used first above 4 MiB, and the whole atlas
  (pages, fonts) is released when no window of the process is mapped. Nothing
  runs while idle: glyphs are rasterized only when a paragraph draws one
  that is not cached.
- Views that are scaled or rotated (not a plain translation from the
  window) keep GSK glyphs.

`PAM_DESKTOP_TEXT_GAMMA=0` keeps GSK glyphs (below).

### Cost

Real point-of-sale application (`perf-visual.php`), container with 2 CPUs,
Xvfb and GTK's Cairo renderer, debug host, 8 interleaved runs per mode, PSS
of host + PHP worker (means):

| metric | GSK glyphs | atlas | change |
|---|---|---|---|
| PSS idle (KiB) | 75 373 | 75 038 | -0.4% |
| PSS after the scroll (KiB) | 80 728 | 79 851 | -1.1% |
| CPU idle (% of a core) | 0.19 | 0.19 | 0 |
| CPU during the scroll (s) | 0.35 | 0.34 | -1.8% |
| scroll updates per second | 4.29 | 4.19 | -2.3% (one update in 11) |

On the pixel sweep (GPU, `ngl`, 39 routes) the mean content difference to
Electron drops on every in-scope route, e.g. /modules 0.345 to 0.099,
/pos 0.403 to 0.281, /sales 0.200 to 0.106 (/255).

## GSK glyphs (`PAM_DESKTOP_TEXT_GAMMA=0`)

The layout is appended with `gtk_snapshot_append_layout`: GSK text nodes,
rasterized and cached by GSK's own glyph cache (GL/Vulkan) or by cairo
(Cairo renderer), with FreeType's exact coverage blended raw.
