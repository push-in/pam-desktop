# Native windows

Every declared PAM window owns a Winit window, Servo WebView and rendering
context. Initial state is immutable PHP policy; runtime changes use typed
effects addressed to a declared window identifier.

```php
use Pam\Desktop\Window;
use Pam\Desktop\WindowTheme;

$window = Window::create('Studio')
    ->load('resources/studio.html')
    ->size(1440, 900)
    ->minimumSize(720, 520)
    ->theme(WindowTheme::Dark)
    ->decorated(false)
    ->transparent()
    ->alwaysOnTop()
    ->maximized();
```

Linux window managers and compositors retain final authority over placement,
focus, transparency and stacking. PAM forwards supported state explicitly and
does not fake native behavior inside CSS.

## Runtime effects

```php
return CommandResult::success()
    ->effect(WindowEffect::fullscreen(true, 'player'))
    ->effect(WindowEffect::alwaysOnTop(true, 'player'))
    ->effect(WindowEffect::maximized(false, 'player'))
    ->effect(WindowEffect::attention(critical: true, windowId: 'player'));
```

Fullscreen uses borderless monitor fullscreen. Always-on-top maps to Winit's
window level contract. Effects with malformed payloads or unknown window IDs
are ignored by the host and reported through diagnostics instead of exposing a
raw native handle.

`attention()` maps to the operating system's native urgency mechanism: taskbar
flashing on Windows, Dock attention on macOS, and the compositor/window-manager
attention hint on supported Linux sessions. Send `attention(active: false)` to
cancel a pending request. PAM does not emulate urgency with renderer animation.
