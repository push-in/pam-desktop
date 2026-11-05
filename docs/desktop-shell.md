# Desktop shell

The shell is what makes a window an application of the desktop: window control, more windows, the menu bar, context menus, the tray, shortcuts, file dialogs, notifications, drag and drop and the clipboard. All of it is native to each platform and driven from PHP.

`examples/shell` exercises most of this page in one application; `examples/windows` and `examples/dragdrop` cover secondary windows and drag and drop.

The classes are in `Pam\Desktop` unless a sample says otherwise. Calls are usually made in `mount()`.

## The main window

`Pam\Desktop\Window` is a facade for the main window. Its initial title and size come from [`pam-desktop.json`](getting-started.md#pam-desktopjson).

```php
use Pam\Desktop\Window;
use Pam\Desktop\WindowState;

Window::title('Invoices — March');
Window::minimumSize(640, 480);

Window::onStateChange(function (WindowState $state): void {
    $this->maximized = $state->maximized;
});

Window::onCloseRequested(function (): void {
    if ($this->unsaved) {
        $this->confirmClose = true;
        return;
    }
    Window::close();
});
```

| Method | Effect |
| --- | --- |
| `title(string)` | Sets the title. |
| `resize(width, height)` | Sets the size of the content area. |
| `minimumSize(width, height)` | Smallest size the user can give the window; `minimumSize(null)` removes the limit. |
| `resizable(bool)` | Whether the user can resize it. |
| `minimize()`, `maximize()`, `restore()` | `restore()` leaves the maximized, minimized or full screen state. |
| `fullScreen(bool)` | Enters or leaves full screen. |
| `focus()` | Brings the window to the front and gives it keyboard focus. |
| `show()`, `hide()` | `hide()` keeps the application running; pair it with a tray icon. |
| `center()`, `alwaysOnTop(bool)`, `progress(?float)` | Placement, stacking and progress on the taskbar or dock icon. See the note below. |
| `close()` | Closes the window and ends the application, without asking. |
| `onCloseRequested(?Closure)` | The application decides what the close button does. Call `Window::close()` from the handler to really close; pass `null` to restore the default. |
| `onStateChange(Closure)` | Runs with a `WindowState` when the window is maximized, minimized, focused, resized or enters full screen. Returns an id for `offStateChange()`. |
| `state(Closure)` | Reads the current `WindowState` once. |
| `onFilesDrop(Closure)` | Files dropped on the window outside any element that handles them. Returns an id for `offFilesDrop()`. |

`WindowState` has `maximized`, `minimized`, `fullScreen`, `focused`, `width` and `height`.

On Linux, `center()` and `alwaysOnTop()` do nothing: Wayland gives applications no say on placement or stacking, and GTK 4 exposes neither.

`progress()` takes a fraction from 0 to 1, or `null` to remove the indicator:

| System | Where it shows |
| --- | --- |
| Linux | On the launcher icon, in docks that listen to the Unity launcher entry API (Ubuntu Dock, Dash to Dock, the Plasma task manager), for an installed application. See [Platforms](platforms.md#wayland-and-x11). |
| macOS | As a badge on the Dock icon ("42%"). |
| Windows | As the progress of the taskbar button. |

### Application state

The state of the main window is the state of the application:

| `AppState` | When |
| --- | --- |
| `Active` | The main window has the focus. |
| `Inactive` | The main window is visible without the focus. `inactive()` runs on mounted components, and `activated()` when the focus returns. |
| `Background` | The main window is minimized or hidden. `paused()` runs on mounted components, and `resumed()` when the window comes back. |

In the background the timers a component made with `$this->timers()` follow their `TimerBackgroundPolicy`: `Suspend` (the default), `Cancel` or `Continue`. `App::onStateChange($listener)` runs `$listener(AppState)` on every transition and returns an id for `App::offStateChange()`.

`App::quit()` ends the application.

## Secondary windows

A secondary window is an element. `Pam\Desktop\UI\Window` opens a native window while it is in the tree and closes it when it leaves. Its children are the content of that window, laid out against that window's size, and they belong to the component that renders them, so they share its state.

```php
use Pam\Desktop\UI\Window;

Column::make(
    Button::make('Inspector')->onPress(function (): void {
        $this->inspector = !$this->inspector;
    }),
    $this->inspector
        ? Window::make(
            Column::make(
                Text::make("The counter is {$this->count}."),
                Button::make('Add one')->onPress(function (): void {
                    $this->count++;
                }),
            )->style(new Style(flexGrow: 1, padding: 24, gap: 12)),
        )
            ->title('Inspector')
            ->size(360, 260)
            ->onClose(function (): void {
                $this->inspector = false;
            })
        : null,
);
```

| Method | Effect |
| --- | --- |
| `title(string)` | Title of the window. |
| `size(width, height)` | Size of the content area when it opens. |
| `minimumSize(width, height)` | Smallest size the user can give it. |
| `resizable(bool)` | Whether the user can resize it. |
| `modal()` | Blocks the main window while this one is open, like a dialog. |
| `open(false)` | Hides the window without unmounting its content. |
| `onClose(Closure)` | The user asked to close the window. |

The window stays open until the application stops rendering it. `onClose` only reports the request, so the handler must clear the state that renders the window.

The element and the main window facade share the short name `Window`. A file that needs both imports one under an alias, for example `use Pam\Desktop\UI\Window as WindowElement;`.

## Menu bar

`App::menu()` sets the application menu. On macOS it is the system menu bar; on Windows and Linux it sits at the top of the main window.

```php
use Pam\Desktop\App;
use Pam\Desktop\Menu;
use Pam\Desktop\MenuItem;
use Pam\Desktop\MenuRole;

App::menu(
    Menu::make(
        'File',
        MenuItem::make('New document', $this->create(...), 'CmdOrCtrl+N'),
        MenuItem::make('Open…', $this->open(...), 'CmdOrCtrl+O'),
        MenuItem::separator(),
        MenuItem::role(MenuRole::Quit),
    ),
    Menu::edit(),
    Menu::make(
        'View',
        MenuItem::checkbox('Sidebar', $this->sidebar, function (bool $visible): void {
            $this->sidebar = $visible;
            $this->installMenu();
        }, 'CmdOrCtrl+B'),
        MenuItem::role(MenuRole::ToggleFullScreen),
    ),
);
```

Menus and items are immutable values. To change the menu (a check mark, a disabled item), call `App::menu()` again with the new menus. Calling it with no arguments removes the menu.

| Item | Built with |
| --- | --- |
| Action | `MenuItem::make($label, $handler, $shortcut, enabled: true, destructive: false)` |
| Check item | `MenuItem::checkbox($label, $checked, $handler, $shortcut)`. The handler receives the new state. |
| Platform item | `MenuItem::role(MenuRole::Copy)` |
| Separator | `MenuItem::separator()` |
| Submenu | `MenuItem::submenu($label, ...$items)` |

`Menu::edit()` is the standard Edit menu (undo, redo, cut, copy, paste, select all). `Menu::window()` is the standard Window menu (minimize, zoom, full screen).

### Roles

A role is an item the operating system implements. It gets the conventional label and shortcut, both of which you can override: `MenuItem::role(MenuRole::Quit, 'Exit')`.

| Role | Default shortcut |
| --- | --- |
| `Quit` | `CmdOrCtrl+Q` |
| `About` | none |
| `Preferences` | `CmdOrCtrl+,` |
| `Undo`, `Redo` | `CmdOrCtrl+Z`; `Cmd+Shift+Z` on macOS, `Ctrl+Y` elsewhere |
| `Cut`, `Copy`, `Paste`, `SelectAll` | `CmdOrCtrl+X`, `C`, `V`, `A` |
| `Minimize` | `Cmd+M` on macOS |
| `Zoom` | none |
| `ToggleFullScreen` | `Ctrl+Cmd+F` on macOS, `F11` elsewhere |
| `CloseWindow` | `CmdOrCtrl+W` |
| `Hide`, `HideOthers`, `ShowAll` | `Cmd+H` for `Hide` on macOS |

## Context menus

`contextMenu()` exists on every element. The menu opens on a secondary click (the right mouse button) over the element. From the keyboard, the menu key and Shift+F10 open the menu of the focused element, or of its nearest ancestor that has one, in the middle of that element. Each item runs its own handler.

```php
Pressable::make(Text::make($name))
    ->contextMenu(
        MenuItem::make('Open', function () use ($index): void {
            $this->selected = $index;
        }),
        MenuItem::separator(),
        MenuItem::make('Delete', fn () => $this->remove($index), destructive: true),
    );
```

Items may be check items and submenus, as in the menu bar.

## Tray icon

```php
use Pam\Desktop\Tray;

Tray::show(
    Menu::make(
        'Tray',
        MenuItem::make('New document', $this->create(...)),
        MenuItem::separator(),
        MenuItem::role(MenuRole::Quit),
    ),
    'PAM Desktop shell',
    onActivate: function (): void {
        Window::focus();
    },
);
```

`Tray::show($menu, $tooltip, $icon, $onActivate)` shows or updates the icon; `Tray::hide()` removes it. `$icon` is the path of a PNG file; without one the application icon is used. `$onActivate` runs when the icon itself is clicked.

The icon lives in the menu bar extras on macOS, the notification area on Windows and a StatusNotifierItem on Linux. On Linux the desktop must run a host for that protocol: see [Platforms](platforms.md#tray-icon).

## Shortcuts

```php
use Pam\Desktop\Shortcut;

$id = Shortcut::register('CmdOrCtrl+K', function (): void {
    $this->searching = true;
});

Shortcut::register('Ctrl+Alt+P', $this->toggle(...), global: true);

Shortcut::unregister($id);
```

A local shortcut fires while the application has the focus, on keys that no text control consumed. A global shortcut (`global: true`) also fires while another application has the focus, where the system allows it.

A combination joins modifiers and one key with `+`:

| Part | Accepted names |
| --- | --- |
| Primary modifier | `CmdOrCtrl` (also `CommandOrControl`, `Mod`): Command on macOS, Control elsewhere |
| Other modifiers | `Ctrl`, `Alt` (or `Option`), `Shift`, `Cmd` (or `Meta`, `Super`, `Win`) |
| Key | A character (`K`, `7`, `,`), a function key (`F5`), a key name (`Enter`, `Escape`, `Space`, `Tab`, `Backspace`, `Delete`, `Home`, `End`, `PageUp`, `PageDown`, `ArrowUp` or `Up`…), or `Plus`, `Minus`, `Comma`, `Period`, `Slash` |

The shortcuts of menu items use the same syntax.

How global shortcuts are bound depends on the system. On Linux they are key grabs on X11 and go through the desktop portal on Wayland; see [Platforms](platforms.md#global-shortcuts) for what was verified.

## File dialogs

`Dialog` opens the file dialogs of the operating system. The callbacks receive absolute paths; a cancelled dialog reports `null` (or an empty list).

```php
use Pam\Desktop\Dialog;
use Pam\Desktop\FileFilter;

Dialog::openFiles(function (array $paths): void {
    foreach ($paths as $path) {
        $this->documents[] = basename($path);
    }
}, 'Open documents', [new FileFilter('Documents', 'md', 'txt'), FileFilter::images(), FileFilter::all()]);

Dialog::saveFile(function (?string $path): void {
    if ($path !== null) {
        file_put_contents($path, $this->export());
    }
}, 'report.md', 'Export document');
```

| Method | Callback receives |
| --- | --- |
| `openFile($completed, $title, $filters, $directory)` | `?string` |
| `openFiles($completed, $title, $filters, $directory)` | `list<string>` |
| `saveFile($completed, $defaultName, $title, $filters, $directory)` | `?string` |
| `selectFolder($completed, $title, $directory)` | `?string` |

A `FileFilter` is a name and a list of extensions without the dot; `*` accepts every file.

These dialogs give the application real paths, which PHP then reads and writes with its own file functions. To copy a user's file into the application's private sandbox instead, use `Files::pick()`: see [Modules](modules.md#files).

## Notifications

```php
use Pam\Desktop\Notification;

Notification::show('Export finished', 'report.pdf is ready', function (): void {
    Window::focus();
});
```

The notification is shown by the operating system. The optional third argument runs when the user clicks it. `show()` returns the id of the notification.

## Drag and drop

### Text between elements

`InteractionRegion` is a box that can be dragged or accept drops. A draggable region carries a string; a region that accepts drops receives it.

```php
use Pam\Desktop\UI\InteractionRegion;

InteractionRegion::make(Text::make($card))
    ->key($card)
    ->draggable($card)
    ->cursor('grab');

InteractionRegion::make(Text::make('Done'), ...$cards)
    ->acceptsDrop()
    ->onDrop(function (string $data): void {
        $this->move($data, 'Done');
    });
```

The drag starts once the pointer moves a few units with the button down, and it is a system drag: text dragged from another application onto a region that accepts drops arrives the same way, and a draggable region can be dropped on other applications. `onDragStart` and `onDragEnd` report the two ends of a drag on its source.

### Files from the system

```php
Column::make(/* … */)->onFilesDrop(function (array $paths): void {
    $this->import($paths);
});
```

`onFilesDrop()` exists on every element and receives the absolute paths of files dropped on it from the file manager. The innermost element under the pointer that handles files takes the drop. Files dropped anywhere else reach the listeners of `Window::onFilesDrop()`.

## Window dragging

`windowDrag()` makes an element behave like a title bar: dragging it moves the window. Controls inside it keep working.

```php
Row::make(
    Text::make('My application'),
)->windowDrag()->style(new Style(height: 40, paddingHorizontal: 16, alignItems: Align::Center));
```

## Keyboard events

`onKeyDown()` and `onKeyUp()` exist on every element. The handler receives a `KeyEvent`.

```php
use Pam\Desktop\KeyEvent;

Column::make(...$rows)->onKeyDown(function (KeyEvent $event): void {
    $this->selected = match ($event->key) {
        'ArrowDown' => $this->selected + 1,
        'ArrowUp' => max(0, $this->selected - 1),
        default => $this->selected,
    };
});
```

| Property | Meaning |
| --- | --- |
| `key` | The W3C UI Events `key` value: `Enter`, `Escape`, `ArrowLeft`, `a`, `F5`… |
| `shift`, `control`, `alt`, `meta` | Modifiers. `meta` is Command on macOS and the Windows or Super key elsewhere. |
| `repeat` | The key is being held. |
| `primary()` | Command on macOS, Control elsewhere. |

A key that no text control consumed goes to the nearest listener at or above the focused element. When nothing has focus it goes to every visible listener, which is how a screen handles keys globally.

## Hover events

`onHoverIn()` and `onHoverOut()` exist on every element and report the pointer entering and leaving it. [Styling](styling.md#hover) shows how to style with them.

## Clipboard

```php
use Pam\Desktop\System\Clipboard;

Clipboard::setText('Copied text', function (bool $copied): void {
    // …
});

Clipboard::getText(function (?string $text): void {
    $this->pasted = $text ?? '';
});
```

`setText()` accepts up to one megabyte. `getText()` reports `null` when the clipboard holds no text or cannot be read; `hasText()` reports a boolean.

## Platform checks

`Pam\Desktop\Platform` tells which system the application runs on:

```php
use Pam\Desktop\Platform;

$modifier = Platform::select(['macos' => 'Cmd', 'default' => 'Ctrl']);

if (Platform::isLinux()) {
    // …
}
```

`Platform::os()` returns `linux`, `macos` or `windows`.
