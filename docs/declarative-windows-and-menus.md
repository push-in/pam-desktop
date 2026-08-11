# Declarative windows and menus

## Named windows

A named window is a type, not a string passed around the application:

```php
#[Window(
    name: 'settings',
    title: 'Settings',
    page: 'resources/settings.html',
    width: 760,
    height: 620,
)]
final readonly class SettingsWindow extends DesktopWindow
{
}
```

Register it once:

```php
protected function windows(): array
{
    return [SettingsWindow::class];
}
```

Then inject it anywhere:

```php
#[Command]
public function openSettings(SettingsWindow $settings): void
{
    $settings->show()->focus();
}
```

The class is invocation-scoped and only accumulates validated effects. It does
not expose a native handle.

## Action-oriented menus

Menu actions live beside their behavior:

```php
#[Menu(
    id: 'app',
    label: 'My desktop app',
    close: TrayCloseBehavior::Hide,
)]
final class ApplicationMenu
{
    #[MenuItem('Show window', shortcut: 'CmdOrCtrl+Shift+KeyP')]
    public function show(WindowHandle $window): void
    {
        $window->show()->focus();
    }

    #[MenuSeparator]
    public function separator(): void {}

    #[MenuItem('Background mode', checkbox: true)]
    public function background(): void {}

    #[MenuItem('Quit')]
    public function quit(ApplicationControl $application): void
    {
        $application->quit();
    }
}
```

Register menu classes through `menus()`. Item method names become command IDs,
explicit IDs remain available, shortcuts become validated native global
shortcuts, and a tray is produced from the class-level declaration.

The immutable `Menu`, `MenuItem`, `Tray`, `GlobalShortcut`, and `Shell` builders
remain available when a generated or deeply nested menu tree is more suitable.
