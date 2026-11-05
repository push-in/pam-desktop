# Getting started

This page takes you from a checkout to a running application, then explains the project layout, the `pam-desktop.json` manifest, the two run modes and where an application keeps its files.

## Requirements

- PHP 8.5.
- A Rust toolchain, 1.92 or newer, to build the host. `rust-toolchain.toml` selects the stable channel.
- On Linux: `libgtk-4-dev` (GTK 4.14 or newer) and, for the `webview` feature, `libwebkitgtk-6.0-dev`.

[Platforms](platforms.md) has the details for each system.

## Build the host and run an example

```bash
cargo build -p pam-desktop-host
target/debug/pam-desktop dev examples/counter
```

A window opens with a counter. Edit `examples/counter/index.php` while it runs: the window updates and the count is kept.

## The command line

```text
pam-desktop dev [app]                 run with hot reload (default: current directory)
pam-desktop run [app]                 run the application
pam-desktop build [app] [options]     bundle the application and build its installers
pam-desktop update-key --output FILE  create the key that signs updates
pam-desktop publish-update [app] ...  sign the update feed of a release
pam-desktop --version
```

This page covers `dev` and `run`. `build` is described in [Distribution](distribution.md); `update-key` and `publish-update` in [Updates](updates.md).

`app` is a project directory or a PHP file. With a directory the entry script is `index.php` inside it. With a file, the file is the entry script and its directory is the project.

Exit codes of `dev` and `run`:

| Exit code | Meaning |
| --- | --- |
| 64 | Unknown command (the usage text is printed). |
| 66 | The entry script does not exist. |
| 69 | The worker bootstrap of the `pushinbr/pam-desktop` package was not found. |
| 73 | The application directories could not be created. |
| 1 | The PHP worker could not be started, or the platform is not supported. |

## A first application

```php
<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\Style;
use Pam\Desktop\UI\Button;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\Text;

require __DIR__.'/vendor/autoload.php';

final class Counter extends Component
{
    #[State]
    public int $count = 0;

    public function mount(): void
    {
        // Runs once, before the first render.
    }

    public function render(): Column
    {
        return Column::make(
            Text::make("Count: {$this->count}")->style(new Style(fontSize: 18)),
            Button::make('Increment')->onPress(function (): void {
                $this->count++;
            }),
        )->style(new Style(padding: 24, gap: 16));
    }
}

App::run(new Counter());
```

The pieces:

- A component extends `Pam\Desktop\Component` and implements `render()`, which returns an element tree.
- A public property marked `#[State]` is reactive: assigning to it schedules a new render.
- Elements are immutable values. Every method such as `style()` or `onPress()` returns a modified copy, so calls chain.
- `App::run()` boots the application with a root. The root can be a component, a closure that returns an element, or a navigator (see [Components](components.md)).

`echo`, `print` and `var_dump` do not corrupt anything: the worker's standard output belongs to the host channel, so their text is redirected to the host's standard error, where you launched `pam-desktop`.

## Project layout

```text
my-app/
  index.php           entry script
  pam-desktop.json    manifest (optional)
  composer.json
  vendor/             Composer autoloader and the pushinbr/pam-desktop package
  assets/             files the application loads with asset:// URLs
  .pam-desktop/       created by `pam-desktop dev`: data, cache and state
```

Only the entry script is required. The project directory is:

- the working directory of the PHP worker;
- the root that `asset://` URLs resolve in (`asset://assets/logo.png` is `assets/logo.png` in the project);
- the directory watched by `dev`.

### How the host finds PHP and the package

The host starts the worker as `<php> <bootstrap> <entry script>`.

`<php>` is, in order: the path in `PAM_DESKTOP_PHP`; `pam` found on `PATH` (the host then runs `pam exec`); `php` found on `PATH`.

`<bootstrap>` is the file `packages/desktop/bin/pam-desktop-worker` of the PHP package. The host looks for it in, in order:

1. the path in `PAM_DESKTOP_WORKER`;
2. `vendor/pushinbr/pam-desktop/` inside the project;
3. the project directory and each of its parents, which is how the examples run from a checkout of this repository.

The entry script loads its own autoloader. The examples require the `vendor/autoload.php` of the checkout.

## `pam-desktop.json`

The manifest is optional, and running an application never fails because of it: a missing or malformed file means defaults. These are the keys the host reads to run an application:

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `applicationId` | string | `dev.pam.desktop.app_<folder name>` | Reverse-DNS identifier. It names the per-user directories, the keychain service and, on Linux, the GTK application. |
| `name` | string | `PAM Desktop` | Title of the first window. |
| `version` | string or number | empty | Reported as `DeviceInfo::$appVersion`. |
| `build` | string or number | empty | Reported as `DeviceInfo::$buildNumber`. |
| `window.title` | string | `name` | Title of the first window; wins over `name`. |
| `window.width`, `window.height` | number | 960 × 640 | Initial size in logical units. Both must be present. Each is clamped to 120…16384. |
| `window.minWidth`, `window.minHeight` | number | none | Smallest size the user can give the window. Both must be present. |
| `window.resizable` | boolean | `true` | Whether the user can resize the window. |

An `applicationId` is valid when it has at least two segments separated by dots, each segment starts with a letter, `_` or `-` (not a digit), holds only ASCII letters, digits, `_` and `-`, and the whole id is at most 255 bytes. An invalid id is ignored and the default is used.

```json
{
    "applicationId": "com.example.invoices",
    "name": "Invoices",
    "version": "1.0.0",
    "window": {
        "width": 1100,
        "height": 720,
        "minWidth": 640,
        "minHeight": 480
    }
}
```

`dev` and `run` do not read the `entry` key that the example manifests carry: they start `index.php` of the directory, or the file passed on the command line.

The manifest holds more keys that matter when the application is packaged and updated: `entry`, `description`, `publisher`, `category`, `icon`, `exclude`, `extensions` and the `updates` block. They are described in [Distribution](distribution.md) and [Updates](updates.md).

## `dev` and `run`

| | `dev` | `run` |
| --- | --- | --- |
| Source changes | Restart the PHP worker and keep the window | Not watched |
| Application directories | `.pam-desktop/` inside the project | Per-user directories of the system |
| `PAM_DESKTOP_MODE` seen by PHP | `development` | `production` |
| Errors | Shown with source and call stack | The application reloads by itself, then offers a retry screen |
| The PHP worker exits | The window stays on its last frame until a source changes | The application quits |
| Plain `http://` | Allowed for images and HTTP requests | Refused, except for loopback hosts in HTTP requests |

### Hot reload

In `dev` a background thread checks the project every 350 ms for changed files with the extensions `php`, `pam`, `json`, `css`, `svg`, `png`, `jpg` and `ttf`. It skips directories whose name starts with a dot and the directories `vendor`, `node_modules`, `dist`, `target`, `build`, `storage`, `cache` and `tmp`. When something changed it waits until the tree is quiet, then reloads:

1. The running worker writes a checkpoint and exits.
2. A new worker starts and runs the entry script again.
3. The first frame of the new worker is diffed against the tree that is already on screen, so the window does not flash or reopen.

Module state that belonged to the old worker is dropped: pending module results are discarded, and the menu bar, tray, shortcuts and timers are whatever the new worker registers.

### State preservation

The checkpoint holds the `#[State]` properties of every mounted component, keyed by the component's class and position in the tree. A component of the same class mounted at the same position by the new worker receives those values right before its `mount()` runs.

- Only `#[State]` values are restored. Constructor arguments, services and objects are not.
- A value that the new declaration of the property rejects (you changed its type) is dropped and the property starts fresh.
- The checkpoint is consumed by the reload that follows it. A normal start never restores it.

## Environment variables

Variables you can set before starting the host:

