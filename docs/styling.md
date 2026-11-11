# Styling

Every element is styled with a `Pam\Desktop\Style` value. This page covers `Style`, the styles that depend on interaction state, the desktop additions (cursors, tooltips, hover), dark mode and the text scale.

## `Style`

`Style` is an immutable value built with named arguments. `style()` returns a copy of the element with those properties.

```php
use Pam\Desktop\Align;
use Pam\Desktop\Style;

$card = new Style(
    padding: 16,
    gap: 12,
    borderRadius: 14,
    backgroundColor: 0xFFFFFFFF,
    borderWidth: 1,
    borderColor: 0xFFE5E7EB,
);

Column::make(
    Text::make('Title')->style(new Style(fontSize: 16, fontWeight: 600, textColor: 0xFF111827)),
    Text::make('Subtitle')->style(new Style(fontSize: 13, textColor: 0xFF6B7280)),
)->style($card);
```

- Lengths are logical units: the host multiplies them by the display scale.
- Colors are `0xAARRGGBB` integers. `0xFF2563EB` is opaque blue; `0x00000000` is transparent.
- A `Style` can be created once and applied to many elements.
- Calling `style()` again layers the new style over the previous one: properties it sets win, the others are kept.

### Properties

| Group | Properties |
| --- | --- |
| Size | `width`, `height`, `minWidth`, `minHeight`, `maxWidth`, `maxHeight`, `widthPercent`, `heightPercent`, `maxWidthPercent`, `maxHeightPercent`, `aspectRatio` |
| Flex container | `flexDirection`, `flexWrap`, `alignItems`, `justifyContent`, `gap`, `layoutDirection` |
| Flex item | `flexGrow`, `flexShrink`, `alignSelf` |
| Padding | `padding`, `paddingHorizontal`, `paddingVertical`, `paddingLeft`, `paddingTop`, `paddingRight`, `paddingBottom` |
| Margin | `margin`, `marginHorizontal`, `marginVertical`, `marginLeft`, `marginTop`, `marginRight`, `marginBottom`, `marginLeftAuto` |
| Position | `positionType`, `left`, `top`, `right`, `bottom`, `leftPercent`, `topPercent`, `rightPercent`, `bottomPercent`, `zIndex` |
| Background and border | `backgroundColor`, `borderRadius`, `borderTopLeftRadius`, `borderTopRightRadius`, `borderBottomRightRadius`, `borderBottomLeftRadius`, `borderWidth`, `borderLeftWidth`, `borderTopWidth`, `borderRightWidth`, `borderBottomWidth`, `borderColor`, `borderStyle` |
| Shadow | `shadowColor`, `shadowOffsetX`, `shadowOffsetY`, `shadowBlurRadius`, `shadowSpreadRadius`, `elevation` |
| Compositing | `opacity`, `overflow`, `visible`, `pointerEvents`, `blurRadius` |
| Transform | `translationX`, `translationY`, `translationXPercent`, `scaleX`, `scaleY`, `rotation` (degrees) |
| Text | `textColor`, `fontSize`, `fontWeight`, `fontFamily`, `fontStyle`, `lineHeight`, `letterSpacing`, `textAlign`, `textDecoration`, `textTransform`, `numberOfLines` |
| Images and inputs | `imageFit`, `tintColor`, `placeholderColor` |
| Grid | `gridColumns`, `gridMinColumnWidth`, `gridTemplate`, `gridColumnGap`, `gridRowGap`, `gridSpan`, `gridOffset`, `gridOrder` and their `Sm`, `Md`, `Lg`, `Xl`, `2xl` variants |
| Pointer | `cursor` |

An element with `opacity: 0` is invisible but still takes clicks and wheel input, as in CSS (a transparent overlay that opens a menu, for instance; GTK used to skip fully transparent widgets and no longer does). `pointerEvents` takes an element out of hit testing on every backend: `PointerEvents::None` (the element and its children), `BoxNone` (the element does not take input, its children still do), `BoxOnly` (the element takes input, its children do not). CSS: `pointer-events: none`.

