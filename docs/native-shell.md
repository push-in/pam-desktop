# Native shell

Pam Desktop 1.0 declares menus, tray behavior, and global shortcuts in immutable
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

The stable Linux shell accepts one menu tree per application and uses it as the
tray menu, so declaring a menu requires a matching `Tray`. Shortcuts remain
valid without a tray. Dynamic menu, checkbox and tray effects fail explicitly
if no tray is configured.

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

Window chrome, transparency, fullscreen, maximization and stacking belong to
the window contract rather than the tray/menu shell. See [Native windows](windows.md).

## Application badge and taskbar progress

Status effects do not require a tray:

```php
use Pam\Desktop\ShellEffect;
use Pam\Desktop\TaskbarProgressState;

return CommandResult::success()
    ->effect(ShellEffect::badge(7))
    ->effect(ShellEffect::taskbarProgress(0.65, TaskbarProgressState::Normal));
```

Use `badge(null)` and `taskbarProgress(0, TaskbarProgressState::Hidden)` to
clear the indicators. Counts are bounded to 9,999 and progress to `0.0..1.0`.
The implementation is native on every desktop target:

| Platform | Badge | Progress |
| --- | --- | --- |
| Linux | Unity LauncherEntry count | Unity progress and error urgency |
| Windows | Generated numeric taskbar overlay with accessible description | `ITaskbarList3`, including indeterminate, paused and error states |
| macOS | `NSDockTile.badgeLabel` | Dock tile preserving the application icon with a determinate or animated indeterminate bar |

Linux uses the packaged desktop identifier and the standard
`com.canonical.Unity.LauncherEntry.Update` session-bus signal. Windows confines
COM/GDI calls to a small audited internal crate and releases every bitmap/icon
handle after the shell copies it. macOS requires and verifies the AppKit main
thread before changing its Dock tile. None of the backends poll.

## Quick actions

Quick actions are independent of tray menus and global shortcuts:

```php
use Pam\Desktop\QuickAction;
use Pam\Desktop\Shell;

$shell = Shell::none()->quickAction(
    QuickAction::create('compose', 'New message')
        ->description('Open the message composer'),
    QuickAction::create('search', 'Search'),
);
```

Selection arrives as `pam.quick-action.selected` with `{ id }`. Only identifiers
declared in the signed bootstrap are accepted; forged reserved launcher
arguments are discarded. The packaged launcher preserves quick-action, deep
link and file-association arguments instead of consuming them.

Linux packages emit freedesktop `Desktop Action` sections for every declared
action. Packaged Windows applications atomically publish the same bounded list
as `IShellLinkW` user tasks in the application's Jump List, targeting the public
launcher rather than the privileged host binary. Each link carries only the
validated reserved identifier. macOS Dock-menu publication remains a separate
release gate; a build must not claim that surface from another platform's
contract.
