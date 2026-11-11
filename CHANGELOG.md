# Changelog

## Unreleased

### Added

- **SSH.** A native `ssh` module (cargo feature `ssh`, on by default) and the
  package `pushinbr/pam-desktop-ssh`: sessions with password or key
  authentication and host key pinning, `exec` with bounded output, SFTP
  (list, stat, mkdir, rename, remove, read, write, realpath, uploads and
  downloads with progress), local port forwarding and interactive shells. It
  runs on russh with pure Rust crypto, on threads that exist only while a
  session is open. See [SSH](docs/ssh.md).
- **Terminal hub.** `pam_desktop_ui::modules::terminal::TerminalHub` routes a
  terminal's bytes between a producer (an SSH shell) and a native consumer
  without PHP.
- **Binary values in module payloads.** `Pam\Desktop\Modules\WireBytes` sends
  opaque bytes (wire tag 5), and results that carry bytes decode to strings.

## 2.2.1 - 2026-11-11

### Fixed

- The launcher downloads the 2.2.x host. 2.2.0 still pointed it at the 2.1.8
  host, so the new tools were missing; update with
  `pam composer update pushinbr/pam-desktop`.
- Task group and deferred deadlines are measured on the monotonic clock.

## 2.2.0 - 2026-11-11

New tools for desktop apps, all used by the four clones in `examples/`. Update
with `pam composer update pushinbr/pam-desktop`; the launcher downloads the
2.2.1 host. No app changes needed.

### Added

- **Transitions on state styles.** `transition` in a stylesheet now animates
  `:hover`, `:focus` and group hover changes in the host, without PHP, and
  `background-color`, `color`, `border-color` and `border-radius` animate
  besides opacity and transforms.
- **Context menus in templates.** `:contextMenu="menu($row)"` takes a list of
  `MenuItem`, and an item's `shortcut` shows beside its label.
- **DesktopUI:** `Popover` (anchored to any `nativeRef`, flips to fit, closes
  on a press outside or Escape), a rich `Tooltip` (title, description,
  shortcut), `SplitView` (draggable divider, keyboard, min/max, collapse),
  a virtualized `TreeView` (lazy branches, keyboard, context menus) and
  `CodeEditor` (highlighting, several cursors, selections, undo, VS Code keys,
  native text input so accents and input methods work).
- **Popovers and tooltips draw above everything** through a window-level
  portal (`Portal::WINDOW_HOST`): no `PortalHost` to place.
- **The window remembers its size and maximized state** between launches
  (`window.remember`, on by default).
- **The inspector:** F12 or Ctrl+Shift+I in `pam-desktop dev` outlines the
  element under the pointer and lists its kind, frame, effective properties,
  state styles and events; a click pins it.
- **Notification buttons** on Linux, macOS and Windows (`Notification::show`
  takes `label => handler` actions) and **`Window::badge($count)`** on the
  dock, launcher or taskbar icon.
- **SQLite migrations:** `Migrations::run()` versions a database on
  `PRAGMA user_version` and applies pending migrations in one transaction.
