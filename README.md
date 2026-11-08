<!-- pam:product-page:start -->
<div align="center">

# PAM Desktop

**Native desktop applications written in PHP.**

Components, state and styles in PHP; a Rust host that lays the tree out and drives real platform views. No Electron, no Node.js, no web view in the rendering path.

![PHP](https://img.shields.io/badge/PHP-8.5-777BB4?style=flat-square&logo=php&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-1.92-000000?style=flat-square&logo=rust&logoColor=white)
![License](https://img.shields.io/badge/license-MIT-green?style=flat-square)

**[Getting started](docs/getting-started.md) · [Components](docs/components.md) · [Templates](docs/templates.md) · [Styling](docs/styling.md) · [Platforms](docs/platforms.md) · [Issues](https://github.com/push-in/pam-desktop/issues)**

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
mkdir my-app && cd my-app
composer require pushinbr/pam-desktop
vendor/bin/pam-desktop init
pam desktop dev          # or: vendor/bin/pam-desktop dev, or: composer dev
```

`init` writes a small application: a router, a home screen as a `.pam.php` template with scoped CSS, design tokens, a service and a test. `dev` needs no argument (it finds the nearest `pam-desktop.json`), reloads the window about 0.3 s after a save and keeps `#[State]` values. `pam desktop dev` is the same command through the [PAM CLI](https://github.com/push-in/pam). Without PHP on the machine, the host installs it (the PAM runtime on Linux and macOS, the official windows.php.net build on Windows).

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
| PHP 8.5 | The `php` executable on `PATH`, or `pam` (the host then runs `pam exec`), or the path in `PAM_DESKTOP_PHP`. On Linux and macOS without any of them, the host installs the PAM runtime, which embeds PHP; on Windows it downloads the official PHP 8.5 build from windows.php.net (not run on Windows yet). |
| Composer | To install the `pushinbr/pam-desktop` package. |
| Linux, to run a released host | glibc 2.35 or newer, GTK 4.6 or newer and the GStreamer plugins for media (Ubuntu 22.04, Debian 12, Fedora 36 and later). WebKitGTK 6 (`libwebkitgtk-6.0-4`, Ubuntu 24.04 and later) only for applications that show a `WebView`. The released host is built on Ubuntu 22.04 and uses the GTK features newer than 4.6 when the installed GTK has them. |
| Rust 1.92 or newer | Only to build the host from a checkout. `rust-toolchain.toml` selects the stable channel. |
| Linux, to build: `libgtk-4-dev` | GTK 4.6 or newer. |

The optional features of the host on Linux, `tray` and `hotkeys`, need no extra system library. [Platforms](docs/platforms.md) lists what each feature does.

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

## Platforms

| System | Backend | Status |
| --- | --- | --- |
| Linux | GTK 4 (`pam-desktop-gtk`) | The reference backend. Every example above, and every feature below, was run and checked on Linux. |
| macOS | AppKit (`pam-desktop-appkit`) | Run on macOS 26 (x86_64) in a virtual machine without a GPU: the core examples, hot reload, keyboard and pointer input, a real application and a DMG from `pam-desktop build`. Not run on Apple Silicon; blur, media, web views, notifications, printing and drag and drop not verified. |
| Windows | Win32 + Direct2D/DirectWrite (`pam-desktop-win32`) | Written against the same `Backend` contract and type-checked by cross compilation. Not run on Windows yet: first runs are expected to need fixes. |

The host starts the backend of the system it was built for. Two things were not verified on Linux either: the accessibility tree with a real screen reader, and global shortcuts through the Wayland portal (X11 key grabs were verified).

Details, a per-feature status table, dependencies and the commands that cross-check the macOS and Windows backends from Linux are in [Platforms](docs/platforms.md). [First run on macOS and Windows](docs/first-run-macos-windows.md) is the checklist for the first run on those systems and the list of known differences from the GTK backend.

## Features

| Area | What you get | Document |
| --- | --- | --- |
| Components | Layout, text with inline boxes, buttons, inputs (masks, currency, decimal), switches, content-sized lists, scroll views, modals with backdrop blur, bottom sheets, stack navigation with a header, tabs | [Components](docs/components.md) |
| Templates | `.pam.php` single-file components: expressions with enums, constants and value methods, `<Slot :content>`, per-component error isolation | [Templates](docs/templates.md) |
| Styling | `Style` and scoped CSS: state selectors with `:focus-within`, `@media`, font stacks and packaged fonts, line breaking, `white-space`, styled scrollbars, `backdrop-filter`, fractional grid tracks, dark mode, text scale | [Styling](docs/styling.md), [Grid tracks](docs/grid-tracks.md) |
| Controls | Select, Autocomplete, MultiSelect, DatePicker, Toast and the window toast service, Checkbox, ProgressButton (`pushinbr/pam-desktop-ui`) | [DesktopUI](docs/desktop-ui.md) |
| Internationalization | Framework defaults in English, Portuguese and Spanish, following the system or `I18n::use()`, overridable per app and per component | [Internationalization](docs/i18n.md) |
| Charts | Line, area, bar, donut, sparkline, progress ring on the native canvas (`pushinbr/pam-desktop-charts`) | [Charts](docs/charts.md) |
| Desktop shell | Main window control, secondary windows, menu bar, context menus, tray, local and global shortcuts, key capture, file dialogs, notifications, typed drag and drop, clipboard | [Desktop shell](docs/desktop-shell.md) |
| Media and web | Images with a disk cache, video and audio, embedded web pages, vector and freehand canvases | [Media and web](docs/media-and-web.md) |
| Motion | Transitions, presets, animation programs, gesture detectors, drag contracts, swipeable rows, native pan and zoom | [Animation and gestures](docs/animation-and-gestures.md) |
| Modules | Key/value storage, file sandbox and save dialog, HTTP, WebSocket, SQLite with guarded transactions, secure storage, printing, timers, device information and status, links and deep links with a single instance, toasts, alerts | [Modules](docs/modules.md) |
| Accessibility | Roles, labels, states and actions mapped to AT-SPI, NSAccessibility and UI Automation; announcements; text selection; keyboard navigation | [Accessibility](docs/accessibility.md) |
| Development | Hot reload with state preservation, the error overlay, screenshots on a virtual display, test suites, static analysis, CI | [Development](docs/development.md) |
| Internals | Processes, the frame protocol, the render core, the `Backend` contract, threading | [Architecture](docs/architecture.md) |
| Compatibility | Host and SDK versions, the property table and what an older host refuses | [Compatibility](docs/compatibility.md) |
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
packages/desktop/      the PHP package (Pam\Desktop), the Composer launcher, the worker bootstrap, the init skeleton and tests
packages/desktop-ui/   ready controls (Pam\DesktopUi)
packages/charts/       charts (Pam\Desktop\Charts), drawn with packages/canvas/
packages/desktop-printing/  native printing of text documents and receipts
examples/              one runnable application per feature
scripts/               shot.sh captures an example on a virtual display; visual.sh compares the examples with a baseline; package-host.sh packs a release archive
.github/workflows/     ci.yml (checks) and release.yml (host archives of a tag)
docs/                  this documentation
```

## Documentation

- [Getting started](docs/getting-started.md)
- [Components](docs/components.md)
- [Templates](docs/templates.md)
- [Styling](docs/styling.md)
- [Grid tracks](docs/grid-tracks.md)
- [DesktopUI](docs/desktop-ui.md)
- [Charts](docs/charts.md)
- [Desktop shell](docs/desktop-shell.md)
- [Media and web](docs/media-and-web.md)
- [Animation and gestures](docs/animation-and-gestures.md)
- [Modules](docs/modules.md)
- [Accessibility](docs/accessibility.md)
- [Development](docs/development.md)
- [Architecture](docs/architecture.md)
- [Compatibility](docs/compatibility.md)
- [Platforms](docs/platforms.md)
- [First run on macOS and Windows](docs/first-run-macos-windows.md)
- [Distribution](docs/distribution.md)
- [Updates](docs/updates.md)
- [Migrating from 1.x](docs/migrating-from-1.x.md)
- [Changelog](CHANGELOG.md)

## License

PAM Desktop is released under the [MIT License](LICENSE).

The reactive PHP component core, the binary wire protocol and the layout and diff engine started as a copy of [PAM Native](https://github.com/push-in/pam-native) and remain under the Apache License 2.0. [NOTICE](NOTICE) lists the origin release and the directories that hold the copied code. The two projects do not depend on each other; fixes are ported between them by hand.