On a text nested in another text, padding, borders, radius and background make an inline box, like a CSS `<span>`; see [Components](components.md#text).

Enumerated values are PHP enums in `Pam\Desktop`: `Align`, `Justify`, `FlexDirection`, `FlexWrap`, `PositionType`, `Overflow`, `TextAlignment`, `TextDecoration`, `TextTransform`, `FontStyle`, `BorderStyle`, `PointerEvents`, `ImageFit`.

```php
Row::make(/* … */)->style(new Style(
    alignItems: Align::Center,
    justifyContent: Justify::Center,
    overflow: Overflow::Hidden,
));
```

## State styles

### Pressed

A `Button` or `Pressable` dims while pressed. `Pressable` lets you choose the feedback; the host animates it without calling PHP:

```php
Pressable::make(Text::make('Archive'))
    ->pressedOpacity(0.8)
    ->pressedScale(0.98)
    ->pressedBackgroundColor(0xFF1D4ED8)
    ->onPress($this->archive(...));
```

A `Button` that sets no background, border or text color gets the neutral look of a native push button, with its own hover and pressed shades in light and dark mode. Style any of those properties and the button is entirely yours.

### Hover

For a style that depends on hover, track it in state with the hover events:

```php
Pressable::make(Text::make($name))
    ->onHoverIn(function () use ($index): void {
        $this->hovered = $index;
    })
    ->onHoverOut(function () use ($index): void {
        if ($this->hovered === $index) {
            $this->hovered = -1;
        }
    })
    ->style(new Style(
        backgroundColor: $index === $this->hovered ? 0xFF222838 : 0x00000000,
    ));
```

### State selectors in template stylesheets

Components written as single-file templates (`.pam` files, as in `examples/showcase/resources/components/`) carry a `<style>` block. There, the selectors `:pressed`, `:hover`, `:focus`, `:focus-visible`, `:focus-within`, `:disabled`, `:checked`, `:selected`, `:active`, `:loading`, `:error` and `:empty` declare styles for a state, and the host applies them with no round trip to PHP:

```css
.language-two-card {
    opacity: 1;
}

.language-two-card:pressed {
    opacity: 0.76;
    transform: scale(0.98);
}
```

The host applies these properties for a state: opacity, scale, translation, background color, text color, border color (also per side: `border-top-color` and the others) and elevation. When several states are active, hover is applied first, then focus, then pressed, so pressed wins.

A hover on an ancestor styles a descendant, as in CSS: `.row:hover .actions` reveals a row's actions while the pointer is anywhere over the row (Slack's message toolbar, a track's play button, a card's edit pencil). The host applies it with the same properties as the other states and no round trip to PHP; the ancestor is the nearest element above that the rule's `.row` matches. A view at `opacity: 0` is not hit by the pointer, so hidden actions overlapping a neighbour do not steal its hover.

```css
.actions { position: absolute; top: -16px; right: 20px; opacity: 0; }
.message:hover .actions { opacity: 1; }
```

`:focus-within` matches a box while it or one of its descendants has the keyboard focus, as in CSS. It styles the frame around an input without making the frame focusable:

```css
.search { border: 1px solid #CBD5E1; border-radius: 8px; }
.search:focus-within { border-color: #2563EB; }
```

## CSS in template stylesheets

The `<style>` block of a `.pam.php` component, and the stylesheets it imports, are compiled to the same native properties as `Style`. The selectors are classes, tags and the state pseudo-classes above; `@media` and `@container` rules apply while their condition holds. A property that has no meaning for native views (`outline`, `appearance` and similar) is accepted and ignored (`-webkit-font-smoothing: antialiased` / `-moz-osx-font-smoothing: grayscale` is inherited and, on macOS, draws text without font smoothing, like Chromium); anything else that does not compile is an error that names the file and the line.

