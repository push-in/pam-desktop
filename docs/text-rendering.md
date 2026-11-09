# Text rendering

How the Linux (GTK) and macOS (AppKit) backends turn a `Text` into pixels,
and how each one rasterizes glyphs like Chromium. The GTK sections come
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

## Default: GSK glyphs

By default the layout is appended with `gtk_snapshot_append_layout`: GSK
text nodes, rasterized and cached by GSK's own glyph cache (GL/Vulkan) or by
cairo (Cairo renderer). GSK blends the raw glyph coverage, so text looks a
little lighter than Chromium's, mostly light text on dark backgrounds.

## Opt-in: Chromium glyph atlas (`PAM_DESKTOP_TEXT_GAMMA=1`)

Chromium on Linux draws text with Skia built with `SK_GAMMA_APPLY_TO_A8`,
gamma 1.2 and contrast 0.2: each glyph mask goes through a correcting table
chosen by the luminance of the text colour (8 levels). Light text on dark
gains weight, dark text on light loses a little.

With `PAM_DESKTOP_TEXT_GAMMA=1` (`crates/pam-desktop-gtk/src/glyphs.rs`):

- A Pango renderer subclass walks the layout, so backgrounds (selection
  included), underline, strikethrough, letter-spacing, ellipsis and
  per-run colours and alphas come from Pango exactly as with GSK's renderer.
- Each glyph is rasterized once with cairo, using the Pango font's own
  scaled font (same hinting and antialiasing), at the quarter-pixel phase
  cairo's image compositor uses (`floor(4(x + 1/8)) - 4 floor(x + 1/8)`).
  Each line's baseline is rounded to the device pixel grid, as Skia does.
- The coverage goes through Skia's correcting table for the text colour and
  is stored premultiplied in that colour.
- Glyphs live in one atlas per process, shared by every view and window.
  Key: Pango font (face, size, options), glyph id, scale factor, x/y
  quarter-pixel phase and the RGBA8 text colour (which selects the luminance
  table). The glyphs rasterized by one paragraph draw are packed into one
  page.
- Drawing places cached glyphs on whole device pixels, never resampled:
  GPU renderers get one texture node per glyph (each glyph texture is a view
  of its page; GSK packs small textures into its own GPU atlas), the Cairo
  renderer gets one cairo node per run that copies the glyphs out of the page
  images. There is no texture per paragraph.
- Colour glyphs (emoji), glyphs missing from the font (hex boxes) and glyphs
  larger than 256 device pixels make their run a GSK text node.
- Pages are least recently used first above 4 MiB, and the whole atlas
  (pages, fonts) is released when no window of the process is mapped.
- Views that are scaled or rotated (not a plain translation from the
  window) keep GSK glyphs.

`PAM_DESKTOP_TEXT_GAMMA=0` forces GSK glyphs.

### Why it is not the default

Measured on a real point-of-sale application
(container, 2 CPUs, Xvfb and GTK's Cairo renderer, `perf-visual.php`, debug
host, 8 interleaved runs per mode; PSS of host + PHP worker), atlas off vs on
(means):

| metric | off | on | change |
|---|---|---|---|
| PSS idle (KiB) | 64 649 | 74 041 | +14.5% |
| PSS after the scroll (KiB) | 72 600 | 76 208 | +5.0% |
| CPU idle (% of a core) | 0.13 | 0.15 | at tick resolution |
| CPU during the scroll (s) | 0.345 | 0.328 | -4.9% |
| scroll updates per second | 2.15 | 2.13 | -1.2% |

The PSS of these runs is bimodal (two clusters about 10 MB apart in either
mode); within a cluster the atlas adds about 2 MB, mostly cairo's recording
of one paint per glyph in the replayed runs (a texture node per glyph costs
far more CPU on the Cairo renderer). On the pixel sweep (GPU, `ngl`) the
atlas lowers the mean content difference on 29 of 37 routes, by 0.01-0.04
/255 (mean 1.009 to 0.997). It stays opt-in until it costs at most 3% on
every metric.

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
  drawn first above 1 MiB, and the whole cache is released when no window of
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

Measured on the Mac VM (debug host, `perf-visual.php`, the module screen,
4 interleaved runs per mode; an 80-step wheel scroll through
`PAM_DESKTOP_CAPTURE_INPUT`), v2.1.1 drawing vs masks (means):

| metric | Core Graphics | masks | change |
|---|---|---|---|
| host footprint idle | 16.71 MB | 16.93 MB | +1.3% |
| host footprint after the scroll | 19.50 MB | 20.66 MB | +5.9% |
| host RSS after the scroll | 53.5 MB | 54.7 MB | +2.3% |
| host CPU idle (20 s) | 0.007 s | 0.007 s | 0 |
| host CPU during the scroll | 0.32 s | 0.47 s | +45% (debug build: the blending loops are unoptimized) |

It stays opt-in until it costs at most 3% on every metric.