| Variable | Effect |
| --- | --- |
| `PAM_DESKTOP_PHP` | Path of the PHP executable used for the worker. |
| `PAM_DESKTOP_WORKER` | Path of the `pam-desktop-worker` bootstrap. |
| `PAM_DESKTOP_WINDOW` | `WIDTHxHEIGHT`, for example `960x640`. Overrides the size of the first window. |
| `PAM_DESKTOP_DATA_DIR` | Absolute path that replaces the data directory. |
| `PAM_DESKTOP_CACHE_DIR` | Absolute path that replaces the cache directory. |
| `PAM_DESKTOP_STATE_DIR` | Absolute path that replaces the state directory. |
| `PAM_DESKTOP_LAUNCH_URL` | The URL the application was launched with; `Linking::initial()` reports it. |
| `PAM_DESKTOP_DEBUG` | Any value other than empty or `0` logs every pointer press and every event sent to PHP. |
| `PAM_DESKTOP_STRICT` | `1` makes `BuildConfiguration::strict()` true in development as well. |
| `PAM_DESKTOP_FRAME_GC` | `0` leaves PHP's cycle collector on its own schedule instead of running it between frames. |
| `TZ`, `LC_ALL`, `LC_NUMERIC`, `LANG` | Read for `DeviceInfo::$timeZone`, `DeviceInfo::$locale` and currency input formatting. |

A relative path in one of the three directory variables is ignored. Variables that only affect `pam-desktop build` are listed in [Distribution](distribution.md).

Variables the host sets for the worker (read them, do not set them):

| Variable | Value |
| --- | --- |
| `PAM_DESKTOP_PROJECT_ROOT` | The project directory. |
| `PAM_DESKTOP_APPLICATION_ID` | The application id in use. |
| `PAM_DESKTOP_DATA_DIR`, `PAM_DESKTOP_CACHE_DIR`, `PAM_DESKTOP_STATE_DIR` | The directories in use. |
| `PAM_DESKTOP_PLATFORM` | `linux`, `macos` or `windows`; `Platform::os()` returns it. |
| `PAM_DESKTOP_MODE` | `development` or `production`. |
| `PAM_BOOT_METRICS`, `PAM_APPEARANCE_MODE`, `PAM_SYSTEM_APPEARANCE`, `PAM_SYSTEM_DARK`, `PAM_RENDERER_CONTRACT` | The window as it is when PHP starts (size, scale, text scale, appearance) and the protocol range of the host. |

## Where application data lives

`run` uses the conventions of each system. `<id>` is the application id.

| Directory | Linux | macOS | Windows |
| --- | --- | --- | --- |
| Data | `$XDG_DATA_HOME/<id>` or `~/.local/share/<id>` | `~/Library/Application Support/<id>` | `%APPDATA%\<id>` |
| Cache | `$XDG_CACHE_HOME/<id>` or `~/.cache/<id>` | `~/Library/Caches/<id>` | `%LOCALAPPDATA%\<id>\Cache` |
| State | `$XDG_STATE_HOME/<id>` or `~/.local/state/<id>` | `~/Library/Application Support/<id>/State` | `%LOCALAPPDATA%\<id>\State` |

`dev` keeps the three inside the project instead: `.pam-desktop/data`, `.pam-desktop/cache` and `.pam-desktop/state`. Development never writes to the user's profile.

What goes where:

| Directory | Contents |
| --- | --- |
| Data | `storage.json` (key/value storage), `pam-files/` (the file sandbox), `pam-databases/` (SQLite), `secure/` (encrypted values too large for a keychain entry) |
| Cache | `pam-images-v1/` (image cache), `pam-media-v1/` (media cache). Safe to delete at any time. |
| State | `settings.json` (appearance and text scale preferences), navigation state, hot reload checkpoints |

## Next steps

- [Components](components.md): the elements you build an interface from.
- [Styling](styling.md): `Style`, state styles, dark mode.
- [Desktop shell](desktop-shell.md): windows, menus, tray, shortcuts, dialogs.
- [Modules](modules.md): storage, files, HTTP, SQLite.
- [Development](development.md): the error overlay, screenshots, tests.
