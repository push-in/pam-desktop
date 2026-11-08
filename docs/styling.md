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

`:focus-within` matches a box while it or one of its descendants has the keyboard focus, as in CSS. It styles the frame around an input without making the frame focusable:

```css
.search { border: 1px solid #CBD5E1; border-radius: 8px; }
.search:focus-within { border-color: #2563EB; }
```

## CSS in template stylesheets

The `<style>` block of a `.pam.php` component, and the stylesheets it imports, are compiled to the same native properties as `Style`. The selectors are classes, tags and the state pseudo-classes above; `@media` and `@container` rules apply while their condition holds. A property that has no meaning for native views (`outline`, `will-change`, `appearance`, `-webkit-font-smoothing` and similar) is accepted and ignored; anything else that does not compile is an error that names the file and the line.

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
| `system-ui`, `-apple-system`, `BlinkMacSystemFont`, `ui-sans-serif`, `sans-serif`, `system` | The interface font of the platform: the GTK font setting on Linux, San Francisco on macOS, the Windows UI font. |
| `monospace`, `ui-monospace` | The monospace font (Fontconfig's `monospace`, the system monospace font, Consolas). |
| `serif`, `ui-serif` | The serif font (Fontconfig's `serif`, the serif design of the system font, Times New Roman). |
| `ui-rounded` | The rounded design of the system font on macOS; the interface font elsewhere. |
| `asset://…` | A font file packaged with the application, loaded on first use: Fontconfig on Linux, CoreText on macOS, DirectWrite (and a private GDI font for text inputs) on Windows. At most 32 files, 8 MiB each and 64 MiB in all. |
| Any other name | An installed family. One that is not installed is skipped. |

A `@font-face` family in the list resolves to its best face for the weight and style; the other entries stay as fallbacks. On Windows a packaged font is only used as the first entry. When no entry matches, the interface font is used. The `system-ui` keyword inside the `font:` shorthand is not accepted; use `font-family`.

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
| `scrollbar-color` | `<thumb> <track>`, `auto` | The colour of the thumb. The track is always transparent. |

With a reserved gutter and a `scrollbar-width` or `scrollbar-color`, the bar is a classic bar: shown in the gutter while the content overflows, at that thickness, with a rounded thumb you can drag. `thin` draws Chromium's thin bar: a 10 px track with the thumb inset 2 px. Without a gutter the platform's own overlay scrollbar stays. Without `scrollbar-color` the thumb is the text colour at 35% on GTK and a neutral grey on AppKit.

### Backdrop blur

```css
.glass { background-color: rgba(15, 23, 42, .6); backdrop-filter: blur(8px); }
```

`backdrop-filter: blur()` (or the `backdropBlurRadius` attribute of a template tag) blurs what is painted behind the box. On a `Modal` it blurs the content behind the backdrop; see [Components](components.md#modal-and-bottom-sheet).

### What each backend does

| Feature | GTK (Linux) | AppKit (macOS) | Win32 (Windows) |
| --- | --- | --- | --- |
| `font-family` lists, generic families | Yes | Yes (run in a VM) | Written, not run |
| Packaged fonts (`asset://`) | Yes | Yes (icon fonts, run in a VM) | Written, not run |
| `overflow-wrap`, `word-break` | Yes | No: words wrap at word boundaries | Written, not run |
| `hyphens: auto` | Yes | No | No |
| `white-space: nowrap`, `text-overflow` | Yes | Yes | Written, not run |
| `scrollbar-gutter`, `scrollbar-width`, `scrollbar-color` | Yes | Yes: overlay scrollers, a classic thumb drawn in a reserved gutter | Written, not run: `none` hides the bar, a reserved gutter shows a classic bar |
| `backdrop-filter: blur()` | Yes (GSK blur) | Core Image filter, not verified on screen | Direct2D blur of the back buffer, not run; see [First run](first-run-macos-windows.md) |
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
| `system-ui`, `-apple-system`, `BlinkMacSystemFont`, `ui-sans-serif`, `sans-serif` | the desktop's interface font (`gtk-font-name`) | the UI font (Segoe UI) | San Francisco |
| `ui-monospace`, `monospace` | Fontconfig `monospace` | Consolas | SF Mono |
| `ui-serif`, `serif` | Fontconfig `serif` | Times New Roman | New York |

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
