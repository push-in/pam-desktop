# Text rendering (GTK)

How the Linux backend turns a `Text` into pixels, and the optional glyph
atlas that rasterizes it like Chromium.

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
