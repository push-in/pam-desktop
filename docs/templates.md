# Templates

A component can be written as one `.pam.php` file: the PHP class with its state and actions, a `<template>` of tags and a `<style>` block. `pam-desktop init` writes one (`src/Screens/Home/HomeScreen.pam.php`). This page is the reference for the template side: expressions, slots, attributes, how templates are compiled and how their errors are reported. The stylesheet side is in [Styling](styling.md#css-in-template-stylesheets).

## A component

```php
<?php

declare(strict_types=1);

namespace App\Screens\Home;

use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;

final class HomeScreen extends Component
{
    public string $title = 'Inbox';

    #[State]
    public int $count = 0;

    public function increment(): void
    {
        $this->count++;
    }
}
?>

<template>
    <Column class="home">
        <Text class="title">{{ $title }}</Text>
        <Button label="Add" on:press="increment" />
        <Text p-if="$count > 0">{{ $count }} items</Text>
    </Column>
</template>

<style scoped>
    .home { flex: 1; padding: 24px; gap: 12px; }
    .title { font-size: 28px; font-weight: 700; }
</style>
```

`App::components($directory, $cacheDirectory)` in `index.php` compiles every `.pam.php` file under `$directory` and registers each class as a tag. The skeleton calls it with `src` and `.pam-desktop/components`.

| Syntax | Meaning |
| --- | --- |
| `{{ $expr }}` | Text from an expression. |
| `name="literal"` | A literal attribute. |
| `:name="$expr"` | An attribute bound to an expression. |
| `on:press="method"` | An event handler: a public method of the component. |
| `p-if="…"`, `p-else-if="…"`, `p-else` | Conditional elements (`v-if` and the other `v-` spellings are accepted too). |
| `p-for="$item in $items"` with `p-key="…"` | A repeated element with a stable key. |
| `class="a b"` | Classes of the scoped stylesheet. |

## Expressions

Expressions are a safe subset of PHP evaluated by the framework, not `eval()`. They read:

- the component's properties (`$title`), loop variables and array indexes (`$order['status']`);
- properties of objects (`$order->status`, or `order.status`);
- the component's public methods (`total($order)`);
- a few PHP helpers: `count`, `in_array`, `trim`, `substr`, `min`, `max`, `array_slice` and others.

### Enum cases and class constants

Fully qualified and qualified names are accepted:

```html
<Text p-if="\App\Orders\Kind::Tab === $kind">Open tab</Text>
<Text>{{ \App\Orders\Kind::Order->value }}</Text>
<Text>{{ \App\Config::DEFAULT_TITLE }}</Text>
```

Enum cases, their `->value`, and constants of classes and interfaces in any namespace resolve.

### Methods of values

A public instance method of an object can be called with `->`:

```html
<Text>{{ $order->status->label() }}</Text>
<Text :title="$item->child()->label() . '?'" />
<Text>{{ $row->label() ?? '-' }}</Text>
```

- Only public, non-static methods whose name does not start with `__` can be called. Private, protected, static and magic methods, closures (`$callback->call()`) and calls on a string are refused when the template compiles or runs.
- Inside `??`, a call on a missing or null value gives the fallback.
- A dot before a call is string concatenation (`$a . trim($b)`), as in PHP.

### `min`, `max` and `array_slice`

```html
<VirtualizedList :maxHeight="min(440, count($rows) * 66)" … />
<Text>{{ max([2, 3.5, -1]) }}</Text>
<Row p-for="$row in array_slice($rows, 0, 5)" p-key="$row->id">…</Row>
```

`min` and `max` take finite integers and floats, as several arguments or one array; `min()` with nothing and mixed types (`max([1, "2"])`) are errors. `array_slice` checks its arguments like PHP.

### Short-circuit

`?:`, `? :`, `??`, `||` and `&&` evaluate only the branch that is taken, as in PHP. A guard such as `$row->id === null ? 'No link' : $names[$row->id]` never reads the missing index.

## Slots

`<Slot />` renders the children the parent passed. `<Slot :content="$expr">` renders a value held by the component instead: a component, an element or anything renderable. When the value is null, the children of the slot are the fallback:

```html
<Slot :content="$chart">
    <Text>No data yet</Text>
</Slot>
```

A component given through `:content` is mounted while the slot shows it and unmounted when the value changes. This is how a component keeps a chart, a [DesktopUI](desktop-ui.md) control or a dialog in a property and places it in its template.

## Attributes

- `width`, `height`, `minWidth`, `minHeight`, `maxWidth` and `maxHeight` take points or a percentage (`width="75.5%"`). A bound value (`:width="$size"`) may switch between the two from one render to the next.
- A bound `:textColor` wins over the `color` of the stylesheet on every render.
- `captureKeys="Space,Enter"` (W3C key names, comma separated, at most 512 bytes): while the element or one of its descendants has the focus, the listed keys go to its `on:keyDown` before the native control handles them. Combinations with Control, Alt or Meta are never captured.
- `dragType`, `dropType`, `on:dragEnter`, `on:dragLeave`: typed drag and drop, see [Desktop shell](desktop-shell.md#typed-drag-and-drop).
- `scrollbarGutter`, `scrollbarWidth`, `scrollbarThickness`, `scrollbarColor` on `ScrollView` and `VirtualizedList`: the same as the CSS properties in [Styling](styling.md#scrollbars).
- `backdropBlurRadius` on any box, and on `Modal`: see [Components](components.md#modal-and-bottom-sheet).
- `inputFormat="decimal"` with `inputFormatDecimalMinDigits`: see [Components](components.md#inputs).

## Compiling

Templates are compiled to PHP classes in the cache directory given to `App::components()`, once per change of the source.

- **One component at a time.** A component that does not compile, cannot be autoloaded or has an invalid contract is reported as a boot error with its file and line; every other component keeps working. Startup fails only when every component failed. `PamPhpCompiler::failures()` lists the failures.
- **Symbolic links** inside the component directory are followed, files and directories. Each real directory is visited once, so a link cycle is safe. This is how a package developed in another checkout (a Composer `path` repository installed as a symlink) is used.
- **`__DIR__` and `__FILE__`** in a `.pam.php` file name the source file, not the compiled class, also in a bundle that was prebuilt and moved. String literals are left alone.
- **Read-only cache.** When the cache directory cannot be written, the compiler uses `PAM_DESKTOP_CACHE_DIR`, else `<temp>/pam-desktop-<uid>/components/<hash>`.
- **Bundles.** `pam-desktop build` prebuilds the templates of the bundle. A failure there is a note in the build output, not a failed build; the templates are then compiled on first launch.

## Errors

In `pam-desktop dev` an error is shown by the [error overlay](development.md#the-error-overlay):

- An error raised inside a template, by a method an expression calls or by a child component, is reported as that error: its type, its message and its line, with the template line as the next frame of the stack.
- An error of the template itself (an expression that does not parse, an unknown component, a refused method) is a **Template error** pointing at the template line and column.
- Stack frames in the compiled cache are mapped back to the `.pam.php` source.

## Hot reload

`#[State]` properties survive a hot reload; see [Getting started](getting-started.md#state-preservation). Mark what must start fresh after a reload, such as a password or an in-flight flag, with `#[State(hotReload: false)]`.

## Related

- [Styling](styling.md) for the `<style>` block.
- [Components](components.md) for the tags.
- [Compatibility](compatibility.md) for attributes that need a recent host.