This section covers the text, font and scrollbar properties. The table at the end says what each backend does with them.

### Responsive rules

A class may be declared only inside a media or container query: it is a known class of the component, and an element that uses it gets nothing while the condition is false.

```css
.aside { width: 350px; flex-basis: 0; }

@media (max-width: 1280px) {
    .aside { width: 100%; flex-basis: auto; }   /* replaces both declarations */
    .narrow-only { display: flex; }
}
```

A rule inside a query replaces an earlier declaration of the same CSS property, also when the two compile to different native properties (`width: 350px` and `width: 100%`, `flex-basis: 0` and `flex-basis: auto`).

### `font-family`

The whole list is kept. Each backend walks it in order and uses the first family it has:

```css
.title  { font-family: "Space Grotesk", system-ui, sans-serif; }
.code   { font-family: ui-monospace, Menlo, "Liberation Mono", monospace; }
.brand  { font-family: "asset://fonts/Inter.ttf", system-ui; }
```

| Entry | Font |
| --- | --- |
| `system-ui`, `sans-serif`, `system` (and `BlinkMacSystemFont` on macOS) | The interface font of the platform: the GTK font setting on Linux, San Francisco on macOS, the Windows UI font. |
| `monospace` | The monospace font (Fontconfig's `monospace`, the system monospace font, Consolas). |
| `serif` | The serif font (Fontconfig's `serif`, the serif design of the system font, Times New Roman). |
| `ui-monospace`, `ui-serif`, `ui-sans-serif`, `ui-rounded`, `-apple-system` | Nothing, on every platform, as in Chromium (measured with Electron 37 on Linux and macOS: only Safari knows them); the list goes on. |
| `asset://…` | A font file packaged with the application, loaded on first use: Fontconfig on Linux, CoreText on macOS, DirectWrite (and a private GDI font for text inputs) on Windows. At most 32 files, 8 MiB each and 64 MiB in all. |
| Any other name | An installed family. One that is not installed is skipped; on Linux a Fontconfig alias does not count (`Consolas` is not DejaVu Sans Mono), only Chromium's metric-compatible equivalents (`Arial` → Liberation Sans, `Times New Roman` → Liberation Serif, `Courier New` → Liberation Mono, `Calibri` → Carlito, `Cambria` → Caladea). |

A `@font-face` family in the list resolves to its best face for the weight and style; the other entries stay as fallbacks. Text inputs (`Input`, multiline `Input`, the fields of `Select` and `Autocomplete`) draw with the same resolved list, weight and size as `Text`. On Windows a packaged font is only used as the first entry. When no entry matches, the interface font is used. The `system-ui` keyword inside the `font:` shorthand is not accepted; use `font-family`.

### Line breaking and white space

| CSS | Effect |
| --- | --- |
| `overflow-wrap: anywhere \| break-word` (also `word-wrap`), `word-break: break-word` | Words wrap at word boundaries; a word longer than the line breaks where it must. |
| `word-break: break-all` | Lines may break between any two characters. |
| `word-break: normal \| keep-all` | The default: words wrap at word boundaries and a long word overflows. |
| `hyphens: auto` | A hyphen is drawn where a word is broken. `none` and `manual` draw none (CSS never hyphenates by default). |
| `white-space: nowrap` (also `pre`, `text-wrap: nowrap`, `text-wrap-mode: nowrap`) | One line that is never wrapped; what does not fit is clipped. |
| `text-overflow: ellipsis` | With one line, the end that does not fit becomes an ellipsis. `clip` cuts it. |

```css
.sku   { overflow-wrap: anywhere; }
.chip  { white-space: nowrap; text-overflow: ellipsis; }
```

### Placeholder colour

```css
.search { placeholder-color: #627184; }
/* the same as */
.search::placeholder { color: #627184; }
```

Without one, the placeholder is the text colour of the input at 45% (GTK and AppKit).

### Scrollbars

The scrollbar properties apply to `ScrollView` and `VirtualizedList`; on other elements they are ignored.

```css
.page   { scrollbar-gutter: stable; scrollbar-width: thin; }
.column { scrollbar-gutter: stable; scrollbar-width: 8px; scrollbar-color: rgba(255, 255, 255, .08) transparent; }
.strip  { scrollbar-width: none; }
```

| CSS | Values | Effect |
| --- | --- | --- |
| `scrollbar-gutter` | `auto`, `stable`, `stable both-edges` | `stable` reserves the width of the bar inside a vertical scroller whether or not the content overflows, so content never moves when the bar appears: 15 px, 10 px with `thin`, the length given to `scrollbar-width`, 0 with `none`. `both-edges` reserves it on both sides. |
| `scrollbar-width` | `auto`, `thin`, `none`, a length up to 64 px | The thickness of the bar. `none` hides it; the view still scrolls. |
| `scrollbar-color` | `<thumb> <track>`, `auto` | The colour of the thumb and, on the default-width classic bar, of the track and the corner (transparent when omitted or `transparent`). Thin and px-width bars keep a transparent track. |

With a reserved gutter and a `scrollbar-width` or `scrollbar-color`, the bar is a classic bar: shown in the gutter while the content overflows, at that thickness, with a rounded thumb you can drag. `thin` draws Chromium's thin bar: a 10 px track with the thumb inset 2 px. Without a gutter the platform's own overlay scrollbar stays. Without `scrollbar-color` the thumb is the text colour at 35% on GTK and a neutral grey on AppKit.

### Tables

`display: table`, `table-header-group` / `table-row-group` / `table-footer-group`, `table-row` and `table-cell` lay a container out as a CSS table, so a screen ported from HTML declares the same rules as its `<table>`:

```css
.app-table { display: table; border-collapse: collapse; }
.head      { display: table-row; border-bottom: 1px solid rgba(255,255,255,.06); }
.th        { display: table-cell; padding: 9px 14px; line-height: normal; }
.body      { display: table-row-group; }          /* a VirtualizedList works too */
.tr        { display: table-row; }
.td        { display: table-cell; padding: 9px 14px; border-top: 1px solid rgba(255,255,255,.05);
             border-bottom: 1px solid #1F2A36; vertical-align: middle; }
```

- The children of a table are rows or row groups (any container of rows, a `VirtualizedList` included); the children of a row are its cells, whatever their kind (`Text`, `Column`, `Row`).
- `table-layout: auto` (the default) sizes the columns like browsers: each starts at its widest cell (max-content, or an authored `width`/percentage), spare room goes to the automatic columns in proportion to those widths, a shortfall is taken above each column's min-content width. `table-layout: fixed` takes the widths of the first row's cells and shares the rest.
- A row is as tall as its tallest cell (or its own `height`/`min-height`); `vertical-align: top | middle | bottom` places a cell's content (`middle` by default, like `td`).
- `border-collapse: collapse` follows CSS 2.1 §17.6.2: each grid line is the widest of the borders meeting on it (a cell's over a row's over the table's), every row and cell takes half of it, and the table half of its outer lines. The line is painted once, centred on the edge between the cells and snapped like Chromium (the pixels of edge ± width/2), by the cell whose box holds those pixels. `separate` (the default) keeps each cell's own border, with no spacing.
- A virtualized row group only measures the rows that are mounted.

### Inline formatting (`display: inline`, `inline-block`, `inline-flex`)

`display: inline`, `inline-block` and `inline-flex`/`inline-grid` make a box inline-level, and its parent lays it on line boxes like a browser's block container, so markup ported from HTML keeps its `<span>`, `<small>`, chips and `<textarea>` as they are:

```css
.td     { padding: 10px 14px; font-size: 16px; line-height: 24px; }   /* the cell's strut */
.code   { display: inline; padding: 4px 8px; font-size: 12px; line-height: 16px; background-color: #314158; }
.status { display: inline-flex; padding: 4px 12px; align-items: center; }
.notes  { display: inline-block; width: 100%; height: 130px; }        /* a <textarea> */
```

- A container whose in-flow children are all inline-level holds line boxes; in a vertical container each run of consecutive inline-level children among block children gets an anonymous block (CSS 2.1 §9.2.1.1). In a row with other block children an inline-level child stays a flex item (blockified, like inside a CSS flex container).
- Every line box starts with the container's strut: a zero-width box with the container's own (or inherited) `font-*` and `line-height`, Blink's rounded ascent/descent plus the floored half-leading. Boxes sit on the line's baseline; a line breaks before a box that does not fit, and `text-align` places each line. The lines are placed as one block by the container's `align-items` (row), `justify-content` (column) or a cell's `vertical-align`.
- An inline `Text` takes its `line-height` around its baseline; its box (background, border, padding, hit area) is the font's content area plus padding and border, which overflow the line box like Chromium's. A text that wraps is laid as one box.
- An `inline-block`/`inline-flex` box is atomic: its margin box sits on the baseline at its own baseline: its text's (an inline-block: the last line's), the first item's in an `inline-flex` (or that item's bottom edge), or its bottom margin edge when it has no text or `overflow: hidden`. A multi-line `Input` (a `<textarea>`) always uses its bottom margin edge, so the strut's descent stays under it; a single-line one its centred text.
- `vertical-align`: `baseline`, `middle` (the box centred half an x-height, taken as 0.5em, above the baseline), `text-top`/`text-bottom` (the parent's content area), `top`/`bottom` (the line box). Lengths, percentages, `sub` and `super` are rejected.
- Hosts report the first baseline of a measured text where they draw it (Chromium's rounded ascent plus floored half-leading), so baseline alignment (`align-items: baseline` included) matches Blink.

### Form-control sizes (`<input>`, `<select>`, `min-width: auto`)

A single-line `<Input>` without a `width` is as wide as Chromium's text field: `inputSize` (HTML `size`, 20 by default) characters of the primary font's average width (Blink rounds it up when its fraction is ≥ .5), plus the font's maximum character width minus one average, rounded up, plus padding and border. The first family of the authored `font-family` list decides whether the average is trusted (Blink's list: Helvetica, Times, Courier, Lucida Grande, `.`-prefixed system names…); otherwise the width is `size` zeros. Each host reports the metrics like its Skia port, so the same CSS gives 200px at 16px DejaVu Sans on Linux and 175px at 16px San Francisco on macOS, as Electron does there. An authored `width` (or a percentage) still wins.

A `<select>` is a styled `Pressable` whose label `Text` carries `menuListOptions` (the option labels, one per line or a list): the label is as wide as the widest option in its font, rounded up. Add the menulist's internal padding (4px before the text, 1px + the 15px arrow after it, when the select keeps its native appearance) in the markup around the label.

`min-width: auto` on a flex item is its automatic minimum size: it never shrinks below its min-content width (capped by its `width` and `max-width`), as in a browser. Native boxes default to a 0 minimum, so author it where the browser's automatic minimum matters (e.g. a `flex: 1` search box holding a text field).

### Sticky positioning

`position: sticky` keeps the box in flow and holds it at its `top`/`bottom` (vertical scroll views) or `left`/`right` (horizontal ones) inset from the nearest scroll view's edge while its parent, the containing block, is on screen, as in CSS: a table header row with `position: sticky; top: 0` stays above the rows it covers until the table scrolls away. The box moves with a transform when the scroll offset changes (no layout pass), paints above the content it covers and takes its clicks.

### Scrolling on both axes (`overflow: auto`)

`overflow: auto` (or `scroll`) on a `<ScrollView>` scrolls its content on both axes in the same viewport, like a browser element: each child is as wide as the viewport or as its min-content width (a table whose columns cannot shrink further overflows), and as tall as its content. Single-axis `overflow-x`/`overflow-y: auto` stays a `<ScrollView>` (`horizontal` for the x axis).

- With `scrollbar-gutter: stable`, an authored `scrollbar-width` or a `scrollbar-color`, the bars are Chromium's classic bars: each overflowing axis takes the bar's thickness (15 px by default, `thin` 10 px), which can make the other axis overflow, and the corner between two bars takes the track colour. `scrollbar-color` alone styles the default 15 px bar (9 px thumb, arrow buttons) in that thumb colour on its track colour.
- The wheel moves both axes; `position: sticky` / `stickyHeader` rows stick on the vertical axis (a table header stays while the body scrolls under it).

### Font-relative lengths

`width`, `min-width`, `max-width`, `height`, `min-height` and `max-height` accept `ch` (`max-width: 52ch`): the advance of "0" in the element's own font, measured by the renderer. `line-height: normal` is the font's rounded ascent plus descent, as in Chromium, and overrides a line height another rule gave the element. Line heights land on Blink's LayoutUnits: `line-height: 18.56px` rounds to 1/64 px (18.5625); a number, percentage, `em` or unitless `calc()` (`1.45`, `calc(1.25 / .875)`) multiplies the font size rounded to 1/64 in float and truncates (13.12px × 1.45 = 19.03125). Write the number the web stylesheet authors, not the px that getComputedStyle prints: the two can differ by 1/64 px per line. A `calc()` of only px, rem and numbers is resolved when the stylesheet compiles, so box sizes stay on LayoutUnits too.

Icons (`<Icon>`) keep their own font and size and are one em tall (`line-height: 1`), like icon fonts on the web: only the colour of their container is inherited.

The package ships [Lucide](https://lucide.dev) (ISC licence), so `<Icon name="hash" font="Lucide" size="18" />` works with no asset in the project; names are Lucide's (`message-square`, `chevron-down`, `send-horizontal`). Any other icon font is registered once at startup with `Icon::register('Ionicons', 'asset://assets/fonts/Ionicons.ttf', 'assets/fonts/Ionicons.json')` (a JSON of name => codepoint).

### Compositing layers

`will-change: transform` makes the box a compositing layer, as in Chromium: the box is placed on whole pixels and its content keeps its exact offsets inside it. Dialogs centered with fractional offsets (PrimeVue's `.p-dialog` sets it) then paint their borders and text on the same pixels as the web build. Other `will-change` values are ignored.

### Backdrop blur

```css
.glass { background-color: rgba(15, 23, 42, .6); backdrop-filter: blur(8px); }
```

`backdrop-filter: blur()` (or the `backdropBlurRadius` attribute of a template tag) blurs what is painted behind the box. On a `Modal` it blurs the content behind the backdrop; see [Components](components.md#modal-and-bottom-sheet).

It paints like Chromium: Skia's three-box approximation of the Gaussian, transparent beyond the box and drawn over the unblurred backdrop, then the box's translucent `background-color` blended in 8 bits (`fill + ⌊dst·(256 − α)/256⌋`); stacked masks (a dialog over a dialog) repeat it on the previous result. A gradient background is painted over the blur as before. On GTK, and on macOS without Metal, a `Modal`'s backdrop also composites its dialog box's `box-shadow` and translucent background (and plain translucent fills inside it) the way Chromium layers them, so a dialog over its mask matches Electron to the level; the boxes then paint neither.

On macOS without a Metal device (a virtual machine, some remote sessions) Core Animation cannot run the filter; the AppKit backend then blurs on the CPU: it renders the window without the box and what paints after it, runs the pipeline above on the part under the box (exactly Chromium's pixels) and shows it below the box. It is redone at most ten times a second, only after a view changed and when the pixels under the box differ; content scrolled natively behind a blurred box is picked up on the next change.

### What each backend does

| Feature | GTK (Linux) | AppKit (macOS) | Win32 (Windows) |
| --- | --- | --- | --- |
| `font-family` lists, generic families | Yes | Yes (run in a VM) | Written, not run |
| Packaged fonts (`asset://`) | Yes | Yes (icon fonts, run in a VM) | Written, not run |
| `overflow-wrap`, `word-break` | Yes | No: words wrap at word boundaries | Written, not run |
| `hyphens: auto` | Yes | No | No |
| `white-space: nowrap`, `text-overflow` | Yes | Yes | Written, not run |
| `scrollbar-gutter`, `scrollbar-width`, `scrollbar-color` | Yes | Yes: overlay scrollers, a classic thumb drawn in a reserved gutter | Written, not run: `none` hides the bar, a reserved gutter shows a classic bar |
| `backdrop-filter: blur()` | Yes (GSK Gaussian; the fill blended like Chromium) | Core Image filter with Metal, not verified on screen; without Metal Chromium's exact pipeline on the CPU (run in a VM) | Direct2D blur of the back buffer and fill blend, not run; see [First run](first-run-macos-windows.md) |
| Subpixel text positions | Yes, as Chromium | TextKit default | DirectWrite default |

"Written, not run" means the code is in the backend and type-checks, but it has not been executed on that system. See [Platforms](platforms.md).

## Font families

`font-family` (or `fontFamily`) takes a CSS list. Every backend uses the first family it has, so a stack written for the web works unchanged:

```css
.code { font-family: ui-monospace, "Cascadia Mono", Menlo, Consolas, monospace; }
.body { font-family: "Inter", system-ui, -apple-system, sans-serif; }
```

The CSS generic families name the platform's own fonts:

| Family | Linux (GTK) | Windows | macOS |
| --- | --- | --- | --- |
| `system-ui`, `sans-serif` (`BlinkMacSystemFont` on macOS) | the desktop's interface font (`gtk-font-name`) | the UI font (Segoe UI) | San Francisco |
| `monospace` | Fontconfig `monospace` | Consolas | SF Mono |
| `serif` | Fontconfig `serif` | Times New Roman | New York |

So Tailwind's `font-mono` (`ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", …`) lands on Liberation Mono on Linux and Menlo on macOS, the fonts Chromium picks. A nested `Text` (a span) takes a family list too.

A family declared with `@font-face` in the stylesheet becomes its packaged file wherever it appears in the list; the families after it are the fallback. When no family of the list is installed the text uses the interface font. The DesktopUI components (`SelectTheme`, `Toast`) default to `system-ui`.

## Desktop additions

### Cursor

`cursor()` sets the pointer shape over an element. It is also a `Style` property.

```php
Pressable::make(/* … */)->cursor('pointer');

DrawingCanvas::make('asset://photo.jpg')->cursor('crosshair');
```

The keywords are the CSS ones: `pointer`, `text`, `grab`, `grabbing`, `not-allowed`, `crosshair`, `wait` (or `progress`), `move`, `ew-resize` (or `col-resize`) and `ns-resize` (or `row-resize`). Anything else is the default arrow.

### Tooltip

```php
Pressable::make(/* … */)->tooltip('Export (Ctrl+Shift+E)');
```

The tooltip is the native one of the platform, shown when the pointer rests on the element.

### Hover events

`onHoverIn()` and `onHoverOut()` exist on every element, not only on pressables.

## Dark mode

### Theme tokens in CSS

A `:root` block inside `@media` redeclares the custom properties for the whole stylesheet while the query matches, as in CSS. Declare the tokens once and switch them with the scheme:

```css
:root { --bg: #13141b; --fg: #f2f3f8; }                     /* dark by default */
@media (prefers-color-scheme: light) { :root { --bg: #ffffff; --fg: #15172b; } }

.page { background-color: var(--bg); }
.row:hover { background-color: var(--fg); }
```

Every rule that reads a redeclared property follows it: rules outside the query, their state blocks (`:hover`, `:focus`…) and the rules of other matched queries. The rules keep their place in the cascade. The scheme is the effective one (`Appearance::set()` or the system), and a change restyles the tree. `oklch()`, `lab()` and `color-mix()` values resolve to sRGB like Chromium (out-of-gamut colors are clipped). Text styles declared on `:root` itself (`:root { color: var(--fg); font-family: var(--font-sans) }`) follow it too. CSS `color` also tints an `ActivityIndicator` (`.spinner { color: var(--muted) }`; a `color` attribute still wins), so spinners switch with the scheme. For colors computed in PHP (DesktopUI themes), see [live theme switching](desktop-ui.md#live-theme-switching).

### In PHP


`Pam\Desktop\Appearance` reads and sets the appearance of the application. Reads are synchronous.

```php
use Pam\Desktop\Appearance;
use Pam\Desktop\AppearanceMode;

$dark = Appearance::isDark();

Column::make(/* … */)->style(new Style(
    backgroundColor: $dark ? 0xFF14181F : 0xFFF6F7F9,
));
```

| Call | Meaning |
| --- | --- |
| `Appearance::isDark()` | Whether the effective scheme is dark. |
| `Appearance::current()` | The effective scheme, `UserInterfaceAppearance::Light` or `Dark`. |
| `Appearance::mode()` | The stored preference: `AppearanceMode::System`, `Light` or `Dark`. |
| `Appearance::set($mode)` | Chooses and persists the preference. |
| `Appearance::onChange($listener)` | Runs `$listener(UserInterfaceAppearance, AppearanceMode)` when the scheme or the preference changes. Returns an id for `unsubscribe()`. |

With `AppearanceMode::System` the application follows the operating system, also when the setting changes while it runs. On Linux the setting is the `color-scheme` of the desktop portal, with the GTK settings as the fallback; see [Platforms](platforms.md#other-notes). `set()` stores the choice in the state directory, so the next launch opens with it, and switches the native chrome and the default look of buttons with it.

To follow changes while the application runs, keep the scheme in state:

```php
#[State]
public bool $dark = false;

public function mount(): void
{
    $this->dark = Appearance::isDark();
    Appearance::onChange(function (UserInterfaceAppearance $scheme): void {
        $this->dark = $scheme === UserInterfaceAppearance::Dark;
    });
}
```

`examples/modules` has a button that switches between light and dark.

## Text scale

Text renders at the font size of its style multiplied by a text scale. The scale is the accessibility text scale of the operating system multiplied by a multiplier the application may set for itself.

```php
use Pam\Desktop\System\Accessibility;
use Pam\Desktop\System\TextScale;

Accessibility::textScale(function (TextScale $scale): void {
    // $scale->systemScale, $scale->multiplier, $scale->effectiveScale
});

// An in-app "text size" setting: 1.25 times the system scale.
Accessibility::setTextScale(1.25);
```

A change of the system scale is applied while the application runs; [Accessibility](accessibility.md#text-scale-and-reduced-motion) says where each system reads it.

`setTextScale($multiplier, $maxSystemScale)` accepts a multiplier from 0.5 to 3 and an optional cap on the system scale (0 means no cap). On desktop the new scale is stored and applied at once: the host measures the text again and lays the tree out again.

To keep some text out of the scale, or to bound it:

```php
Text::make('12')->allowFontScaling(false);
Text::make('Heading')->maxFontSizeMultiplier(1.3);
```

`TextScalePolicy::set(maximum: 1.6, display: 1.3, displayFontSize: 18)` sets application-wide caps once, before the first render: body text grows up to `maximum`, text of `displayFontSize` or larger only up to `display`.

## Related

- [Templates](templates.md) for the `.pam.php` syntax these stylesheets belong to.
- [Grid tracks](grid-tracks.md) for `grid-template-columns` with `fr` tracks.
- [Components](components.md) for the elements these styles apply to.
- [Animation and gestures](animation-and-gestures.md) to animate style changes.
- [Accessibility](accessibility.md) for roles, labels and keyboard focus.
