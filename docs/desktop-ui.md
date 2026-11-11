# DesktopUI

`pushinbr/pam-desktop-ui` (namespace `Pam\DesktopUi`) is a set of ready controls written as PAM Desktop templates: selects, autocomplete, a date picker, toasts and a few smaller pieces. They are ordinary components made of native views, so they follow the backend they run on and need no host feature of their own.

It ships inside `pushinbr/pam-desktop`: nothing else to require.

```php
// index.php, before App::run()
use Pam\DesktopUi\DesktopUI;

DesktopUI::boot(__DIR__.'/.pam-desktop/desktop-ui');
```

`DesktopUI::boot(?string $cachePath)` registers the components as template tags. Controls that you configure in PHP (a `Select` with its options, a `Toast` you call) are created in the component that owns them and placed in its template with [`<Slot :content>`](templates.md#slots).

## `SelectProvider`

Popups (the list of a `Select`, the suggestions of an `Autocomplete` or a `MultiSelect`) and toasts are drawn in a portal above the screen. `SelectProvider` is that portal: wrap the screen, the form or the modal that uses them, and give the controls the same `host` name.

```php
final class SettingsScreen extends Component
{
    public string $host;
    public Select $theme;

    public function __construct()
    {
        $this->host = 'settings-'.spl_object_id($this);
        $this->theme = new Select(
            options: [new SelectOption('light', 'Light'), new SelectOption('dark', 'Dark')],
            value: 'light',
            changed: fn (int|string|null $value) => $this->apply($value),
            host: $this->host,
            placeholder: 'Choose a theme',
            label: 'Theme',
        );
    }
}
?>
<template>
    <SelectProvider :host="$host">
        <Column class="form">
            <Slot :content="$theme" />
        </Column>
    </SelectProvider>
</template>
```

A unique host per screen (`spl_object_id`) keeps two open screens from sharing a portal. Inside a `Modal`, put the provider inside the modal and outside a `DialogViewport`, so popups are not clipped by its scroll view.

## Select

A closed list of options, with an optional filter field and a clear button.

| Argument | Default | Meaning |
| --- | --- | --- |
| `options` | | `list<SelectOption(value, label, disabled = false)>`; values are `int` or `string`. |
| `value`, `changed` | | The selected value, controlled by the owner; `changed(int\|string\|null)`. |
| `placeholder`, `label` | "Select" | Text shown without a value; accessibility label. |
| `showClear` | `false` | A clear button when a value is selected. |
| `filter`, `filterPlaceholder`, `emptyMessage` | `false` | A filter field at the top of the popup. |
| `theme`, `fieldStyle` | | `SelectTheme` (or a `Closure` returning one, see [live theme switching](#live-theme-switching)), `AutocompleteInputStyle`. |

`setOptions()` replaces the options. Arrow keys, Enter, Escape and typing to filter work while the popup is open.

The default strings of every control ("Select", "Search", "No options found.", the calendar, the accessibility labels) come from the framework catalog in the app language: English, Portuguese and Spanish are included. See [Internationalization](#internationalization). A value you pass (`placeholder`, `label`, `filterPlaceholder`, `emptyMessage`) always wins.

## Autocomplete

A text field with asynchronous suggestions. See [Autocomplete](../packages/desktop-ui/docs/autocomplete.md) for the provider contract, debouncing and pages.

**Select mode.** With `filterInPopup: true`, a `selectedValue`, a `cleared` callback and a `pageProvider`, the control behaves like a searchable select: the field shows the selected label, the filter is typed inside the popup, and results come in bounded pages.

```php
$this->category = new Autocomplete(
    value: $this->categoryLabel,
    changed: fn (string $text) => $this->categoryLabel = $text,
    selected: fn (int|string $id) => $this->categoryId = $id,
    suggest: static fn () => null,
    host: $this->host,
    minimumLength: 0,
    showImages: false,
    pageProvider: $this->categoryPage(...),   // (query, page, complete, failed): ?Closure
    filterInPopup: true,
    selectedValue: $this->categoryId,
    cleared: fn () => $this->categoryId = null,
    filterPlaceholder: 'Search categories',
);
```

`limit` is 1 to 20 (8 by default) and `debounceMs` 0 to 2000 (250).

## MultiSelect

Several values from paged, searchable results; the popup stays open while choosing. See [MultiSelect](../packages/desktop-ui/docs/multi-select.md).

## DatePicker

A calendar for a date, a date and time, or a range, opened in a modal dialog of its own, so it needs no `SelectProvider`.

```php
$this->period = new DatePicker(
    value: new DateSelection(new DateTimeImmutable('2026-11-01'), new DateTimeImmutable('2026-11-07')),
    changed: fn (?DateSelection $selection) => $this->load($selection),
    mode: DatePickerMode::Range,
    locale: DatePickerLocale::system(),
    presentation: DatePickerPresentation::Field,
    rangeSeparator: ' - ',
    showFieldIcon: false,
);
```

| Argument | Default | Meaning |
| --- | --- | --- |
| `mode` | `Date` | `DatePickerMode::Date`, `DateTime` or `Range`. |
| `presentation` | `Button` | `Button` (a button with the label) or `Field` (an input-like field). |
| `locale` | `DatePickerLocale::system()` | `DatePickerLocale`: month and weekday names, button labels, `dateFormat` and `datePlaceholder`. `english()`, `portuguese()`, `spanish()` and `forLocale('pt-BR')` are built in. `system()` follows the app language (`I18n::use()`), else `LC_ALL`, `LC_TIME`, then `LANG` (skipping empty, `C` and `POSIX`), else the framework locale; a language without a catalog gets English. |
| `displayFormat` | the locale's | A PHP date format for the label. |
| `rangeSeparator` | `' — '` | Between the two dates of a range (`01/11/2026 - 07/11/2026`). |
| `showFieldIcon` | `true` | The calendar icon of the `Field` presentation. |
| `minimumYear`, `maximumYear` | 1900, 2100 | Bounds of the year selector. |

`DateSelection(start, ?end)` keeps the time zone of the values it was given.

## Toast

A stack of messages with severities, auto-dismiss and a close button. There are two ways to use it.

### The window toast service

`Toast::service()` (or `ToastService::instance()`) is one stack for the window, like PrimeVue's `ToastService`: it is drawn above everything, open modals and sheets included, and its messages stay when the app navigates to another screen or route. It needs no `SelectProvider` and nothing in the template.

```php
use Pam\DesktopUi\Toast;

Toast::service()->success('Saved', 'The product was updated.');
Toast::service()->error('Upload failed', $error->getMessage(), life: 4500);
$id = Toast::service()->info('Syncing…', life: null);
Toast::service()->remove($id);
Toast::service()->clear();
```

- `add()`, `success`, `info`, `warn`, `error`, `remove` and `clear` take the same arguments as on a `<Toast>`; `messages` lists what is shown.
- `configure(position, limit, width, offset, fontFamily, backdropBlur, theme)` sets the geometry and look; call it once at boot. `ToastTheme` restyles the PrimeVue look for a product design system (surface, border, a severity edge on the left with `edgeWidth`/`edges`, icon colours per severity in `accents`, summary/detail colours and type, padding, radius, shadow, icon sizes, close offset); every token left out keeps the Aura look. A `<Toast>` takes the same `theme:` argument.
- A dialog that reports an error shows it in the service: the message is drawn above the dialog and stays after the dialog closes.
- Nothing is mounted while the stack is empty. With messages, the runtime mounts it next to the application root in a window layer: a modal with `ModalPresentation::Overlay`, stacked above every other modal, with no backdrop, that Escape never closes and that lets presses outside its messages through. `Pam\Desktop\UI\WindowLayer::set($key, $renderable)` gives other content the same layer.
- It belongs to the main window.

### A stack bound to one screen

A `Toast` component draws in a `SelectProvider` of the screen and goes away with the screen.

```php
$this->toast = new Toast($this->host, position: ToastPosition::TopRight);

$this->toast->success('Saved', 'The document was saved.');
$this->toast->error('Upload failed', $error->getMessage(), life: 6000);
$id = $this->toast->info('Syncing…', life: null);   // stays until closed
$this->toast->remove($id);
```

```html
<SelectProvider :host="$host">
    …
    <Slot :content="$toast" />
</SelectProvider>
```

| Argument | Default | Range |
| --- | --- | --- |
| `host` | `'desktop-ui'` | The `SelectProvider` it draws in. |
| `position` | `TopRight` | `ToastPosition`: `TopRight`, `TopLeft`, `TopCenter`, `BottomRight`, `BottomLeft`, `BottomCenter`, `Center`. |
| `limit` | 5 | 1 to 20; the oldest message leaves when it is exceeded. |
| `width`, `offset` | 400, 20 | 160 to 800; 0 to 200. |
| `fontFamily` | `'system-ui'` | A CSS family list. |
| `backdropBlur` | 1.5 | 0 to 50 px; blurs what is behind each message (CSS `backdrop-filter: blur()`); 0 turns it off. |

`add(ToastSeverity, summary, detail, life, closable)` returns an id; `success`, `info`, `warn` and `error` are shortcuts. `life` is 500 to 120 000 ms, or `null` for a message that stays. Severities: `Success`, `Info`, `Warn`, `Error`, `Secondary`, `Contrast`. Timers stop while the screen is paused and resume with the time that was left. Messages are not kept across a hot reload.

The blur is drawn on GTK; on macOS and Windows see [Platforms](#platforms).

## Checkbox

```php
$this->active = new Checkbox($this->enabled, fn (bool $checked) => $this->enabled = $checked, label: 'Enabled');
```

`size` is 24 by default; `theme` takes a `SelectTheme` or a `Closure` returning one. Space and Enter toggle it while focused. `boxSize` paints a smaller box at the top-left of the `size` slot (PrimeVue `.p-checkbox` vs `.p-checkbox-box`, e.g. `size: 20, boxSize: 18`) and `radius` sets its corners (5 by default).

## Slider

```php
$this->volume = new Slider($this->level, fn (float $value) => $this->level = $value, label: 'Volume');
$this->seek = new Slider(
    $this->position,
    fn (float $value) => $this->position = $value,
    max: $this->duration,
    committed: fn (float $value) => $this->player->seek($value),
    label: 'Position',
);
```

A click or a drag sets the value (a drag keeps following the pointer past either end); the arrow keys step it by `step`, or a twentieth of the range when `step` is 0, Page Up and Page Down by five steps, Home and End to the ends. `$changed` reports every new value; `$committed`, when given, runs once the pointer is released or a key changed the value, for work too costly to repeat while dragging. `trackHeight`, `thumbSize`, `trackColor`, `fillColor`, `activeColor` (the fill while hovered or dragged) and `thumbColor` (the thumb shows while hovered or dragged) style it; the defaults are a media player's. It is exposed to screen readers as a slider with its range, its value and `valueText`. `min`, `max` and `value` are state: change them on the same instance (the length of the next track) rather than creating another slider.

## ProgressButton

A button that keeps its label and shows a spinner while `loading`; it ignores presses while loading or disabled. See [ProgressButton](../packages/desktop-ui/docs/progress-button.md).

```html
<ProgressButton label="Sign in" :loading="$busy" :enabled="canSubmit()" backgroundColor="#2563EB" on:press="submit" />
```

## MediaTile and DialogViewport

- [`MediaTile`](../packages/desktop-ui/docs/media-tile.md) `(source, label, primary, enabled)`: a square thumbnail whose actions show on hover or keyboard focus; emits `primary` and `remove`.
- [`DialogViewport`](../packages/desktop-ui/docs/dialog-viewport.md) `(inset = 16)`: a scroll view that centres a dialog and keeps `inset` points free around it (0 to 128), so a tall dialog scrolls instead of overflowing a small window.

## SelectTheme

One theme object styles `Select`, `Autocomplete`, `MultiSelect`, `DatePicker` and `Checkbox`. Colours are `0xAARRGGBB` integers.

```php
$theme = new SelectTheme(
    surface: 0xFF0F172A, text: 0xFFF8FAFC, muted: 0xFF94A3B8, border: 0xFF334155,
    highlight: 0x1F6366F1, selectedSurface: 0x1F6366F1, selectedText: 0xFFA5B4FC,
    popupMinWidth: 288, popupOffset: 2,
);
```

| Group | Arguments |
| --- | --- |
| Colours | `surface`, `text`, `muted`, `border`, `highlight`, and the optional `inputSurface`, `popupSurface`, `filterSurface`, `optionText`, `popupBorder`, `filterBorder`, `selectedSurface`, `selectedText`, `clearIconColor`, `triggerIconColor` |
| Type | `optionFontSize` (8 to 32), `optionFontWeight`, `fontFamily` (`'system-ui'`), `iconFont` |
| Focus | `focusBorder`, `focusRing`, `focusRingWidth` (0 to 8, default 2): border and `box-shadow: 0 0 0 <width>` ring of an open select field and of a focused `DatePicker` field (the picker gives the focus back to its field when it applies). Null keeps the built-in focus style |
| Placeholder | `placeholderColor` (null: the field text at 45%, the hosts' dimmed default) |
| Rows | `optionHeight` (16 to 96, null = 36): option height; select mode adds its 2px gap. PrimeVue: 2 × 8 padding + font size × 1.5 |
| Popup | `popupElevation`, `popupMinWidth` (up to 1024), `popupOffset` (up to 32), `popupShadowColor`, `popupShadowOffsetY` (±64), `popupShadowBlur` (up to 64) |

### Live theme switching

Every control that takes a theme (`Select`, `Autocomplete`, `MultiSelect`, `DatePicker`, `Checkbox`, `Toast` and `Toast::service()->configure()`) also takes a `Closure` that returns one. The Closure is called on every render, and a light/dark switch (`Appearance::set()` or the system) renders every component again, so a control built once follows the switch at once, popups and toasts already on screen included:

```php
// Two fixed themes, picked by the effective scheme.
$theme = SelectTheme::adaptive(light: $lightTheme, dark: $darkTheme);
// Or build it from the product's tokens on each render.
$theme = static fn (): SelectTheme => new SelectTheme(surface: Tokens::argb('surface'), text: Tokens::argb('fg'));

$this->status = new Select($options, null, $this->changed(...), theme: $theme);
Toast::service()->configure(theme: ToastTheme::adaptive(light: $lightToasts, dark: $darkToasts));
```

`SelectTheme::resolve()` and `ToastTheme::resolve()` return the theme a value or Closure stands for; the component's public `$theme` is the one drawn by the current render. Keep the Closure cheap (it runs on every render of the control). Colours that only CSS sets (classes on your own views, `ActivityIndicator` `color`) follow the scheme through custom properties without PHP; see [theme tokens in CSS](styling.md#theme-tokens-in-css).

`AutocompleteInputStyle(height, fontSize, fontWeight, paddingHorizontal, radius)` sets the geometry of the field of `Autocomplete`, `Select` (`fieldStyle`) and `DatePicker`.

## Internationalization

The default copy of the controls follows the app language. English is the base; Portuguese (`pt`) and Spanish (`es`) are included. Set the language once at boot, or let it follow the system:

```php
\Pam\Desktop\I18n\I18n::use('pt-BR'); // the app's language, on any system locale
```

| Level | How |
| --- | --- |
| System | `LC_ALL`, `LC_MESSAGES`, then `LANG`; else the locale the host reported; else English. |
| App | `I18n::use('es')`. It also drives `useLocale()`, `DatePickerLocale::system()` and the host chrome (share menu, release error screen). |
| App, one text | Register the key under the `pam` namespace of an app catalog: `I18n::catalog('pt', ['pam' => ['select.placeholder' => 'Escolher']])`. |
| Component | The arguments (`placeholder:`, `label:`, `emptyMessage:`, `locale:`) win over everything. |

The keys and the three catalogs are in `Pam\Desktop\I18n\FrameworkText`; [Internationalization](i18n.md) lists what else in the framework is localized the same way.

## Platforms

The controls are templates, so they run wherever the backend runs. What depends on the backend:

| | Linux | macOS | Windows |
| --- | --- | --- | --- |
| Controls, popups, keyboard | Verified | The category select popup verified in a VM | Not run |
| Toast backdrop blur | Verified | Not verified on screen | Not run |
| Window toast layer above modals | Verified in tests | Not verified on screen | Not run |

## Tests

```bash
for test in packages/desktop-ui/tests/*.php; do php "$test"; done
```

## Related

- [Templates](templates.md) for `<Slot :content>`.
- [Internationalization](i18n.md) for the framework copy.
- [Charts](charts.md) for the chart package.
