# Examples

Every directory under `examples/` is a runnable application that exercises one feature. This page was moved out of the README; nothing was removed.

## Run an example from a checkout

```bash
git clone https://github.com/push-in/pam-desktop.git
cd pam-desktop
composer install
cargo build -p pam-desktop-host
target/debug/pam-desktop dev examples/counter
```

`composer install` writes the `vendor/autoload.php` that the examples load. `dev` restarts the PHP worker when a source file changes and keeps `#[State]` values across the restart. `run` starts the application without the watcher, with production error handling:

```bash
target/debug/pam-desktop run examples/showcase
```

Every directory under `examples/` is a runnable application that exercises one feature:

| Example | Shows |
| --- | --- |
| `counter` | State and a button |
| `showcase` | A larger application: routes, templates, lists, themes |
| `controls` | Switch, activity indicator, masked inputs, dialog, bottom sheet |
| `lists`, `chat` | A 2000-row virtualized list; an inverted list |
| `navigation` | Stack navigation, the header, search, a form sheet |
| `motion` | Implicit transitions, springs, endless programs, presets, shimmer |
| `gestures` | Swipeable rows, long press, swipe, native pan and zoom |
| `images`, `media`, `webview`, `canvas` | Images, the media player, the web view, vector and freehand canvases |
| `shell`, `windows` | Menu bar, context menus, tray, shortcuts, dialogs, notifications; secondary windows |
| `dragdrop`, `text` | Drag and drop between elements; selectable text |
| `modules` | Timers, storage, alerts, clipboard, device information, appearance |
| `links` | A URL scheme, a single instance that receives the link of every later launch, a screen reader announcement |
| `toast`, `errors` | Toasts; the development error overlay |
| `web-defaults` | Web semantics: a content-sized list, CSS font stacks, thin classic scrollbars, value methods in templates, Chart.js value axes |
