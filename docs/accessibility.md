# Accessibility

The interface of a PAM Desktop application is made of platform views, so it is exposed to assistive technology through each system's own API: AT-SPI on Linux (through GTK), NSAccessibility on macOS, UI Automation on Windows. The application describes roles, names, states and actions once, in PHP; the render core resolves them and each backend maps the result.

**Verification status.** The accessibility tree was not checked with a real screen reader on any system, Linux included. Treat this page as a description of what the code does, not as a statement that a screen reader was observed reading an application. See [Platforms](platforms.md).

## What you get without doing anything

The core derives a role from the kind of element:

| Element | Default role |
| --- | --- |
| `Text` | Static text |
| `Button` | Button |
| `Input` | Text input |
| `Image` | Image |
| `Toggle` | Switch, with its checked state |
| `ActivityIndicator` | Progress bar |
| `Modal`, `BottomSheet`, `UI\Window` | Dialog |
| `VirtualizedList`, `FlatList`, `SectionList` | List |
| Any element with a press handler | Button |
| Anything else | A group without meaning of its own |

Other defaults:

- An element that shows text is named by that text.
- An image without a label is treated as decoration and hidden from assistive technology.
- An element with `enabled(false)` is reported as disabled. A `Button` in its `loading()` state is reported as busy.
- Native text fields describe themselves (role, value, caret, selection). The core only adds the name and the state.

## Describing an element

Every element has these methods:

```php
use Pam\Desktop\AccessibilityRole;

Pressable::make(Image::make('asset://assets/trash.png')->style(new Style(width: 20, height: 20)))
    ->accessibilityLabel('Delete invoice')
    ->accessibilityHint('Moves the invoice to the trash')
    ->onPress($this->delete(...));

Text::make('Invoices')
    ->accessibilityRole(AccessibilityRole::Header);

Image::make('asset://assets/chart.png')
    ->accessibilityLabel('Revenue by month, rising since March')
    ->style(new Style(width: 320, height: 180));
```

| Method | Meaning |
| --- | --- |
| `accessibilityLabel($label)` | The accessible name. Replaces the text of the element as its name. |
| `accessibilityHint($hint)` | A longer description of what the element does. |
| `accessibilityRole(AccessibilityRole)` | The role, when the default is wrong. |
| `accessible(false)` | Hides the element from assistive technology; its children stay exposed. |
| `accessibilityImportance(AccessibilityImportance)` | `No` hides the element; `NoHideDescendants` hides it and everything inside it. |
| `testId($id)` | A stable identifier for UI automation. |

### Roles

`AccessibilityRole` has: `Generic`, `Button`, `Input`, `Image`, `Switch`, `Adjustable`, `Alert`, `Checkbox`, `ComboBox`, `Header`, `ImageButton`, `KeyboardKey`, `Link`, `Menu`, `MenuBar`, `MenuItem`, `ProgressBar`, `Radio`, `RadioGroup`, `ScrollBar`, `Search`, `SpinButton`, `Summary`, `Tab`, `TabList`, `Text`, `Timer`, `ToggleButton`, `Toolbar`, `Grid`, `List`, `ListItem`, and two that hide the element: `None` and `Presentation`.

`ImageButton` and `KeyboardKey` are exposed as buttons, `Adjustable` as a slider and `Header` as a heading.

## States

```php
use Pam\Desktop\AccessibilityCheckedState;
use Pam\Desktop\AccessibilityLiveRegion;

Pressable::make(Text::make('Bold'))
    ->accessibilityRole(AccessibilityRole::ToggleButton)
    ->accessibilityChecked($this->bold ? AccessibilityCheckedState::Checked : AccessibilityCheckedState::Unchecked)
    ->onPress(function (): void {
        $this->bold = !$this->bold;
    });

View::make()
    ->accessibilityRole(AccessibilityRole::Adjustable)
    ->accessibilityLabel('Volume')
    ->accessibilityValue(0, 100, $this->volume, "{$this->volume} percent");

Text::make($this->status)
    ->accessibilityLiveRegion(AccessibilityLiveRegion::Polite);
```

| Method | Meaning |
| --- | --- |
| `accessibilityChecked(AccessibilityCheckedState)` | `Unchecked`, `Checked` or `Mixed`. Switches, checkboxes, radios and toggle buttons report a checked state by default. |
| `accessibilitySelected(bool)` | Selected, for tabs, segmented controls and list items. |
| `accessibilityExpanded(bool)` | Expanded or collapsed. |
| `accessibilityBusy(bool)` | The element is working. |
| `accessibilityValue($minimum, $maximum, $current, $text)` | A range and its current value; `$text` is how the value reads. The call throws when `$current` is outside the range. |
| `accessibilityLiveRegion(AccessibilityLiveRegion)` | `Polite` or `Assertive`: the new name of the element is announced when it changes. |

