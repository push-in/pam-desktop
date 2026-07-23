# Native shell

Pam Desktop 0.6 declares menus, tray behavior, and global shortcuts in immutable
PHP objects. Native code owns operating-system handles and sends selections
back through the normal typed event path.

## Configuration

```php
use Pam\Desktop\GlobalShortcut;
use Pam\Desktop\Menu;
use Pam\Desktop\MenuItem;
use Pam\Desktop\Shell;
use Pam\Desktop\Tray;
use Pam\Desktop\TrayCloseBehavior;

$shell = Shell::none()
    ->menu(Menu::create(
        'application',
        'Application',
        MenuItem::command('show', 'Show window', 'CmdOrCtrl+Shift+KeyP'),
        MenuItem::checkbox('background', 'Run in background', true),
        MenuItem::submenu(
            'tools',
            'Tools',
            MenuItem::command('inspector', 'Runtime inspector'),
        ),
        MenuItem::separator(),
        MenuItem::command('quit', 'Quit'),
    ))
    ->tray(
        Tray::create('application', 'My application')
            ->closeBehavior(TrayCloseBehavior::Hide),
    )
    ->shortcut(
        GlobalShortcut::create('show', 'CmdOrCtrl+Shift+KeyP'),
    );

$app->shell($shell);
```

Every ID is unique across the shell. Menus accept at most 256 items and eight
levels. Supported item kinds are sequential integers: command `1`, checkbox
`2`, separator `3`, and submenu `4`. Tray close behavior is exit `1` or hide
`2`.

Accelerators use explicit `+`-separated tokens such as
`CmdOrCtrl+Shift+KeyP`. At least one modifier is required. The same syntax is
used for menu accelerators and global shortcuts.

## Events

Selections and operating-system activation are published to JavaScript and
sent asynchronously to an optional PHP handler:

```js
window.pam.on("pam.menu.selected", ({ id }) => {
    console.log("menu", id);
});

window.pam.on("pam.tray.activated", ({ button }) => {
    console.log("tray button", button); // 1 left, 2 right, 3 middle
});

window.pam.on("pam.shortcut.changed", ({ id, state }) => {
    // state: 1 pressed, 2 released
});
```

```php
use Pam\Desktop\EventContext;
use Pam\Desktop\WindowEffect;

$app->on('pam.menu.selected', static function (EventContext $event) {
    return match ($event->string('id')) {
        'show' => \Pam\Desktop\CommandResult::success()
            ->effect(WindowEffect::visible(true))
            ->effect(WindowEffect::focus()),
        'quit' => \Pam\Desktop\CommandResult::success()
            ->effect(WindowEffect::close()),
        default => null,
    };
});
```

An unregistered native event handler is ignored by PHP while JavaScript still
receives the event. This makes native shell configuration usable without
requiring a PHP callback for every item.

## Dynamic state

Handlers and background jobs may return typed shell effects:

```php
use Pam\Desktop\CommandResult;
use Pam\Desktop\ShellEffect;

return CommandResult::success()
    ->effect(ShellEffect::menuEnabled('inspector', false))
    ->effect(ShellEffect::menuChecked('background', true))
    ->effect(ShellEffect::trayVisible(true));
```

Effects address only declared native objects. There is no generic
operating-system menu or window handle in PHP or JavaScript.

## Linux behavior

Linux uses the StatusNotifierItem protocol over D-Bus. Menu trees, checkbox
state, tray activation, and visibility are available without linking the Servo
host to GTK or AppIndicator. Desktop environments without a StatusNotifier
watcher may not display a tray icon; the application continues running and
menus remain configuration data.

Global shortcut registration is best-effort because compositors and desktop
policies may reserve or reject an accelerator. Unsupported syntax fails boot;
an unavailable operating-system registration emits a host warning and leaves
the application usable.

With close behavior `Hide`, closing the main window hides it while the tray
remains active. Use a menu item or global shortcut that returns
`WindowEffect::visible(true)` and `WindowEffect::focus()` to restore it.

