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

Components written as single-file templates (`.pam` files, as in `examples/showcase/resources/components/`) carry a `<style>` block. There, the selectors `:pressed`, `:hover` and `:focus` declare styles for a state, and the host applies them with no round trip to PHP:

```css
.language-two-card {
    opacity: 1;
}

.language-two-card:pressed {
    opacity: 0.76;
    transform: scale(0.98);
}
```

The host applies these properties for a state: opacity, scale, translation, background color, text color, border color and elevation. When several states are active, hover is applied first, then focus, then pressed, so pressed wins.

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

With `AppearanceMode::System` the application follows the operating system. `set()` stores the choice in the state directory, so the next launch opens with it, and switches the native chrome and the default look of buttons with it.

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

`setTextScale($multiplier, $maxSystemScale)` accepts a multiplier from 0.5 to 3 and an optional cap on the system scale (0 means no cap). On desktop the new scale is stored and applied at once: the host measures the text again and lays the tree out again.

To keep some text out of the scale, or to bound it:

```php
Text::make('12')->allowFontScaling(false);
Text::make('Heading')->maxFontSizeMultiplier(1.3);
```

`TextScalePolicy::set(maximum: 1.6, display: 1.3, displayFontSize: 18)` sets application-wide caps once, before the first render: body text grows up to `maximum`, text of `displayFontSize` or larger only up to `display`.

## Related

- [Components](components.md) for the elements these styles apply to.
- [Animation and gestures](animation-and-gestures.md) to animate style changes.
- [Accessibility](accessibility.md) for roles, labels and keyboard focus.