- **`Pam\Desktop\Markdown`:** Markdown as native rich text and boxes (lists,
  tasks, quotes, code blocks, tables, links, mentions, Slack's mrkdwn).
- **`Pam\Desktop\Code`:** syntax highlighting of 18 languages with VS Code's
  themes, and a Language Server Protocol client.
- **`Pam\Desktop\Auth`:** OAuth 2.0 and OpenID Connect sign-in with PKCE
  through the system browser and the app's URL scheme, tokens in the keychain
  and refreshed for you. `Linking::claim()` lets a library take its callbacks.
- **End-to-end scripts:** `PAM_DESKTOP_SCRIPT=steps.json` runs clicks, typing,
  keys and checks against the live window and exits with the result. The four
  clones have scripts, and CI runs them on Linux, macOS and Windows.
- **Components listen to their events with `on:name`** as well as `@name`.
- **`$event` is typed in handler expressions** (`on:press="pick($row, $event)"`
  gets a `PressEvent`; keys, layout, gestures and drops too).

### Changed

- **`pam-desktop check` reports more:** unknown tags (a package not booted),
  unknown events (`on:pres`), unknown props of `.pam` components, and unknown
  attributes of `Input`, `ScrollView`, `VirtualizedList`, `GestureDetector`,
  `Modal` and other tags.
- **A bound `null` leaves any property unset**, colours included, instead of
  failing.

### Fixed

- Positions of elements inside transformed content (a list stacked from the
  end) are right for menus, file drops and the inspector.
- Icon fonts bundled with the package load when it is installed through a
  Composer `path` repository linked from elsewhere.

## 2.1.8 - 2026-11-11

What the VS Code clone in `examples/vscode` needed: Linux shortcuts and shell
events that reach PHP. Update with `pam composer update pushinbr/pam-desktop`;
the launcher downloads the 2.1.8 host. No app changes needed.

### Fixed

- **Linux: shortcuts, menus and other shell events reach PHP.** The GTK host
  woke its inbox with `MainContext::invoke`, which runs the pump at once on the
  UI thread. A reply posted while the runtime was draining (a shortcut, a menu
  or tray item answering the shell listener) found it busy, was skipped and
  left the inbox stranded: `Shortcut::register('CmdOrCtrl+P', …)` matched but
  its handler never ran. The wake is now always queued.
- **`PAM_DESKTOP_DEBUG=1` logs key presses** with their modifiers and the
  focused node, and the shortcut a key matched.

## 2.1.7 - 2026-11-11

What the Trello clone in `examples/trello` needed: cards that open on click
and drag between lists. Update with `pam composer update pushinbr/pam-desktop`;
the launcher downloads the 2.1.7 host. No app changes needed.

### Fixed

- **A clickable card drags.** A `Pressable` inside a `DragSource` (a Trello
  card that opens on click) never started a drag: the host stopped looking
  for the drag source at the first pressable. A click stays a press; past
  4 points of travel the source drags. Text fields still select.

## 2.1.6 - 2026-11-11

What the Spotify clone in `examples/spotify` needed: a `Slider` control,
`?->` in templates and typed key and layout events. Update with
`pam composer update pushinbr/pam-desktop`; no app changes needed.

### Added

- **`Slider`** in DesktopUI: a controlled horizontal slider for volumes,
  playback positions and settings. Click or drag (past either end), arrow,
  Page and Home/End keys, a `committed` callback on release, steps, and the
  slider role with its range for screen readers.

- **`?->` in templates.** `{{ track()?->title ?? '' }}` and `$user?->name`
  end the chain with null on a null value, as in PHP, without evaluating the
  rest (its arguments included).

### Fixed

- **Key, layout and drop handlers receive typed events.** A template handler
  typed `KeyEvent` (`on:keyDown`), `LayoutEvent` (`on:layout`) or
  `DragPayload` (`on:drop`, `on:dragEnter`, …) got the wire string and failed
  with a TypeError.
- **DesktopUI and Charts install with `pushinbr/pam-desktop`.** The docs told
  to require `pushinbr/pam-desktop-ui` and `pushinbr/pam-desktop-charts`, which
  are not published: both ship in `pushinbr/pam-desktop`.

## 2.1.5 - 2026-11-11

Everything the Slack clone in `examples/slack` needed: Lucide icons in the
package, hover actions revealed from an ancestor's `:hover`, call chains in
templates, expression checks in `pam-desktop check` and an interaction
profiler. Linux opens new views without the one-second stall it had without
a session bus. Update with `pam composer update pushinbr/pam-desktop`; no
app changes needed.

### Added

- **Lucide icons ship with the package.** `<Icon name="hash" font="Lucide" />`
  draws any of Lucide's 2,370 icons with no font or glyph map copied into the
  project.
- **Calls chain in templates.** `{{ owner()->name }}`, `active()->isDirect()`,
  `entries()[0]` and `($a ?? $b)->title` read the result of a call or a group
  like any other value (a `.` after a call stays concatenation).
- **`pam-desktop check` validates template expressions (`PAM2405`).** A syntax
  error (`{{ $count + }}`) or a bare call to a missing or private component
  method (`p-if="missing()"`) is listed with its file, line and column instead
  of failing when the screen first renders.
- **Hover on an ancestor styles its descendants.** `.message:hover .actions
  { opacity: 1; }` reveals hover actions with no PHP round trip, on every
  host. Before, a `:hover` anywhere but the last compound silently matched
  nothing.
- **`p-for="($item, $i) in $items"`**, Vue's header, names the loop index
  (the same as `$itemIndex` or `p-index="i"`).
- **`PAM_DESKTOP_PROFILE=1`** times every interaction: the PHP handler and
  render, the host layout of the committed frame, the native view work and the
  frame painted after it, with a wall clock that lines worker and host up.

### Fixed

- **A Span keeps the spaces at its edges.** `<Span>between </Span><Span>@Ana</Span>`
  rendered "between@Ana": the edge trimming meant for a plain Text leaf also
  ran on each Span, the runs whose separators those spaces are.
- **A class styled only in a state is a known class.** A template using
  `class="closed"` whose sheet only has `.closed:hover { … }` failed with
  "Unknown Pam Desktop class closed".
- **Numeric state styles reach the host.** `opacity` and the other numbers of
  a `:hover`, `:focus` or `:pressed` rule were sent as strings and ignored.

- **Linux without a session bus renders new views at once.** Started without a
  D-Bus session bus (Xvfb, ssh, CI, a system service), GTK spent 5 to 13 ms
  per new widget failing to reach the accessibility bus: opening a panel of a
  hundred views froze the window for over a second. The host now sets
  `GTK_A11Y=none` there, where no screen reader could connect anyway.

## 2.1.4 - 2026-11-09

The same screen now looks the same on Linux, macOS and Windows: text widths,
line breaks and custom font faces match across the three hosts. Update the
Composer package (`pam composer update pushinbr/pam-desktop`); the launcher
downloads the verified 2.1.4 host. No breaking changes and no app changes
needed.

### Fixed

- **Windows lays text out on the same grid as Linux and macOS.** The Win32
  host rounded every measured text width up to a whole pixel, where GTK and
  AppKit round up to a Blink LayoutUnit (1/64 px). Shrink-wrapped labels
  (buttons, chips, tabs) were up to 1 px wider each, so rows of them drifted
  by several pixels on Windows only (a POS category row ended 6 px further
  right). Found by the cross-platform comparison of the PDV storefront screens.
- **Windows wraps text at the engine's exact width.** The Win32 host had no
  `set_text_geometry`: it wrapped each paragraph at its frame snapped to whole
  pixels, so a label laid out in exactly its measured width (106.25 px drawn
  in a 106 px frame) lost its last word to a second line. Lines now break at
  the content width the engine laid out, as on GTK and AppKit.
- **Inherited `@font-face` families resolve in package components.** A
  component of another stylesheet (a DesktopUI component such as `MediaTile`,
  or one placed with `<Slot :content>`) inherited the app's `@font-face`
  family by name, or the face resolved for its ancestor, and its stylesheet
  could not resolve it for the component's own weight. Fontconfig found the
  face by family name; DirectWrite and Core Text did not and drew another face
  (`MediaTile`'s "PRINCIPAL" badge 5 px wider on macOS, 3 px on Windows). The
  SDK now resolves such a family, or face, among the faces any stylesheet
  declared for it.
- **A bound font weight picks the right face on macOS and Windows.** When the
  weight or style comes from a binding (`:fontWeight="$style->fontWeight"`,
  as in DesktopUI's `Autocomplete` input) the SDK looked the face up before
  the value was known, so Core Text and DirectWrite drew the Regular face (or
  another font) where Linux drew the right one. The face is now chosen after
  the binding is evaluated, for every backend; a weight from a CSS variable
  (`font-weight: var(--title-weight)`) also resolves to its face.

## 2.1.3 - 2026-11-09

Theme switching and text inputs for storefront-style apps. Update the
Composer package (`pam composer update pushinbr/pam-desktop`); the launcher
downloads the verified 2.1.3 host. No breaking changes: DesktopUI themes still
take plain `SelectTheme`/`ToastTheme` values.

### Fixed

- **Theme tokens switch with the scheme.** A `:root` block inside a matched
  `@media` (`@media (prefers-color-scheme: light) { :root { --bg: #fff; } }`)
  now redeclares the custom properties for the whole stylesheet: rules outside
  the query, their state blocks and the rules of other matched queries read
  the new values and keep their cascade order. Before, only rules written
  inside the query saw them, so the skeleton's dark tokens never applied.
  State blocks (`:hover`, `:focus`…) inside a matched query now apply too.
- **Packaged font families with several weights.** An element that inherits a
  `@font-face` family and sets its own `font-weight` keeps the weight next to
  the resolved face, so the GTK host (which names a packaged face by its
  family) draws the Bold file instead of the Regular one.
- **Text inputs draw the CSS font on Linux.** `Input` (single and multiline)
  and the fields of `Select`/`Autocomplete` resolve their `font-family` list
  like `Text` (packaged `@font-face` faces, installed names, generics) with
  the weight and size. Before, a list that started with a packaged face
  (`font-family: var(--font-sans)` with `@font-face`) left GTK without a
  family and the field drew a fallback face. macOS and Windows already
  shared the `Text` font path.
- **`:root` text styles follow the scheme.** `:root { color: var(--fg) }`
  and the other inherited text properties on `:root` resolve again with the
  custom properties of a matched scheme query and `StyleVariables`, instead
  of once per stylesheet.

### Added

- **Live theme switching for DesktopUI.** `Select`, `Autocomplete`,
  `MultiSelect`, `DatePicker`, `Checkbox`, `Toast` and
  `Toast::service()->configure()` take a theme or a `Closure` that returns
  one; it is called on every render, and a light/dark switch renders every
  component again, so controls (open popups and toasts on screen included)
  switch at once. `SelectTheme::adaptive(light:, dark:)` and
  `ToastTheme::adaptive(light:, dark:)` pick by the effective scheme;
  `resolve()` returns the theme a value or Closure stands for. See
  docs/desktop-ui.md, "Live theme switching".
- **CSS `color` tints `ActivityIndicator`.** `.spinner { color: var(--muted) }`
  colors the spinner and follows the scheme; a `color` attribute still wins.

## 2.1.2 - 2026-11-09

Sharper, Chromium-exact text on Linux, an opt-in Chromium text pipeline on
macOS, Windows fixes and a first run that works. Update the Composer package
(`pam composer update pushinbr/pam-desktop`); the launcher downloads the
verified 2.1.2 host. No new properties: applications written for 2.1 run
unchanged.

### Added

- **Chromium text on macOS (opt-in).** Set `PAM_DESKTOP_TEXT_GAMMA=1` to draw
  glyphs the way Chromium does on macOS: Skia's glyph masks at quarter-pixel
  positions, with Chromium's smoothing and gamma for
  `-webkit-font-smoothing: auto`. Masks are cached per process (512 KiB at
  most) and released while no window is visible. It costs some memory and
  scroll CPU today, so Core Graphics text stays the default.
- **macOS capture input `scroll:x,y,dy`.** `PAM_DESKTOP_CAPTURE_INPUT` can
  turn the wheel over a point, for scroll measurements.

### Changed

- **Linux text now matches Chromium pixel for pixel, at no extra cost.**
  Glyphs are rasterized with a port of Skia's analytic anti-aliasing and
  Skia's mask gamma, and placed at Skia's quarter-pixel positions, so black
  text is identical to Chromium's and coloured text is within one level.
  It is the default and uses the same memory and CPU as GTK's own glyphs;
  `PAM_DESKTOP_TEXT_GAMMA=0` switches back to GTK's glyphs.

### Fixed

- **macOS text size.** Fonts are sized to the CSS size truncated to 1/64 px,
  like Chromium, so fractional sizes such as `0.78rem` lay out with
  Chromium's advances.
- **Windows: views with `backdropBlur`.** When Direct2D fails to flush a
  frame under a backdrop blur, the window paints again without the blur
  instead of failing on every frame (a modal with a backdrop blur could never
  show).
- **Windows: PHP download.** Installing PHP from windows.php.net gives up on
  a stalled connection instead of waiting forever.
- **Windows: printing with no printer.** Printing reports that no printer is
  installed instead of a generic error.
- **`pam desktop init` in an existing project.** An existing `composer.json`
  is completed with the `App\` autoload and the `dev`, `start`, `build` and
  `test` scripts, so the new application no longer stops with
  `Class "App\Navigation\AppRouter" not found`.
- **PHP through the PAM CLI.** When the host finds `pam` inside its
  installation it starts PHP through `pam-run`, so `pam desktop dev` no
  longer stops with `libsodium.so.23: cannot open shared object file` on PAM
  2.2.11 and earlier.
- **Getting started.** The README and the guide teach the PAM CLI flow:
  install PAM, `pam init my-app --template desktop`, then
  `cd my-app && pam desktop dev`.

## 2.1.1 - 2026-11-09

A pixel-parity patch for 2.1. Update the Composer package
(`composer update pushinbr/pam-desktop`); the launcher downloads the
verified 2.1.1 host. No new properties: 2.1.0 hosts keep running the same
applications.

### Fixed

- **Linux text size.** Font faces are sized in FreeType 26.6 (truncated to
  1/64 px) like Skia, so fractional sizes such as `0.78rem` shape and
  rasterize like Chromium; long lines no longer drift a pixel to the right.
- **macOS text fields.** Unfocused `<input>` and `<textarea>` values and
  hints paint from the content box with the CSS line box and
  `-webkit-font-smoothing`, matching Chromium's inner editor instead of
  AppKit's inset and stem darkening.
- **`<Slot :content>` inheritance.** Components built in PHP and placed with
  `<Slot :content>` now inherit text styles (font family, smoothing, line
  height) from where they render, including `:root`.
- **macOS classic scrollbar.** Scrollers with a reserved gutter and no
  `scrollbar-width`/`scrollbar-color` draw Chromium's default classic bar
  instead of the overlay scroller; an authored `scrollbar-color` track is
  painted too.
- **macOS `filter: blur()` without Metal.** Plain decorative boxes are blurred
  on the CPU like Skia when Core Animation has no Metal device (virtual
  machines, remote sessions).

### Tooling

- `PAM_DESKTOP_DUMP_FRAMES` nodes include their declared corner radii, for
  comparison with Chromium's computed `border-radius`.

## 2.1.0 - 2026-11-09

PAM Desktop 2.1 makes native windows lay out and paint like Chromium. A
screen written with web CSS (Tailwind, PrimeVue-style themes) now lands on
the same device pixels as the same markup in Chrome or Electron, measured
side by side on Linux and macOS. Update the Composer package and the host
together (`composer update pushinbr/pam-desktop`); the launcher downloads
the verified 2.1.0 host. Older hosts keep running applications that do not
use the new properties (2029-2043, see Compatibility).

### Highlights

- **Chromium-exact layout.** Frames on Blink's LayoutUnit grid (1/64 px),
  Blink's rounding of fixed lengths, line heights and centring, flex
  shrinking weighted like CSS, form controls (`<input size>`, menulists)
  sized like Blink, and pixel snapping that breaks ties like Chromium.
- **New CSS support.** CSS tables (`display: table*`, `table-layout`,
  `border-collapse`, `vertical-align`), inline formatting contexts
  (`display: inline`, `inline-block`, `inline-flex`, `inline-grid`,
  `vertical-align`), `position: sticky`, two-axis `overflow: auto`,
  classic and thin scrollbars with `scrollbar-color` (thumb and track),
  `scrollbar-gutter`, `:focus-visible`, `ch` units, `min-width: auto`,
  `line-height: normal` and `calc()` line heights, `will-change: transform`
  layers, `-webkit-font-smoothing`, Chromium's `font-family` fallback rules.
- **Skia-exact effects.** `backdrop-filter: blur()`, translucent scrims and
  dialog `box-shadow` follow Skia's 8-bit pipeline (new
  `pam-desktop-pixels` crate); macOS without Metal blurs on the CPU.
- **Developer tools.** `vendor/bin/pam-desktop check` lists every template,
  CSS and route diagnostic without opening a window; unknown attributes on
  built-in tags are `PAM2401` diagnostics with a suggestion;
  `PAM_DESKTOP_DUMP_FRAMES` plus `scripts/parity/` compare a native screen
  with Chromium's boxes edge by edge.
- **Charts, canvas and QR.** Chart.js-faithful axes and bars, canvases drawn
  1:1 like an HTML `<canvas>`, QR codes with margin, stretch and image modes.
- **Performance.** New features cost nothing when unused: frame dumps and
  the text gamma atlas are off by default, the backdrop compositor only runs
  for open dialogs (one window-sized texture) and the AppKit CPU backdrop is
  limited to ten updates a second when pixels changed.

### Changes

- PHP's default time zone follows the system (`TZ`, `/etc/localtime`) when
  php.ini sets none (`Pam\Desktop\System\SystemTimezone`, applied by
  `App::run()`), instead of PHP's UTC fallback.
- `Pam\Desktop\System\TextMeasure` measures single-line text widths on the
  host for code that lays text out in PHP (chart axes and legends).
- QR (`pushinbr/pam-desktop-qr`): `margin` (quiet zone in modules),
  `stretch` (fractional modules that fill `size`) and `image` (a bitmap
  scaled into the box, like a QR image service).
- DesktopUI: `ToastTheme` tokens for `ToastService` (surface, border,
  severity edge); the `DatePicker` icon uses the theme icon font when set.
- Backdrop filter like Chromium (measured on Electron 37, identical on Linux and macOS): the blur is Skia's three-box Gaussian (box `⌊σ·3√(2π)/4 + ½⌋`), transparent beyond the box and drawn over the unblurred backdrop, and the box's translucent fill is blended in 8 bits (`fill + ⌊dst·(256 − α)/256⌋`, also per stacked mask). New `pam_desktop_ui::backdrop` (`composite`, `skia_over`, `backdrop_fill`). GTK composites the fill into the blurred backdrop in one colour-matrix pass (the 8-bit store rounds it to Skia's floor); AppKit's CPU backdrop runs `composite` and the box paints no fill; Win32 composites blur over the copy and the fill with Direct2D effects (blur border soft, σ = the CSS length, it was half). A Modal's own box no longer frosts the window a second time under its mask. Two stacked slate-950/70 masks now read (1,5,22) like Electron, not (2,5,22)/(4,8,24).
- Dialogs over a frosted mask paint like Chromium: Chromium draws the dialog's `box-shadow` inside the mask's layer and the dialog's translucent background (and the translucent fills inside it) as its own layer, all in 8 bits. The modal presenter now hands the first box of a dialog (and, when its fill is translucent, the plain translucent fills inside it that hover/press do not restyle and that sit in unscrolled containers) to the backdrop as `Decoration::backdrop_boxes`; those boxes then paint no shadow (`box-shadow` overlay) and no fill (`Decoration::fill_composited`). Only backends that opt in through `Backend::composites_backdrop_boxes` (`BackdropBoxes::{None, Topmost, All}`) get them: GTK for the topmost dialog (its backdrop is rendered with the window's renderer shortly after a change, composited and shown as a texture; the GPU path stays until it is ready; one window-sized texture while the dialog is open) and AppKit without Metal for every dialog. The presenter restyles on frame changes inside the dialog, on scroll and when the stack changes. The exact Skia pipeline (three-box blur, shadow mask `(α·(mask+1))>>8`, 8-bit layers) lives in the new `pam-desktop-pixels` crate, built optimised in development too (a 1200×800 window: ~45 ms dev, ~30 ms release).
- Fonts: `font-family` lists resolve like Chromium on every platform: Safari's `ui-monospace`, `ui-serif`, `ui-sans-serif`, `ui-rounded` and `-apple-system` match nothing (measured in Electron 37 on Linux and macOS), `BlinkMacSystemFont` is the UI font on macOS only. Tailwind's `font-mono` now lands on Menlo on macOS (was SF Mono), as in Chromium.
- AppKit: without a Metal device (virtual machines, some remote sessions) `backdrop-filter: blur()` is done on the CPU (Chromium's pipeline below, encoded colour space) into a view just below the box: the window's layer tree is rendered (after a display pass, so views drawn in the same update are in it) without the box and what paints after it, at most ten times a second and only after a view changed and when the pixels under the box differ. Before, the box was shown unblurred.
- Layout: form controls are sized like Chromium. A single-line `<Input>` without a width is Blink's text field: `inputSize` (HTML `size`, property 2041; 20 by default) times the primary font's average character width, plus the maximum character width minus one average, rounded up, plus padding and border (`TextFieldIntrinsicInlineSize`; families Blink distrusts — Helvetica, Times, `.`-prefixed system names… — use the width of `0`). Hosts report the metrics like each Skia port: GTK the OS/2 `xAvgCharWidth` and head bounding box (HarfBuzz tables), AppKit the `hmtx` advance of `x` and the rounded ascent, Win32 the DirectWrite OS/2/head tables (new `Backend::text_control_metrics`, `PamTextMeasureRequest::flags`, `PamTextMeasureResult::{avg,max}_char_width`). `menuListOptions` (property 2042) on a `<select>` label `Text` makes it as wide as its widest option, rounded up (Blink menulist). CSS `min-width: auto` (property 2043) is a flex item's automatic minimum (its min-content width) instead of 0. Measured against Electron 37 on Linux (DejaVu) and macOS (San Francisco, Arial, Helvetica). Older hosts drop the three properties.
- Styles: `scrollbar-color: <thumb> <track>` paints the track (and corner) of the default 15 px classic bar (`PropKey::ScrollbarTrackColor`, property 2040; older hosts drop it and keep a transparent track).
- Modals: dialogs shown in the same render stack in tree order (a later one on top), not in hash-map order; with `backdropBlurRadius`, a dialog stacked over another frosts the dialog below too (GTK recorded only the content before the first backdrop).
- GTK text: every line of a multi-line text lands on the pixel of its exact line-box top, like Blink snaps each line box (lines a fractional line-height apart, e.g. 19.5px, drifted a pixel from the second line on).
- Canvas: `PropKey::CanvasBitmap` (property 2039) paints a canvas like an HTML `<canvas>` bitmap (`floor(size × scale)` device pixels, clipped, drawn 1:1 into the pixel-snapped box — new `DrawCommand::Bitmap` in GTK, AppKit and Win32). Charts' `integerBitmap()` uses it instead of stretching the bitmap to the fractional size.
- CLI: `vendor/bin/pam-desktop check [--json] [--src=<dir>]` compiles every component template and stylesheet of the app (the `App::components()` roots of its entry file) in a private cache and lists every diagnostic as `file:line:col CODE message`: template contracts (`PAM2401`, every one instead of the first), compile errors (`PAM2402`), invalid CSS (`PAM2403`) and route errors (`PAM2404`); no host is downloaded or started and no window opens; exit 0 when clean, 1 otherwise.
- Templates: an unknown attribute on a built-in tag (`<Text color=…>`, `maxLines`, typos) is a `PAM2401` diagnostic with file:line and a suggestion ("did you mean textColor?") at component load (dev overlay, tests), no longer ignored. DesktopUI `DatePicker` dropped its no-op `accessibilityHidden`.

- Text/styles: `line-height: <px>` rounds to 1/64 px like Blink (18.56px → 18.5625; was floored); unitless line heights multiply in float like Blink's `MinimumValueForLength` (12px × `calc(1/.75)` = 16, 12.8px = 17.046875, measured); `line-height: calc(<numbers>)` is a factor (Tailwind v4 `text-*`); a `calc()` of px/rem/numbers resolves at compile time (box sizes stay LayoutUnits); CSS math reads `1.25` as one number (it split into `1` `.25`).
- Charts: `AxisStyle(fitEdgeLabels: true)` ports Chart.js scale fitting for offset (bar) category axes, measures labels with the host (`text.measure`, kerning like canvas `measureText`) before the advance tables, places tick labels with Blink's `textBaseline: 'middle'`, and draws bars like Chart.js (grouped and stacked: half-grid base, `inflateAmount` 0.33, circular corners). Canvas text (GTK) snaps its baseline to whole device pixels like Skia's axis-aligned glyph runs.
- GTK: a translucent fill partly over an earlier sibling's shadow is composited like Skia outside that sibling's painted area instead of not at all.
- Docs/tests: a unitless `line-height` is Blink's `MinimumValueForLength(percent, ComputedFontSizeAsFixed)` (font size rounded to 1/64 first), so `22.4px × 1.5` is 33.609375 while a pre-multiplied `line-height: 33.6px` floors to 33.59375 (`LayoutUnit::FromFloatFloor`) and drifts 1/64 px per line; ports of web styles should keep the inherited number (`line-height: 1.5`) instead of the px product.
- Text (GTK): `PAM_DESKTOP_TEXT_GAMMA=1` now draws text from one glyph atlas shared by every view of the process instead of a texture per paragraph: glyphs rasterized once per font, glyph, quarter-pixel phase and colour, corrected with Skia's gamma/contrast tables like Chromium, packed into pages, bounded to 4 MiB (least recently drawn pages first) and released when no window is mapped. Positions, selection, decorations, letter-spacing, inline boxes and ellipsis are unchanged; colour glyphs stay GSK's. Still opt-in (see Text rendering).
- Scrolling: `overflow: auto` on a `ScrollView` scrolls both axes in one viewport (property 2038, `ScrollBothAxes`), with Chromium's classic bars on each overflowing axis and the corner; `scrollbar-color` alone styles the default 15 px classic bar. Tables measure single-line texts and inline boxes at their CSS min-content width (longest word, nowrap lines whole), so a table wider than its scroller overflows like Chromium's.
- Layout: centred flex items use Blink's LayoutUnit halving (toward zero on the 1/64 px grid); icon fonts take Blink's unitless `line-height: 1` (the LayoutUnit-rounded font size); a field's text (GTK) starts at the exact fractional left edge of its box.
- Layout: CSS inline formatting contexts (property 2037, `InlineLevel`): `display: inline`, `inline-block`, `inline-flex`/`inline-grid` lay boxes on line boxes that start with the parent's strut, aligned on the baseline by `vertical-align` (`baseline`, `middle`, `top`, `bottom`, `text-top`, `text-bottom`), with anonymous block boxes around inline runs among blocks; inline texts are boxed on the font's content area and fields without lines (`<textarea>`) sit on their bottom margin edge. Hosts report Blink's first baseline from text measurement (GTK, AppKit), and `TextContent::inline` places an inline box's line box over its content area on every backend.
- Text: `display: inline-block` on a nested `Text` (property 2029, `TextInlineBlock`): the box's vertical padding and border take room in its line, like a browser `<kbd>`.
- Text (GTK): baselines placed like Blink (rounded unhinted ascent/descent, floored half-leading), lines wrapped at the engine's exact width with glyphs from the exact x (`Backend::set_text_geometry`), measured widths and line heights on Blink's LayoutUnit grid (1/64 px). Without an XSettings daemon on X11 the font hinting follows fontconfig.
- Layout: frames on the LayoutUnit grid (1/64 px, edges rounded), so pixel snapping breaks .5 ties like Chromium instead of by float drift. `scrollbar-gutter: auto` with an authored `scrollbar-width` reserves a classic bar only while the content overflows (horizontal scrollers take its height).
- Development: `PAM_DESKTOP_DUMP_FRAMES=<path>` writes every node's layout (LayoutUnit), content and painted frame as JSON; `scripts/parity/` compares them with Chromium's `getBoundingClientRect()` (see Development › Layout parity with a web build).
- Layout: flex shrinking is weighted by the inner (content-box) base size and `justify-content: center`/`flex-end` keep centring overflowing content, like CSS. Icons (`Icon`) are one em tall (`line-height: 1`), like icon fonts on the web.
- CSS tables (properties 2030-2033): `display: table`/`table-row-group`/`table-row`/`table-cell`, automatic and fixed column widths, `border-collapse: collapse` (CSS 2.1 shared half borders, painted once) and `vertical-align` of cells.
- `ch` lengths for sizes (property 2034, resolved with the element's font); `line-height: normal` is Chromium's (rounded ascent + descent) and overrides an inherited or element-wide line height.
- Icons keep their own font and size inside containers that set `font-size`/`line-height`.
- DesktopUI `DatePicker`: the field icon is the theme icon font's `calendar` glyph when `SelectTheme::iconFont` is set; without one it is drawn like Chromium's `<input type="date">` picker indicator.
- Text: line boxes snap to whole pixels from the exact top of the box and centre in its exact height, not in the snapped frame (`Backend::set_text_geometry` takes a `TextGeometry`: wrap width, exact left/top offsets and height). Half-pixel tops round down the page like a browser's.
- Text: the block's own font sets up every line box (the CSS strut) when all of its characters sit in spans of other fonts, so a monospace span in a sans line sits on the sans baseline. A nested `Text` (span) carries a CSS `font-family` list.
- Fonts (GTK): `font-family` lists resolve like Chromium on Linux: `ui-monospace`/`ui-serif`/`ui-sans-serif`/`ui-rounded` and uninstalled names are skipped instead of becoming Fontconfig aliases (metric-compatible equivalents such as Courier New → Liberation Mono still match).
- Layout: fixed lengths (margin, padding, gap, width/height, min/max sizes, insets; not borders) truncate to 1/64 px like Blink's `LayoutUnit(float)` for `kFixed` lengths (`length_functions.cc`): `margin-top: 10.4px` is 10.390625, so boxes down a page no longer drift 1/64 px each from the float values.

## 2.0.1 - 2026-11-08

### Fixed

- The released Linux host runs on glibc 2.35 and GTK 4.6 again (Ubuntu and
  Pop!_OS 22.04, Debian 12 and later). The 2.0.0 host was built on Ubuntu
  24.04 against GTK 4.14 and stopped at start on older systems with
  ``GLIBC_2.39' not found`` and `undefined symbol:
  gdk_file_list_new_from_array`. The Linux hosts are now built on Ubuntu
  22.04, the GTK backend is compiled against GTK 4.6, and each GTK function
  newer than 4.6 is looked up when first used: GTK 4.8 scroll units, picture
  content fit and file lists; GTK 4.10 alert and file dialogs, URI and file
  launchers, snapshot masks, scaled textures and widget colors; GTK 4.12 CSS
  loading from a string; GTK 4.14 path strokes and screen reader
  announcements. A newer GTK keeps using them; on an older one the backend
  falls back to the GTK 4.6 equivalent (`GtkMessageDialog`,
  `GtkFileChooserNative`, `gtk_show_uri`, `GtkAppChooserDialog`, the
  `org.freedesktop.FileManager1` interface, a `text/uri-list`, a color
  matrix for image tints, Cairo for dashed and gradient borders). The
  toggle button accessible role (GTK 4.10) is a button before 4.10, and
  announcements are not spoken before 4.14.
- CI builds and tests the host on Ubuntu 22.04 too, and the release fails
  when a Linux host needs a glibc newer than 2.35
  (`scripts/check-glibc.sh`).

## 2.0.0 - 2026-11-08

PAM Desktop 2.0 renders with native toolkit widgets (GTK 4 on Linux, AppKit on macOS, Win32/Direct2D on Windows) instead of a web view. The first part of this entry covers the work since the 2.0 foundation; the foundation itself is listed after it.

Changes on top of the 2.0.0 tree below, not in a tag yet. Linux (GTK 4) is
where everything here was run. macOS was run in a virtual machine (see
Platform); Windows was only type-checked by cross compilation.

### Added

CSS and text

- `scrollbar-gutter: auto | stable | stable both-edges` on vertical scroll
  views and lists: the gutter is reserved whether or not the content
  overflows (15 px, 10 px with `scrollbar-width: thin`, the authored length,
  or nothing with `none`).
- `scrollbar-width: auto | thin | none | <length>` (at most 64 px) and
  `scrollbar-color: <thumb> <track>` (the thumb colour; the track stays
  transparent). With a reserved gutter a styled bar is a classic bar, always
  shown and draggable; `thin` follows Chromium's thin bar (a 10 px track with
  the thumb inset 2 px).
- `placeholder-color: <color>`, a shorthand of `::placeholder { color }`.
- Line breaking: `overflow-wrap` / `word-wrap: anywhere | break-word` and
  `word-break: break-word` break long words only when they do not fit;
  `word-break: break-all` breaks anywhere. `hyphens: auto` draws a hyphen
  where a word is broken (GTK).
- `font-family` keeps the whole list (`"Inter", system-ui, sans-serif`): each
  backend uses the first family it has. `system-ui`, `-apple-system`,
  `BlinkMacSystemFont`, `ui-sans-serif` and `sans-serif` are the platform UI
  font, `monospace` / `ui-monospace`, `serif` / `ui-serif` and `ui-rounded`
  their platform counterparts, `asset://` entries packaged fonts.
- Packaged fonts (`asset://…ttf`) are loaded by every backend: Fontconfig on
  GTK, CoreText on AppKit, a DirectWrite collection plus a private GDI font
  for inputs on Win32 (at most 32 files, 8 MiB each, 64 MiB in all).
- `:focus-within` state selector, and per-side border colours in state
  styles.
- Fractional grid tracks: `grid-template-columns` with `fr`, `minmax(<px>,
  <n>fr)` and `repeat()`, `GridTracks` and `Grid::tracks()`
  (`docs/grid-tracks.md`).
- Inline boxes in nested text: a nested `<Text>` / `Text::make()` with
  padding, a border, or a background and a radius is laid out like a CSS
  `<span>` / `<kbd>` (`TextSpanBoxes`).

Components and templates

- `VirtualizedList` and `ScrollView` with `max-height` follow their content
  and only scroll past the maximum. A vertical `VirtualizedList` without a
  height takes the height of its rows, up to the window height, and shrinks
  by default. `column-gap` / `row-gap` space the cells of a vertical list.
- `backdropBlurRadius` (or `backdrop-filter: blur()`) on a `Modal` frosts the
  content behind its backdrop; ignored on a transparent modal.
- Typed drag and drop inside the application: `DragSource::make()->payload(
  $type, $data)`, `DropTarget::make()->accepts($type)` with `onDrop`,
  `onDragEnter` and `onDragLeave` (`DragPayload`); auto-scroll near the edge
  of a scroll view; drags in secondary windows.
- `captureKeys="Space,Enter,…"`: the listed keys go to the nearest focused
  `on:keyDown` before native handling.
- `Input::decimal($minDigits, $maxDigits, $prefix, $suffix, $locale)` and
  `inputFormat="decimal"`: a locale-aware decimal field that reports an ASCII
  number.
- Template expressions: qualified enum cases and class constants
  (`\App\Kind::Tab`, `\App\Kind::Order->value`), public instance methods of
  values (`$order->status->label()`), `min()`, `max()` and `array_slice()`.
- `<Slot :content="$expr">` renders a component or element held in a
  property, with the slot's children as the fallback.
- Percentage values on `width`, `height`, `min*` and `max*` attributes
  (`width="75.5%"`), also when bound.
- `#[State(hotReload: false)]` keeps a property out of the hot reload
  checkpoint (passwords, in-flight flags).

Modules

- `WebSocket::connect()`: `wss://` (and `ws://` to loopback or in
  development), text messages up to 64 KiB, at most 4 connections, bounded
  queues.
- `SQLite::transactionIf()`: runs statements in an `IMMEDIATE` transaction
  only when a read-only guard query returns 1.
- `Files::save($contents, $defaultName, …)`: the native save dialog, up to
  1 MiB, replaced atomically, never through a symlink.
- `SecureStorage::get()` takes an optional failure callback that tells an
  unavailable keychain from a missing key.
- Native printing (`printing` module, `pushinbr/pam-desktop-printing`):
  text documents and receipts through the print dialog of each system.
- Canvas: vector paths (`M/L/C/Q/Z`) with solid or linear gradient fills,
  single-line text, and pointer events.

Packages

- `pushinbr/pam-desktop-ui` (`Pam\DesktopUi`): `Select`, `Autocomplete`
  (with a select mode: filter in the popup, a selected value, clear),
  `MultiSelect`, `DatePicker` (date, date and time, range;
  `rangeSeparator`, `showFieldIcon`, `DatePickerLocale::system()`), `Toast`
  (positions, severities, life, pause, a 1.5 px backdrop blur), `Checkbox`,
  `ProgressButton`, `MediaTile`, `DialogViewport` and `SelectTheme`.
- `pushinbr/pam-desktop-charts`: `LineChart`, `AreaChart`, `BarChart`
  (`barThickness()`), `DonutChart` (`legendInline()`), `Sparkline` and
  `ProgressRing`; the value axis picks 1-2-5 steps from the data, like
  Chart.js.
- `pushinbr/pam-desktop-canvas`: the drawing library the charts are built on.

Host and tooling

- `pam-desktop init` writes a small application: a router, a home screen as
  a `.pam.php` template with scoped CSS, design tokens, a service, a helper
  and a test.
- `dev`, `run`, `build` and `publish-update` without an `app` argument run
  the project of the current directory (the nearest `pam-desktop.json` going
  up). `pam desktop dev` runs it through the PAM CLI.
- PHP discovery: `PAM_DESKTOP_PHP`, then `pam` on `PATH` or in
  `~/.local/bin`, then `php`; on Linux and macOS the PAM runtime is installed
  when none exists. On Windows the official PHP 8.5 NTS x64 build is
  downloaded from windows.php.net, checked against its published SHA-256 and
  installed under `%LOCALAPPDATA%\PAM Desktop\php`.
- `build` prebuilds the component templates of the bundle; on Windows it
  ships only the DLLs PHP actually imports, plus the Visual C++ runtime.
- `dev` registers the URL schemes of the manifest for the run on Linux and
  restores the previous handlers afterwards.
- `PAM_DESKTOP_CAPTURE=<file.png>` (macOS) writes the first window to a PNG
  from inside the process.

### Changed

- Hot reload polls every 150 ms and settles for 80 ms (was 350 and 120):
  save to updated window went from a median of about 555 ms to about 315 ms
  on the skeleton (Linux, 22 samples, 181 to 346 ms); fixing an error clears
  its report in about 0.2 to 0.25 s.
- The desktop property table ends at 2028 and the event table at 2007
  (were 2016 and 2005). The SDK refuses to send a property an older host did
  not declare (`ProtocolMismatchException` naming the property); update the
  host and the SDK together (`docs/compatibility.md`).
- Template components compile one by one: a component that fails is reported
  as a boot error and the others keep working. Symlinked directories and
  files are followed (each real directory once). `__DIR__` and `__FILE__`
  in a `.pam.php` point at the source, also in a bundle.
- Error reports name what failed (Template error, Syntax error, PHP worker
  stopped), point at the `.pam.php` line instead of the compiled cache, and a
  PHP fatal error that ends the worker is reported (`RuntimeErrorPhase::Worker`).
- `?:`, `??`, `||` and `&&` in template expressions only evaluate the branch
  that is taken.
- A media query rule now replaces an earlier declaration of the same
  property when the two compile to different keys (`width: 350px` then
  `width: 100%`).
- A `Button` is at least as wide as its label plus padding (minimum 100).
- The loading `Button` and `ActivityIndicator` are drawn as dot spinners.
- `useNetwork` stops sampling while the application is in the background.

Performance (measured against an Electron build of the same application,
see `docs/performance.md`)

- GTK, software rendering (no GPU: virtual machines, containers, remote
  sessions): while a scroll view scrolls, the rows on screen are drawn from
  images rasterized once, placed on whole pixels, and rows off screen are
  left out of the frame; a second after the scroll the images are released.
  Host CPU while scrolling 2500 products: 1.56 to 0.99 s down, 1.68 to
  0.65 s up.
- GTK: while a scroll view moves, its frame clock keeps a steady 16.7 ms
  cadence. Without a compositor reporting presented frames, GTK timed each
  frame from the late start of the previous one (18 to 19 ms, about 53 frames
  per second).
- GTK: the tick callback that drives the host's animations stays installed
  while frames keep being requested instead of being added for every frame.
- The drain a backend event starts (a scroll, a resize, a scroll offset GTK
  changes during layout) handles backend events only; worker frames and
  module results are applied by the worker's own pump at idle priority,
  after the frame being drawn.
- A structural patch (rows joining or leaving a list) lays out only what it
  touched and their ancestors, as property patches already did; it laid the
  whole tree out again before. Debug builds check every such pass against a
  full layout.
- GTK: freed heap memory is returned to the system (`malloc_trim`) half a
  second after the windows stop painting, when more than a megabyte was
  freed. Host memory after 18 screens: 78 to about 63 MB.
- The PHP worker no longer keeps the style IR, its bytecode, source map and
  compatibility report of each component's stylesheet: rendering never reads
  them, and they were about 12 MB of an application's 53 MB heap after 18
  screens. Components compiled at run time no longer store them either.

### Security

- PHP is no longer installed without consent: a terminal prompt says what is
  downloaded, from where and into which folder; `--yes` or
  `PAM_DESKTOP_AUTO_INSTALL=1` agree beforehand, and without a terminal the
  host stops with instructions.
- Linux/macOS: the PAM runtime is no longer installed with `curl … | sh`. The
  host verifies the release's Ed25519-signed update manifest against the
  pinned PAM release identity (as `pam self-update`), then the archive's
  signed size and SHA-256, and unpacks it in Rust.
- Windows: PHP archives must match a SHA-256 pinned in the host as well as the
  one in `releases.json`; unknown builds fall back to the newest pinned one
  (`PAM_DESKTOP_PHP_ALLOW_UNPINNED=1` to opt out).
- Updates: optional signed `expiresAtUnix` (`publish-update --expires-in`),
  refusal of feeds older than one already seen, Linux archives unpacked in
  Rust, and `ditto`, `codesign`, `ps`, PowerShell and `tasklist` run from
  absolute system paths.
- Bundles: a symlink may leave the project only from the project's `vendor/`
  into a package listed in `vendor/composer/installed.json`.
- Components: symlinks are followed only into the project, its `vendor/` or a
  declared `path` repository; `asset://` fonts are resolved to their real path
  and must stay inside the application.

### Fixed

- Bundles: `pam-desktop build` compiled the prebuilt components in
  development mode while an installed application runs in production
  (strict) mode, so every component was compiled again at the first launch
  and the pack was never used. They are compiled in the mode the bundle runs
  in.
- Updates: an update the helper refuses after the application closed (a bundle that does not match its `manifest.json`, an archive that does not match, a rolled back swap) starts the installed version again, removes the staged archive and extraction, and records the release so the `automatic` policy does not download it and close the application at every start. Before, the application stayed closed and the next start repeated the loop, leaving about 180 MB per attempt beside the bundle.
- Updates: the host leaves for the update helper with `_exit` on Linux and macOS; `exit` from the updates worker ran the GTK and GL exit handlers while the interface thread drew and crashed the process (segmentation fault on every installation).
- Classes declared only inside `@media` or `@container` were reported as
  unknown classes.
- GTK: text is laid out with subpixel glyph positions, as Chromium does;
  Pango had rounded every advance and made words up to about 2 px wider
  (`R$ 37,50` bold 18 px: 92 px, now 89.59 px).
- GTK: a view with `opacity: 0` can be clicked again, as in CSS.
- `pointerEvents` reaches native hit testing on every backend, so
  transparent overlay layers no longer swallow clicks and wheel input.
- A bound `:textColor` overrides the `color` of the scoped stylesheet.
- Inputs use their `line-height`; the placeholder colour falls back to the
  text colour (at 45% on AppKit, like GTK).
- `openUrl` on Linux falls back to `gio open` when `xdg-open` is missing.

### Platform: Linux

- Reference backend; everything above was run on GTK 4 (Ubuntu 24.04
  container under Xvfb and the development machine).
- New on GTK: backdrop blur (`backdropBlur`, `backdrop-filter`, modal
  backdrops), the scrollbar styles, subpixel text, inline boxes, packaged
  font stacks through Fontconfig, the print dialog (Cairo and Pango).
- Not verified: printing to a physical printer (no printer was attached;
  Print to File and its PDF were checked).

### Platform: macOS

- First real run: macOS 26.7.1 (Tahoe) x86_64 in a virtual machine without a
  GPU, Xcode 26.6, PHP 8.5.12 from PAM 2.2.9. The examples `counter`,
  `controls`, `lists` (2000 rows), `text`, `images`, `motion`,
  `navigation` and `showcase` ran in light and dark; hot reload, clicks,
  typing with masks, Tab focus, the pasteboard, Command-Q, wheel scrolling,
  a modal closed with Escape and window dragging were checked with real
  input events. A bundle from `pam-desktop build --format directory --format
  dmg` (19 MB DMG) opened; idle at its first screen for 30 s the host and the
  PHP worker used 0.0% CPU and 41 MB resident each.
- Fixed by that run: typed text never reached the core; a click left the
  focus ring on focusable boxes (now shown only after keyboard input); icon
  fonts drew a missing glyph; legacy scrollers took 15 points from content;
  inputs used the system hint colour for the placeholder; a bundle built with
  PAM shipped the launcher script instead of `pam` and `libphp.dylib`.
- New on AppKit, type-checked and run only where listed above: packaged
  fonts through CoreText, inline boxes, classic styled scrollbars in a
  reserved gutter, the print panel, `PAM_DESKTOP_CAPTURE`.
- Not verified: Apple Silicon, `blur` / `backdropBlur` on screen, menus
  opened with the pointer, context menus, drag and drop, media, web views,
  notifications, printing, signing. AppKit does not apply `word-break`,
  `overflow-wrap` or `hyphens` (words wrap at word boundaries).

### Platform: Windows

- Not run on Windows. `cargo check --target x86_64-pc-windows-gnu` of the
  backend and the host passes; nothing in this section was executed.
- Written for Win32: packaged fonts, inline boxes (DirectWrite inline
  objects), styled scrollbars, CSS line breaking and `white-space: nowrap`,
  key capture before `TranslateMessage`, typed drag and drop in secondary
  windows, the print dialog with GDI spooling off the UI thread, the
  windows.php.net PHP installer, and import-table based DLL selection in
  `build`. `hyphens` is not applied.

### Native renderer foundation

PAM Desktop 2.0 is a rewrite with a different rendering model. The interface is
no longer a web page in a Servo web view: applications are written in PHP with
the reactive component syntax of PAM Native, and a Rust host lays the tree out,
diffs it and drives real platform views (GTK 4 on Linux, AppKit on macOS,
Win32 with Direct2D and DirectWrite on Windows). PHP runs in a worker process
that exchanges binary frames with the host over stdin and stdout.

The PHP component core, the wire protocol and the layout and diff engine
started as a copy of PAM Native 1.46.34 (see `NOTICE`) and are maintained here
independently.

#### Breaking

Applications built for 1.x do not run on 2.0. The web interface, the Servo
host, the typed command gateway, capabilities, the protocol 6 manifest declared
in PHP and the plugin contracts are gone, and there is no compatibility layer.
The 1.x sources are not part of the 2.0 tree; they are in the history of the
repository, at the `v1.*` tags. See `docs/migrating-from-1.x.md`.

#### Rendering and layout

- Native views for every element: no web view in the rendering path.
- Flexbox layout, absolute positioning, percentages, aspect ratio, grids and
  wrapping computed by the engine; text measured by the platform's own shaper,
  so a box is sized by exactly the paragraph that is painted.
- Tree frames for the first render and patch frames afterwards; the engine
  turns both into mutation batches and the core pushes only what changed.
- Backgrounds, per-side borders, per-corner radii, shadows, opacity, transforms,
  clipping and z-order.
- Light and dark appearance (system or chosen by the application, persisted)
  and a text scale applied live. The dark mode, text scale and reduced motion
  settings of the system are followed while the application runs.
- `.pam` templates rendered with `View::make()`, looked up in
  `resources/views` by default.

#### Interaction and input

- Press, long press, double tap, hit slop and press retention implemented once
  in the core, identical on every backend.
- Hover, pressed and focus state styles, pointer cursors and tooltips.
- Native text fields and editors with input methods, selection, masks
  (patterns and currency), secure entry and submit handling.
- Host-drawn switch and activity indicator.
- Keyboard focus traversal, key events for the focused element and
  application shortcuts.
- Selectable text with drag, double and triple click, copy and select all.

#### Lists and scrolling

- Native scroll views with scroll events, programmatic scrolling and sticky
  headers.
- Virtualized lists that build rows on demand, with columns, sections,
  inverted lists and end-reached reports.
- Refresh with F5 or Ctrl/Cmd+R (there is no pull gesture), with a host-drawn
  spinner whose color, backdrop, size and offset the application sets.

#### Animation and gestures

- Implicit transitions, motion presets, keyframes, animation programs
  (timing, spring, sequence, repeat) and shimmer, all sampled by the host's
  frame clock.
- Gesture detectors: tap, pan, swipe and long press with the pointer; pinch
  and rotation with the trackpad or Control + wheel.
- Drag contracts with bounds, snap points and driven properties, swipeable
  rows, and native pan, zoom and rotate transforms.
- Stack navigation transitions, modal and bottom sheet presentation.
- Shared element transitions between routes, for images and for any other
  element.
- `TextEllipsizeMode::Marquee`: a line that does not fit slides to its end
  and back.

#### Images, media, web and canvas

- Images from HTTPS, bundled assets, sandbox files, absolute paths and data
  URIs, with a disk cache, cache policies, placeholders, tinting and load
  events. SVG is drawn at the size it is shown at on Linux and macOS.
- A media player with play intent, seeking, a disk cache, visible autoplay,
  and clock and progress views bound to the player by the host.
- An embedded web view with a message bridge, an allowed-host policy and an
  injected script.
- A vector canvas and a freehand drawing canvas.

#### Desktop shell

- Main window control and state, secondary windows described as elements.
- Application menu bar with platform roles, a tray icon, and context menus
  opened by a secondary click, the menu key or Shift+F10.
- Application and global shortcuts, native file dialogs and notifications.
- Drag and drop of text between elements and applications, files dropped from
  the system, window dragging from custom title bars.
- Clipboard, alerts and toasts.
- Deep links: URL schemes declared in `pam-desktop.json` and registered by the
  installers, a single instance that receives the link of every later launch,
  and `Linking::listen()` to read them.
- Application state: a minimized or hidden main window is the background
  state (`paused()`, timer background policies) and a visible window without
  the focus is the inactive state.
- `Window::progress()` on the launcher icon (the Unity launcher entry API on
  Linux, a Dock badge on macOS, the taskbar button on Windows).
- Screen capture protection (`Screen::secure()`, secure routes) on macOS and
  Windows. It is not enforced on Linux.

#### Modules

- Key/value storage, a private file sandbox with pickers and downloads, HTTP
  requests and transfers, SQLite databases, secure storage in the system
  keychain, timers, device information and links.
- Device status (battery, network type, power saving) read from each system,
  once or as a watch.
- `Share::text()` and `Share::files()` through the share sheet of the system
  on macOS and Windows, and on Linux, which has none, through a menu with
  copy, e-mail, open with and show in folder.
- Capabilities that only exist on phones in PAM Native (location, contacts,
  sensors, biometrics, SMS and others) are not included; their class names
  throw a `PluginException` naming the optional package.

#### Distribution

- `pam-desktop build` bundles the application with the host and a PHP runtime
  and builds the installers of the system it runs on: AppImage, `.deb` and
  `.rpm` on Linux, `.dmg` and `.pkg` on macOS, MSIX and MSI on Windows, and a
  portable archive on each.
- A bundle ships only the shared PHP extensions the application refers to
  (`extensions` in the manifest overrides the choice): a small application
  bundled with the PHP of Ubuntu is 40 MB instead of 120 MB.
- Signed updates: an Ed25519-signed feed, `pam-desktop update-key` and
  `pam-desktop publish-update`, and the `Updates` API.

#### Development experience

- `composer require pushinbr/pam-desktop` installs `vendor/bin/pam-desktop`, a
  launcher that downloads the host archive of its version from the GitHub
  release, verifies its SHA-256 and keeps it in the cache of the user.
  `PAM_DESKTOP_HOST` runs a host built from source instead.
- `pam-desktop init` writes `pam-desktop.json`, `index.php`, `composer.json`
  and `.gitignore` for a new application and keeps the files that exist.
- `pam-desktop dev` restarts the PHP worker when sources change and restores
  `#[State]` values; `pam-desktop run` starts the application for production.
  Both start the `entry` of the manifest and take the link the application is
  opened with.
- An error overlay drawn by the host: a toast for errors in handlers and a
  full report with source and call stack for fatal ones; in production the
  application reloads by itself and falls back to a retry screen.
- One runnable example per feature under `examples/` and `scripts/shot.sh` to
  capture them on a virtual display.
- PHPStan at level 9 over a listed subset of the PHP package
  (`composer analyse`).
- A CI workflow (Linux: build, tests, clippy, rustfmt, PHP tests and PHPStan;
  macOS and Windows: build and tests) and a release workflow that builds the
  host for six targets and publishes the archives of a tag. Neither has run
  yet.

#### Accessibility

- Roles, labels, hints, values, states, live regions and custom actions
  resolved in the core and mapped to AT-SPI (through GTK), NSAccessibility and
  UI Automation.
- `Accessibility::announce()` speaks through the screen reader of the system
  and `Accessibility::screenReaderEnabled()` reports whether one is running.

#### Platforms

- The host starts the backend of the system it was built for.
- Linux (GTK 4) is the reference backend and the only one exercised so far.
  The released Linux host is built on Ubuntu 24.04: it needs glibc 2.39
  and GTK 4.14 or newer. WebKitGTK 6 is loaded the first time a `WebView`
  is shown, so only applications with one need it installed.
- The macOS (AppKit) and Windows (Win32, Direct2D) backends are written
  against the same `Backend` contract and type-check by cross compilation, but
  have not been run on real machines yet (macOS has since run in a virtual
  machine; see Unreleased). `docs/first-run-macos-windows.md`
  is the checklist for their first run and the list of known differences.
- On Windows a release host is a windowed program that attaches to the
  console of the terminal it was started from, and embeds an application
  manifest.
- Not verified on Linux: the accessibility tree with a real screen reader, and
  global shortcuts through the Wayland portal (X11 grabs were verified).

## 1.3.4 - 2026-08-31

- Certify the Linux graphical clean-room with the EGL loader and Mesa software
  driver installed explicitly, so a freshly provisioned runner can launch the
  Servo host under Xvfb instead of depending on the runner image by accident.

## 1.3.3 - 2026-08-31

- Align the Linux AccessKit executor with the host's unified Tokio-backed D-Bus
  runtime, preserving AT-SPI accessibility without a startup reactor abort.

## 1.3.2 - 2026-08-31

- Install the XKB X11 runtime in Linux release jobs so the mandatory graphical
  clean-room can launch the production host under Xvfb before publication.

## 1.3.1 - 2026-08-27

- Run the mandatory graphical release clean-room inside an isolated Xvfb
  display, preserving real create/dev/package/install/launch certification on
  headless Linux runners.

## 1.3.0 - 2026-08-27

- Turn PAM Desktop into a Native Workstation runtime with configurable
  multi-process window isolation, native window roles, mixed-DPI restoration,
  system shell integrations, secure single-instance activation and a separate
  background agent.
- Add persistent jobs, typed bounded IPC, constant-memory streams, supervised
  PHP pools, real PTYs, incremental processes, SQLite WAL workers, coalesced
  project watching and background full-text search.
- Add strict per-plugin Linux sandboxing with explicit filesystem, network,
  shell and device grants, fail-closed unsupported-platform behavior and a
  release permission audit.
- Add signed channel-bound updates with rollback, native packaging contracts,
  deterministic host archives and mandatory Windows/macOS native-host gates.
- Add startup snapshots, deep idle, lazy capabilities, a GPU backend with safe
  software fallback, dirty-region presentation, bounded virtualization,
  docking, detachable tabs, commands, transactional history and crash recovery.
- Add native notification responses, device portals, AccessKit trees,
  authenticated automation and DevTools, symbolizable diagnostics and enforced
  performance/clean-room release evidence.

- Align all PAM Desktop Rust crates with release 1.2.5 so packaged hosts report
  the same version as their signed release artifacts.

- Add integer-coded UTF-8 and Base64 PAM HTTP request bodies so authorized
  origins can receive multipart forms and binary uploads without browser networking.
- Point the Composer launcher at native host 1.2.5 so database, HTTP,
  binary-body, secrets, process, and portal calls use the corrected bridge.

- Keep command cancellation metadata out of strict native capability payloads,
  restoring database, HTTP, secrets, process, and portal calls through the PAM bridge.
- Allow inline renderer styles while preserving the strict packaged-script and
  capability-scoped network policies, enabling Vue and PrimeVue component styling.

- Build and execute the real Servo host on macOS arm64 and Windows x64, with
  bounded commit/toolchain/hash/size smoke evidence retained for 14 days.
- Replace the redundant Servo `cargo check` plus build sequence with the
  stronger executable build alone.
- Align public documentation with the pinned Servo 0.5 engine.

- Add native macOS arm64 and Windows x64 CI that tests and lints the portable
  gateway/plugin contracts and compiles the real Servo host on hosted runners.
  These are pre-release compatibility gates, not yet a distribution claim.
- Scope XDG portal and Linux Secret Service dependencies to Linux, preserve a
  fail-closed response on other systems, and compile notification urgency and
  native menu handlers against their actual platform APIs.
- Declare Tokio's directly used asynchronous I/O extension feature instead of
  inheriting it accidentally from Linux-only portal dependencies.
- Exercise security, database and packaging fixtures with canonical paths,
  closed native handles and the required platform launcher on every host.
- Keep shared asynchronous and fallible platform contracts explicit while
  documenting their no-op or fail-closed implementations outside Linux.
- Require the native macOS arm64 and Windows x64 compatibility matrix before
  either Desktop tagged-release publication path can proceed.
- Align the embedded engine and its cryptography, JavaScript and SQLite graph
  with Servo 0.5, including its fixed macOS font implementation and bundled
  FreeType runtime.
- Preserve the host lifecycle contract on pre-release non-Linux builds while
  leaving cross-process single-instance forwarding disabled until its native
  Windows and macOS transports are certified.
- Install the declared Rust 1.88 toolchain by immutable action ref in source,
  platform and release workflows, and assert the compiler version at runtime.
- Carry a bounded, source-auditable `servo-fonts 0.5.0` macOS lifetime fix
  until an upstream crates.io release contains the equivalent correction;
  omit upstream's 14 MB test fixture corpus from the override.
- Make the complete Desktop source CI reusable and mandatory for both native
  host and API-only tagged releases, preventing publication from bypassing the
  contracts validated on `main`.
- Add explicit-opt-in, privacy-bounded OTLP HTTP/JSON spans for Desktop command
  execution with a non-blocking bounded exporter, diagnostic counters and
  official Collector interoperability certification.
- Continue validated W3C version `00` trace lineage from authenticated bridge
  invocations into distinct Desktop command child spans.
- Reserve `traceparent` and `tracestate` on capability-scoped HTTP requests so
  renderer code cannot forge outbound distributed-trace lineage.
- Add a dedicated authenticated bridge option that validates and injects W3C
  version `00` context only into a declared capability-scoped HTTPS origin.

- Add a bounded, authenticated `pam desktop diagnostics` transport for live
  development snapshots, backed by an ephemeral private session descriptor.
- Add authenticated Desktop package-footprint evidence for compressed,
  installed and executable bytes, plus a release comparison gate that rejects
  regressions beyond the accepted baseline.

## 1.2.1

- Align the Rust workspace, packaged host and Composer launcher on version
  1.2.1 after the additive 1.2.0 PHP API publication.

- Add the convention-first `App` authoring layer aligned with PAM Native:
  `#[Desktop]`, typed `#[Command]` methods and invokable classes, constructor
  injection, DTO hydration, integer-backed enum binding and ordinary PHP
  return normalization.
- Add invocation-scoped `WindowHandle`, `Windows`, typed `DesktopWindow`,
  `Events` and `ApplicationControl` services that collect native effects and
  event objects without exposing host handles or protocol envelopes.
- Add declarative `#[Window]`, `#[Menu]`, `#[MenuItem]`, `#[MenuSeparator]` and
  `#[Listen]` discovery while preserving the immutable API 1 builders as the
  advanced layer.
- Add focused authoring, typed-command, window and menu documentation plus a
  convention-first README quick start.

- Add capability-scoped, backpressured binary read/write streams without
  base64 payload expansion.
- Add capability-scoped native HTTPS with named origins, path confinement,
  bounded bodies, strict headers and integer method contracts.
- Harden Rust sidecars with optional SHA-256 pinning on every start and a clean
  inherited environment.
- Add Linux single-instance activation, deep-link/file forwarding, MIME and URL
  associations, and opt-in XDG autostart packaging.
- Add capability-gated Linux Secret Service storage with encrypted D-Bus
  sessions, application scoping and bounded UTF-8 values.
- Add manifest-allowlisted bundled process execution with integer argument
  policy, empty environments, bounded concurrent I/O and hard timeouts.
- Add capability-scoped public file watching and direct XDG portal integration
  for URI opening, interactive screenshots and native PDF printing.
- Add a development-only responsive runtime Inspector with accessible live
  aggregate metrics and no sensitive payload collection.

- Added capability-scoped native SQLite with a bundled engine, prepared
  parameters, WAL, foreign keys, bounded queries and atomic transactions.
- Added explicit stateful, parallel and background PHP command execution lanes
  backed by a lazy supervised worker pool.
- Added privacy-bounded Linux system snapshots for CPU, memory, uptime,
  connectivity, power and battery state.
- Added decorated, transparent, always-on-top, maximized and fullscreen window
  policy plus typed runtime effects for fullscreen, maximization and stacking.
- Added authenticated runtime diagnostics and a reproducible bridge benchmark
  harness.
- Expanded architecture, capability, database, execution, diagnostics, window,
  system-information and performance documentation.

## 1.1.2

- Added GitHub artifact provenance for the byte-reproducible Linux x86-64 host
  archive.
- Made release-workflow changes participate in native-host change detection so
  supply-chain changes cannot produce an API-only release without assets.
- Updated the public version record and completed the missing 1.0.5–1.1.1
  changelog history.

## 1.1.1

- Preserved secondary Servo WebViews and rendering contexts when their native
  close control is used, preventing shared EGL display invalidation on
  GLVND/NVIDIA systems.

## 1.1.0

- Added the concise `Application::make(...)`, `Window::load(...)`, and
  `Window::defaults(...)` DSL while preserving the complete stable API 1
  compatibility surface.

## 1.0.5

- Released old Servo window generations before hot-reload replacements are
  created so an old EGL connection cannot invalidate a new shared display.

## 1.0.4

- Forced bindgen and mozangle to use the Clang 18 executable and resource
  headers together on the GLIBC 2.35 build runner.

## 1.0.3

- Restored the GLIBC 2.35 build baseline for compatibility with Ubuntu 22.04,
  Pop!_OS 22.04 and newer Linux distributions.
- Kept Clang 18 for the Servo build without raising the runtime GLIBC baseline.

## 1.0.2

- Added a Composer vendor binary that downloads the matching native desktop host
  on first use, verifies its SHA-256 checksum and reuses the cached installation.
- Removed the need for a separate manual `pam-desktop` installation when a project
  is created with Composer.
- Updated the production Linux build to Clang 18 on Ubuntu 24.04.

## 1.0.1

- Published the PHP application API as `pushinbr/pam-desktop` from the
  repository root so public `pam init --template desktop` projects resolve
  without custom Composer repositories.
- Updated packaged vendor-path handling and compatibility fixtures for the
  organization-owned Composer coordinate.

## 1.0.0

- Declared public API version `1` while retaining worker protocol `6` and Rust
  plugin protocol `1` for the complete 1.x line.
- Added an exact reflection snapshot for every public PHP symbol, method
  signature, constant, enum case and promoted property.
- Added golden protocol-6 bootstrap, request, response and plugin transport
  fixtures with deserialize/validate/reserialize compatibility tests.
- Added a standalone Rust plugin consumer that compiles the complete stable SDK
  surface as part of the locked workspace.
- Added `window.pam.apiVersion`, the Rust `PUBLIC_API_VERSION` constant and
  explicit SemVer, deprecation and support policies.
- Tightened the stable Linux shell contract to one tray-backed menu and made
  invalid shell effects fail explicitly.
- Excluded standard Rust plugin scaffold sources from application bundles while
  retaining and hashing configured release executables.
- Documented that process-isolated Rust plugins are trusted native code, not an
  operating-system sandbox.
- Added deterministic Linux x86-64 host archives with a versioned manifest,
  per-file sizes and SHA-256 digests, normalized metadata and an adjacent
  archive checksum.
- Added atomic rootless XDG install/uninstall scripts that refuse unrelated
  command paths and affect only the exact installed version.
- Added end-to-end host archive validation for safe members, exact file sets,
  checksums, byte-for-byte reproducibility, installed execution and uninstall.
- Prevented a custom application output directory from entering its own bundle
  and added a byte-for-byte portable archive reproducibility regression test.
- Focused the official 1.x build, release, documentation and compatibility
  guarantee on Linux x86-64 built from Ubuntu 22.04.

## 0.6.0

- Added protocol 6 contracts for native shell configuration, supervised jobs,
  PHP plugins, Rust plugins and dynamic shell effects.
- Added immutable PHP `Shell`, `Menu`, `MenuItem`, `Tray`, `GlobalShortcut`,
  `BackgroundJob` and `RustPlugin` configuration APIs with sequential
  integer-backed variants.
- Added composable PHP plugins that register only through the public
  `Application` API and are reported in the validated boot contract.
- Added the `pam-desktop-plugin` Rust SDK with a bounded, versioned JSON-lines
  process protocol and typed metadata, results, failures and events.
- Added a Rust plugin supervisor with exact export enforcement, per-plugin
  serialization, deadlines, cancellation and crash recovery without replay.
- Added `pam desktop plugin new` and `pam desktop plugin build` scaffolding for
  project-local, release-built native extensions.
- Added native menu trees, checkbox state, status tray activation and
  close-to-tray behavior; Linux uses the D-Bus StatusNotifierItem protocol
  without a GTK/AppIndicator runtime dependency.
- Added global shortcuts with graceful registration fallback and typed
  press/release lifecycle events.
- Added periodic PHP background jobs with interruptible shutdown, initial
  delay, timeout, skip/wait overlap policy, typed effects and ordered lifecycle
  events.
- Added hot-reload replacement of the complete plugin, scheduler and native
  shell configuration without leaving duplicate timers or registrations.
- Focused tagged host artifacts and official 0.6 package validation on Linux
  x86-64 while preserving the existing non-Linux packager code for future work.

## 0.5.0

- Added protocol 5 update configuration with integer-backed policy, platform,
  artifact-kind and lifecycle-state enums.
- Added immutable PHP `Updates` configuration with pinned HTTPS feed, channel,
  Ed25519 public key and manual, notify or automatic policy.
- Added the frozen `pam.updater` API and background feed checks/downloads.
- Added strict typed feed parsing, canonical Ed25519 verification, bounded
  HTTPS transport, signed byte lengths and SHA-256 artifact verification.
- Added detached update application with parent-process coordination, complete
  bundle verification, atomic directory swap, rollback and relaunch.
- Added owner-only update-key generation and deterministic multi-platform feed
  publication without exposing the private seed.
- Added a native launcher for Windows and macOS bundles.
- Added cross-platform portable ZIP packaging, macOS `.app`/DMG metadata,
  Windows MSIX metadata and platform icon generation.
- Added hardened-runtime codesigning, optional Apple notarization/stapling and
  certificate-store Authenticode signing.
- Added Linux, macOS and Windows CI contracts and tagged host release builds.

## 0.4.0

- Added protocol 4 application manifests with validated reverse-DNS identity,
  portable versions, publisher, description, icon and bundle exclusions.
- Added sequential integer-backed `ApplicationCategory` contracts in PHP and
  Rust with Freedesktop and Debian mappings.
- Added safe PNG/SVG icon validation and generated application metadata.
- Added `pam desktop run` production mode without the development watcher.
- Added atomic `pam desktop build` staging with exact-artifact replacement only
  behind `--force`.
- Added self-contained Linux directories containing the application, Pam
  worker, Servo host, isolated `php.ini` and non-glibc runtime libraries.
- Added materialization of Composer path packages, secret/build exclusions and
  explicit rejection of unsafe non-vendor symlink escapes.
- Added deterministic portable `.tar.gz` archives with per-user install and
  uninstall scripts.
- Added optional native Debian packages with `/opt`, `/usr/bin`, Freedesktop
  desktop entry and hicolor icon integration.
- Added a sorted bundle manifest containing application/runtime/target metadata
  and SHA-256 integrity records for every shipped file.
- Updated the Pam starter with a typed 0.4 manifest, custom vector icon and
  `desktop:build` Composer command.

## 0.3.0

- Added PHP-declared native capabilities through immutable `Capabilities` and
  named `FileSystemRoot` contracts.
- Added capability-based filesystem access with read, write and read-write
  permissions, relative paths, UTF-8 limits and symlink/traversal rejection.
- Added process-lifetime opaque grants for files and directories selected by
  dialogs or dropped onto a window; frontend code never receives ambient paths.
- Added native open, multi-open, save and directory dialogs with validated
  filters and main-event-loop ownership.
- Added separately gated clipboard read/write/clear operations and persistent
  host ownership of clipboard content.
- Added native notifications with bounded title/body content and sequential
  integer urgency values.
- Added Winit drag enter, leave, drop and error events targeted to the receiving
  application window.
- Added the frozen `pam.fs`, `pam.dialog`, `pam.clipboard` and
  `pam.notification` frontend APIs.
- Added protocol 3 contracts, capability error codes and security tests for
  traversal, permissions and symbolic links.
- Updated the Pam Desktop starter with a PHP-authorized Native Lab and protocol
  3 diagnostics.

## 0.2.0

- Added bidirectional application events with `window.pam.emit`, `window.pam.on`,
  `Application::on` and `ClientEvent`.
- Added per-request deadlines, `AbortSignal` cancellation and explicit timeout,
  cancellation and crash error codes.
- Added supervised worker recovery without automatic command replay.
- Added multiple Winit/Servo windows, per-window entries and targeted title,
  visibility, close and focus effects.
- Added development hot reload with asset refresh, PHP worker restart,
  bootstrap revalidation and visible reload-error events.
- Added the protocol 2 contract, bounded event history, secure window routes and
  worker recovery tests.
- Added PHPStan level 9 validation for the public PHP package.
- Unified the public workflow under `pam desktop`; `pam-desktop` remains the
  separately shipped native host.