A live region is the way to announce something on desktop. `Accessibility::announce()` exists for code shared with PAM Native, but the desktop host accepts it without delivering it, and `Accessibility::screenReaderEnabled()` always reports `false`.

## Actions

The default action of a pressable element is its press. Assistive technology that runs it presses the element exactly as a click would: `onPress` runs.

Custom actions give an element more verbs than a press:

```php
use Pam\Desktop\AccessibilityAction;

Row::make(Text::make($message))
    ->accessibilityActions(
        new AccessibilityAction('archive', 'Archive'),
        new AccessibilityAction('delete', 'Delete'),
    )
    ->onAccessibilityAction(function (string $name) use ($index): void {
        match ($name) {
            'archive' => $this->archive($index),
            'delete' => $this->remove($index),
            default => null,
        };
    });
```

An element takes one to eight actions. A name is a lower-case identifier of up to 64 bytes and a label up to 128 bytes; names are unique within an element. Actions are only exposed when the element also has `onAccessibilityAction`.

## What each backend maps

| | Linux (GTK 4, AT-SPI) | macOS (AppKit) | Windows (UI Automation) |
| --- | --- | --- | --- |
| Roles | GTK accessible roles (`Button`, `TextBox`, `Img`, `Switch`, `Slider`, `Heading`, `List`, `Dialog`…) | `NSAccessibility` roles and subroles (a switch is a check box with the switch subrole, a tab is a radio button with the tab subrole) | Control types (`Button`, `Edit`, `Image`, `CheckBox`, `Slider`, `Text`, `List`, `Window`…) |
| Name and hint | Label and description properties | Label and help | Name and help text |
| Range values | Minimum, maximum, current and text value | Value | Range value pattern |
| Checked, selected, expanded, disabled, busy | Accessible states | Value and state attributes | Toggle, selection item and expand/collapse patterns |
| Live regions | `announce()` on the widget, medium or high priority | An announcement notification, medium or high priority | Live setting |
| Default press from assistive technology | Not reported by the backend | `accessibilityPerformPress` | Invoke pattern |
| Custom actions | Not exposed | `NSAccessibilityCustomAction` | Not exposed |
| Native text fields and web views | Described by the toolkit | Described by AppKit and WebKit | Their own providers |

On Linux, GTK fixes the role of a widget once its accessible object is in use, so a role that changes after the element was shown is not picked up.

The Linux column is implemented and runs; the macOS and Windows columns are written and type-checked but have not run on a real machine.

## Text selection

`Text` is not selectable by default. `selectable()` turns it on for one element:

```php
Text::make($this->paragraph)
    ->selectable()
    ->style(new Style(fontSize: 16, lineHeight: 25));
```

| Input | Result |
| --- | --- |
| Drag | Selects from the press to the pointer. |
| Double click | Selects the word. |
| Triple click | Selects everything. |
| Shift + click | Extends the selection. |
| Ctrl+C (Cmd+C on macOS) | Copies the selection to the clipboard. |
| Ctrl+A (Cmd+A on macOS) | Selects all the text of the element, while it holds the selection. |
| Escape | Clears the selection. |

The selection is painted with the selection colors of the platform. Text inside a pressable is not selectable: a press wins over a selection. Text inputs have their own native selection.

`examples/text` shows selectable and non-selectable paragraphs.

## Keyboard navigation

- **Tab** and **Shift+Tab** move focus between focusable elements. Every enabled pressable element (buttons, pressables, anything with a press handler) and every text input is focusable.
- **Enter** and **Space** activate the focused pressable.
- **Escape** closes the topmost modal or bottom sheet (`onRequestClose` runs).
- **Alt+Left** (Linux and Windows) and **Cmd+[** (macOS), and the back button of a mouse, go back in a navigator.
- **F5** and **Ctrl+R** (Cmd+R) ask a visible scroll view or list with `onRefresh` to refresh.
- Keys that no control consumed are delivered to `onKeyDown` and `onKeyUp` of the focused element, and to [application shortcuts](desktop-shell.md#shortcuts).
- The tab bar of a `TabNavigator` is made of ordinary nodes of the scene: its tabs take focus and are activated like the rest of the interface.

A focused element can be styled with the `:focus` selector of a template stylesheet; see [Styling](styling.md#state-selectors-in-template-stylesheets).

Scroll views, text fields and web views keep the native keyboard behaviour of their platform.

## Text scale and reduced motion

The host reads the text scale and the reduced-motion setting of the system. Text follows the scale ([Styling](styling.md#text-scale)); with reduced motion, feedback and settle animations land at their end state at once ([Animation and gestures](animation-and-gestures.md#reduced-motion)).
