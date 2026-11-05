<!-- pam:product-page:start -->
<div align="center">

# PAM Desktop

**Native desktop applications written in PHP.**

Components, state and styles in PHP; a Rust host that lays the tree out and drives real platform views. No Electron, no Node.js, no web view in the rendering path.

![PHP](https://img.shields.io/badge/PHP-8.5-777BB4?style=flat-square&logo=php&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-1.92-000000?style=flat-square&logo=rust&logoColor=white)
![License](https://img.shields.io/badge/license-MIT-green?style=flat-square)

**[Getting started](docs/getting-started.md) · [Components](docs/components.md) · [Architecture](docs/architecture.md) · [Platforms](docs/platforms.md) · [Issues](https://github.com/push-in/pam-desktop/issues)**

</div>

---

## Why this exists

A PHP developer should be able to ship a desktop application whose interface is made of the operating system's own views, without writing HTML, CSS or JavaScript and without bundling a browser.

| | |
| --- | --- |
| **Role** | Desktop product |
| **Execution path** | PHP worker · Rust host · GTK 4, AppKit or Win32 + Direct2D |
| **This repository owns** | The PHP component package, the wire protocol, the layout and diff engine, the render core, the platform backends and the `pam-desktop` host binary |
| **Boundary** | PHP describes the interface and handles events; the host owns windows, input, animation, scrolling, media and system services |

## What you can build

- Tools with native windows, menus, tray icons, shortcuts and file dialogs
- Document and data applications with long lists, local SQLite databases and a private file sandbox
- Media and drawing applications with images, video, canvases and gestures

## Quick start

```bash
composer require pushinbr/pam-desktop
vendor/bin/pam-desktop init
vendor/bin/pam-desktop dev
```

`vendor/bin/pam-desktop` downloads the native host of the installed version for the current system, verifies its SHA-256 and runs it. Version 2.0.0 is not released yet: there is no `v2.0.0` tag for Composer to install and no host archive to download. Until there is, work from a checkout (see below); `PAM_DESKTOP_HOST` points the launcher at a host built from source.

[Getting started](docs/getting-started.md) covers the requirements, the project layout and the `pam-desktop.json` reference.
<!-- pam:product-page:end -->

> PAM Desktop 2.0 is a different product from 1.x. Version 1 rendered a web interface in a Servo web view behind a typed command gateway; version 2 renders a native interface from PHP components. Applications written for 1.x do not run on 2.0. See [Migrating from 1.x](docs/migrating-from-1.x.md).

## What it is

A PAM Desktop application is a PHP script. It declares components with reactive state and returns a tree of elements (`Column`, `Row`, `Text`, `Button`, `Input`, `VirtualizedList`…), using the same component syntax as PAM Native, the mobile sibling of this project.

The `pam-desktop` host starts that script in a PHP worker process and talks to it over stdin and stdout in binary frames. For every frame the host lays the tree out, computes the difference from the previous one and updates real platform views:

- GTK 4 on Linux
- AppKit on macOS
- Win32 with Direct2D and DirectWrite on Windows

Rendering never goes through a web view. A `WebView` element exists for the cases where an application wants to embed a page.

The host does not wait for PHP to animate, scroll, track gestures, play media or edit text. PHP runs when an event needs application logic and sleeps otherwise.

## A minimal application

`examples/counter/index.php`:

```php
<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Component;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Style;
use Pam\Desktop\UI\Button;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\Text;

require __DIR__.'/../../vendor/autoload.php';

final class Counter extends Component
{
    #[State]
    public int $count = 0;

    public function render(): Column
    {
        return Column::make(
            Text::make('PAM Desktop')->style(new Style(fontSize: 28, fontWeight: 700)),
            Text::make("Count: {$this->count}")->style(new Style(fontSize: 18)),
            Button::make('Increment')->onPress(function (): void {
                $this->count++;
            }),
        )->style(new Style(padding: 24, gap: 16));
    }
}

App::run(new Counter());
```

Assigning to a `#[State]` property schedules a render. The host receives a patch with the one text that changed.

## Requirements

| Requirement | Notes |
| --- | --- |
| PHP 8.5 | The `php` executable on `PATH`, or `pam` (the host then runs `pam exec`), or the path in `PAM_DESKTOP_PHP`. |
| Composer | To install the `pushinbr/pam-desktop` package. |
| Linux, to run a released host | glibc 2.39 or newer, GTK 4.14 or newer, WebKitGTK 6 (`libwebkitgtk-6.0-4`) and the GStreamer plugins for media. The released host is built on Ubuntu 24.04. |
| Rust 1.92 or newer | Only to build the host from a checkout. `rust-toolchain.toml` selects the stable channel. |
| Linux, to build: `libgtk-4-dev` | GTK 4.14 or newer. |
| Linux, to build: `libwebkitgtk-6.0-dev` | Only for the `webview` cargo feature of the host (on by default). |

The other optional features of the host on Linux, `tray` and `hotkeys`, need no extra system library. [Platforms](docs/platforms.md) lists what each feature does.

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

## Platforms

| System | Backend | Status |
| --- | --- | --- |
| Linux | GTK 4 (`pam-desktop-gtk`) | The reference backend. Every example above was run and checked on Linux. |
| macOS | AppKit (`pam-desktop-appkit`) | Written against the same `Backend` contract and type-checked by cross compilation. Not yet run on a real machine: first runs are expected to need fixes. |
| Windows | Win32 + Direct2D/DirectWrite (`pam-desktop-win32`) | Same as macOS: written and type-checked, not yet run on a real machine. |

The host starts the backend of the system it was built for. Two things were not verified on Linux either: the accessibility tree with a real screen reader, and global shortcuts through the Wayland portal (X11 key grabs were verified).

Details, dependencies and the commands that cross-check the macOS and Windows backends from Linux are in [Platforms](docs/platforms.md). [First run on macOS and Windows](docs/first-run-macos-windows.md) is the checklist for the first run on those systems and the list of known differences from the GTK backend.

## Features

| Area | What you get | Document |
| --- | --- | --- |
| Components | Layout, text, buttons, inputs, switches, lists, scroll views, modals, bottom sheets, stack navigation with a header, tabs | [Components](docs/components.md) |
| Styling | `Style`, pressed/hover/focus styles, cursors, tooltips, dark mode, text scale | [Styling](docs/styling.md) |
| Desktop shell | Main window control, secondary windows, menu bar, context menus, tray, local and global shortcuts, file dialogs, notifications, drag and drop, clipboard | [Desktop shell](docs/desktop-shell.md) |
| Media and web | Images with a disk cache, video and audio, embedded web pages, vector and freehand canvases | [Media and web](docs/media-and-web.md) |
| Motion | Transitions, presets, animation programs, gesture detectors, drag contracts, swipeable rows, native pan and zoom | [Animation and gestures](docs/animation-and-gestures.md) |
| Modules | Key/value storage, file sandbox, HTTP, SQLite, secure storage, timers, device information and status, links and deep links with a single instance, toasts, alerts | [Modules](docs/modules.md) |
| Accessibility | Roles, labels, states and actions mapped to AT-SPI, NSAccessibility and UI Automation; announcements; text selection; keyboard navigation | [Accessibility](docs/accessibility.md) |
| Development | Hot reload with state preservation, the error overlay, screenshots on a virtual display, test suites, static analysis, CI | [Development](docs/development.md) |
| Internals | Processes, the frame protocol, the render core, the `Backend` contract, threading | [Architecture](docs/architecture.md) |
| Shipping | Packaging and installers | [Distribution](docs/distribution.md) |
| Updates | Application updates | [Updates](docs/updates.md) |

Capabilities that only exist on phones in PAM Native (location, contacts, sensors, biometrics, SMS…) are not part of PAM Desktop. Their class names are still known: using one throws a `PluginException` that names the optional package it belongs to.

## Repository layout

```text
crates/
  pam-desktop-wire     frame, patch and mutation batch encoding; property and event tables
  pam-desktop-engine   layout and tree diff
  pam-desktop-ui       platform-neutral render core and the Backend trait
  pam-desktop-gtk      Linux backend (GTK 4)
  pam-desktop-appkit   macOS backend (AppKit)
  pam-desktop-win32    Windows backend (Win32, Direct2D, DirectWrite)
  pam-desktop-host     the pam-desktop binary; SQLite, secure storage, packaging and updates
packages/desktop/      the PHP package (Pam\Desktop), the Composer launcher, the worker bootstrap and tests
examples/              one runnable application per feature
scripts/               shot.sh captures an example on a virtual display; package-host.sh packs a release archive
.github/workflows/     ci.yml (checks) and release.yml (host archives of a tag)
docs/                  this documentation
```

## Documentation

- [Getting started](docs/getting-started.md)
- [Components](docs/components.md)
- [Styling](docs/styling.md)
- [Desktop shell](docs/desktop-shell.md)
- [Media and web](docs/media-and-web.md)
- [Animation and gestures](docs/animation-and-gestures.md)
- [Modules](docs/modules.md)
- [Accessibility](docs/accessibility.md)
- [Development](docs/development.md)
- [Architecture](docs/architecture.md)
- [Platforms](docs/platforms.md)
- [First run on macOS and Windows](docs/first-run-macos-windows.md)
- [Distribution](docs/distribution.md)
- [Updates](docs/updates.md)
- [Migrating from 1.x](docs/migrating-from-1.x.md)
- [Changelog](CHANGELOG.md)

## License

PAM Desktop is released under the [MIT License](LICENSE).

The reactive PHP component core, the binary wire protocol and the layout and diff engine started as a copy of [PAM Native](https://github.com/push-in/pam-native) and remain under the Apache License 2.0. [NOTICE](NOTICE) lists the origin release and the directories that hold the copied code. The two projects do not depend on each other; fixes are ported between them by hand.
