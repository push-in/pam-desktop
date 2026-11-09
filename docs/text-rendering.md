# Text rendering

How the Linux (GTK) and macOS (AppKit) backends turn a `Text` into pixels,
and how each one rasterizes glyphs like Chromium (on Linux by default, on
macOS opt-in). The GTK sections come
first; [macOS](#macos-appkit) is at the end.

## Layout and positions (GTK)

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

## macOS (AppKit)

### Layout and positions

TextKit lays every paragraph out (`crates/pam-desktop-appkit/src/text.rs`),
one shared layout manager for measuring, drawing and hit testing.

### Default: Core Graphics glyphs

By default TextKit draws the glyphs with Core Graphics at their exact
advances, with subpixel positioning and Core Graphics' own quantization;
the line origin moves an eighth of a pixel so that Core Graphics'
truncation lands where Skia's rounding does on small text.

### Opt-in: glyphs like Chromium (`PAM_DESKTOP_TEXT_GAMMA=1`)

Chromium does not let Core Graphics draw text into the page. Skia's
`SkScalerContext_Mac` draws each glyph once into an offscreen device-RGB
bitmap (black on white, Core Graphics' subpixel quantization off, subpixel
positioning on) at one of four horizontal quarter-pixel phases, keeps its
coverage as an 8-bit mask and blends the mask in the text colour at whole
device pixels. Drawn straight into a layer, Core Graphics places glyphs on
other positions (it keeps fewer than four phases above 11 px) and
antialiases those phases differently, so almost every glyph edge of
Chromium's text came out different.

`GlyphPlacer` (the layout manager subclass) intercepts TextKit's glyph runs
and `crates/pam-desktop-appkit/src/glyphs.rs` reproduces that pipeline:

- As on Linux, the font that shapes and draws `size` px text is sized to
  the CSS size truncated to 1/64 px (`face_size`): Chromium on macOS lays
  fractional sizes out the same way (0.78rem, 12.48px, is 12.46875px text).
  Core Graphics' own quantization (the default) loses more than it gains
  from this, so it only applies with the masks.
- Each glyph's device x is rounded to the nearest quarter pixel
  (`floor(4(x + 1/8))`) and its baseline to the nearest pixel, as Skia's
  subpixel rounding.
- The mask is rasterized with Skia's flags and box (the glyph's bounds,
  grown by the phase, rounded out and outset by one pixel). With
  `-webkit-font-smoothing: antialiased` (Chromium: no hinting) the coverage
  is used as is. With `auto` the glyph is drawn with Core Graphics smoothing,
  its 2.0 coverage gamma undone (`sk_pow2_table`) and Skia's sRGB mask gamma
  table (contrast 0, `SK_GAMMA_SRGB` as Chromium builds Skia for macOS)
  applied for the text luminance (8 levels). Whether smoothing changes
  anything on this system is detected once, as Skia does.
- Masks are kept as 8-bit coverage. A glyph run (one font and colour)
  blends its masks into one buffer (overlapping edges cover `a + b - ab`, as
  two blends in one colour would) and draws it as a single image mask in the
  run's colour, on whole device pixels, without resampling. The buffer is
  freed after the draw.
- Masks live in one cache per process shared by every view and window
  (key: font, glyph, phase, smoothing, luminance level), least recently
  drawn first above 512 KiB, and the whole cache is released when no window of
  the process is visible (occlusion: hidden, minimized, covered). Nothing
  runs while nothing draws.
- Colour glyphs (emoji), glyphs larger than 256 device pixels, rotated or
  skewed contexts and text with a shadow (Core Graphics would cast it per
  glyph) keep Core Graphics drawing, still at Skia's quarter pixels.
  Printing keeps exact positions.


Measured on the Mac VM against Electron 37 on the same machine (probe of 51
lines, 11-18px, regular and semibold, light and dark): mean difference per
line 1.46 to 0.26 /255; 11-16px integer sizes are within 0.05-0.07 (equal
masks). What is left is fractional sizes: Chromium's glyph positions there
(HarfBuzz advances and kerning) differ from TextKit's by up to ~0.4 px over a
700 px line, so a few glyphs land on the neighbouring quarter.

On the PushinShop point-of-sale application (39 routes, content x >= 216)
the masks lower the difference on 36 routes, by 0.03-0.32 /255 (modules
0.80 to 0.48, products 0.71 to 0.46, caderneta-saldos 0.66 to 0.46); reports
+0.02 (chart labels), the two login routes are unchanged.

#### Why it is not the default

Measured on the Mac VM (release hosts, `perf-visual.php`, the module
screen, 10 interleaved runs per mode; an 80-step wheel scroll through
`PAM_DESKTOP_CAPTURE_INPUT`), v2.1.1 drawing vs masks (means):

| metric | Core Graphics | masks | change |
|---|---|---|---|
| host footprint idle | 16.41 MB | 16.68 MB | +1.6% |
| host footprint after the scroll | 19.03 MB | 20.43 MB | +7.3% |
| host RSS idle | 47.1 MB | 47.4 MB | +0.7% |
| host RSS after the scroll | 49.8 MB | 51.2 MB | +2.8% |
| host CPU idle (20 s) | 0.006 s | 0.005 s | 0 |
| host CPU during the scroll | 0.253 s | 0.280 s | +10.7% |

The mask cache itself holds about 80 KiB on that screen (655 masks); the
rest of the footprint is Core Graphics' own work for one image mask per
run. Drawing each glyph from shared 256 x 256 atlas pages instead (one
image per page, clipped per glyph) was worse: footprint +9% idle, scroll
CPU +30%.

It stays opt-in until it costs at most 3% on every metric.
