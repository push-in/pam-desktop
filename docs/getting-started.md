# Getting started

This page takes you from an empty directory to a running application, then explains the project layout, the `pam-desktop.json` manifest, the two run modes and where an application keeps its files.

## Requirements

- PHP 8.5 and Composer.
- On Linux, to run a released host: glibc 2.35 or newer, GTK 4.6 or newer and the GStreamer plugins for media (Ubuntu 22.04 and later); WebKitGTK 6 (`libwebkitgtk-6.0-4`, Ubuntu 24.04 and later) only for applications that show a `WebView`. The released host is built on Ubuntu 22.04.
- To build the host from a checkout instead: a Rust toolchain, 1.92 or newer (`rust-toolchain.toml` selects the stable channel) and, on Linux, `libgtk-4-dev`.

[Platforms](platforms.md) has the details for each system.

## A new project

```bash
mkdir my-app && cd my-app
composer require pushinbr/pam-desktop
vendor/bin/pam-desktop init
pam desktop dev            # or: vendor/bin/pam-desktop dev, or: composer dev
```

A window opens on the home screen of the new application. Edit `src/Screens/Home/HomeScreen.pam.php` while it runs: the window updates in about a third of a second and keeps its state (the click count survives the reload).

`dev` needs no argument: it runs the project of the current directory, found by walking up to the nearest `pam-desktop.json`, so it works from any folder of the project (`src/Screens/Home` included). `pam desktop ...` is the same command through the [PAM CLI](https://github.com/push-in/pam), which registers it from the installed package. It also starts PHP for you: when no PHP is installed, the host installs the PAM runtime, which embeds PHP 8.5 (see [How the host finds PHP](#how-the-host-finds-php-and-the-package)).

`vendor/bin/pam-desktop` is the launcher of the PHP package. It runs the native host with the arguments it was given:

- The first time, it downloads `pam-desktop-<version>-<target>.tar.gz` from the GitHub release of the installed version, verifies it against the published SHA-256 and keeps the host in the cache of the user: `$XDG_CACHE_HOME/pam-desktop` or `~/.cache/pam-desktop` on Linux, `~/Library/Caches/pam-desktop` on macOS, `%LOCALAPPDATA%\pam-desktop` on Windows. The first installation needs the Phar extension of PHP.
- `PAM_DESKTOP_HOST` is the absolute path of a host binary to run instead, with no download.
- A release has archives for Linux and macOS on x86_64 and arm64 and for Windows on x86_64.

Version 2.0.0 is not released yet: there is no `v2.0.0` tag for Composer to install and no host archive to download. Until there is, work from a checkout, as described below.

### `pam-desktop init`

```text
pam-desktop init [directory] [--name NAME] [--id APPLICATION-ID]
```

`init` writes a small, ready application in the current directory or in `directory` (created when missing):

```text
my-app/
  pam-desktop.json                     identity, window and packaging
  composer.json                        PHP 8.5, pushinbr/pam-desktop; scripts dev, start, build, test
  index.php                            boots the router
  README.md
  .gitignore                           vendor, .pam-desktop and dist
  src/
    app.css                            design tokens (light and dark) shared by every screen
    Navigation/AppRoute.php            the routes, as an enum
    Navigation/AppRouter.php           the stack and the screen of each route
    Screens/Home/HomeScreen.pam.php    the home screen: class, <template> of tags and scoped <style>
    Services/Greeting.php              application logic the screens call
    Lib/Plural.php                     a small shared helper
  tests/HomeScreenTest.php             mounts the home screen without a window (composer test)
```

Each screen is one `.pam.php` file: the PHP class with its state and actions, then a `<template>` written with tags (`<Column>`, `<Text>`, `<Button on:press="increment">`, `{{ $title }}`) and a `<style scoped>` block. The skeleton lives in `packages/desktop/resources/skeleton` of the package.

Template expressions, including method calls on values (`{{ $order->status->label() }}`), are described in [Templates](templates.md#expressions).

A file that already exists is kept: the command prints `[new]` or `[kept]` for each one, so it is safe to run in a directory where Composer already wrote `composer.json`. Without `--name` the name is the name of the directory; without `--id` the identifier is `com.example.<directory name>`, in lower case with dashes. An identifier that [`build`](distribution.md) would refuse stops the command.

## From a checkout

To work on PAM Desktop itself, or to run the examples, build the host from the repository:

```bash
git clone https://github.com/push-in/pam-desktop.git
cd pam-desktop
composer install
cargo build -p pam-desktop-host
target/debug/pam-desktop dev examples/counter
```

`composer install` writes the `vendor/autoload.php` that the examples load. A project of your own uses that host through the launcher:

```bash
PAM_DESKTOP_HOST=/path/to/pam-desktop/target/debug/pam-desktop vendor/bin/pam-desktop dev
```

## The command line

```text
pam-desktop init [directory]          write the files of a new application
pam-desktop dev [app] [url]           run with hot reload (default: current directory)
pam-desktop run [app] [url]           run the application, opened with a link
pam-desktop build [app] [options]     bundle the application and build its installers
pam-desktop update-key --output FILE  create the key that signs updates
pam-desktop publish-update [app] ...  sign the update feed of a release
pam-desktop --version
```

`vendor/bin/pam-desktop` takes the same commands. This page covers `init`, `dev` and `run`. `build` is described in [Distribution](distribution.md); `update-key` and `publish-update` in [Updates](updates.md).

`app` is a project directory or a PHP file. With a directory the entry script is the `entry` of its manifest, `index.php` by default. With a file, the file is the entry script and its directory is the project. Without `app`, `dev`, `run`, `build` and `publish-update` take the project of the current directory: the nearest directory, from it up, that holds a `pam-desktop.json`, else the current directory.

`url` is the link the application is opened with: `Linking::initial()` reports it. See [Modules](modules.md#linking).

Exit codes of `dev` and `run`:

| Exit code | Meaning |
| --- | --- |
| 64 | Unknown command (the usage text is printed). |
| 66 | The entry script does not exist (outside a project, the message says no `pam-desktop.json` was found). |
| 69 | The worker bootstrap of the `pushinbr/pam-desktop` package was not found. |
| 73 | The application directories could not be created. |
| 1 | The PHP worker could not be started, or the host was built for a system other than Linux, macOS and Windows. |
| 0 | The application ended, or it is a [single instance](distribution.md#links-and-single-instance) that already runs and was handed the link. |

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
  .gitignore          written by `pam-desktop init`
  vendor/             Composer autoloader and the pushinbr/pam-desktop package
  assets/             files the application loads with asset:// URLs
  .pam-desktop/       created by `pam-desktop dev`: data, cache and state
```

Only the entry script is required to run. The project directory is:

- the working directory of the PHP worker;
- the root that `asset://` URLs resolve in (`asset://assets/logo.png` is `assets/logo.png` in the project);
- the directory watched by `dev`.

### How the host finds PHP and the package

The host starts the worker as `<php> <bootstrap> <entry script>`.

`<php>` is, in order: the path in `PAM_DESKTOP_PHP`; `pam` found on `PATH` or where the PAM installer links it (`$XDG_BIN_HOME/pam`, else `~/.local/bin/pam`), started as `pam exec`; `php` found on `PATH`. On Linux and macOS, when none of them exists, the host can install the PAM runtime, which embeds PHP 8.5, and use it: `pam-desktop dev` works on a machine without PHP.

Nothing is installed without consent. In a terminal the host shows what it will download, from where and into which folder, and asks `Install it now? [y/N]`. Scripts and CI agree beforehand with `pam-desktop dev --yes` (also `run --yes`) or `PAM_DESKTOP_AUTO_INSTALL=1`; without a terminal and without either, the host stops with the ways to install PHP yourself.

The PAM runtime is not installed by piping a script into `sh`. The host reads the latest release tag (or the one in `PAM_DESKTOP_PAM_VERSION`, for example `v2.2.9`), downloads that release's update manifest, the document `pam self-update` trusts, and checks its Ed25519 signature against the PAM release key, whose SHA-256 is pinned in the host (`runtime_install::SIGNING_IDENTITIES`); the release, the target system, the archive type and, for the latest release, the signed freshness window (at most 31 days) must match. Then it downloads `pam-<version>-<target>.tar.gz` and accepts it only with the byte length and SHA-256 the manifest signs. The archive is unpacked in Rust (only regular files and folders below the release folder, at most 100,000 entries and 4 GiB, no owners or setuid bits), `bin/pam-run --version` must name the verified release, and it is installed the way the PAM installer does it (`$PAM_INSTALL_DIR`, else `$XDG_DATA_HOME/pam`, else `~/.local/share/pam`, `<version>-<target>`), with `pam` linked at `$XDG_BIN_HOME/pam` or `~/.local/bin/pam`, replacing only a symlink. Downloads use the host's HTTPS stack (system trust store); `curl`, `tar` and `sh` are not used. When the latest manifest has expired (PAM published nothing for a month), name a release with `PAM_DESKTOP_PAM_VERSION`: an explicitly chosen release is checked the same way, except for the freshness window.

PAM has no Windows release. On Windows `<php>` is the path in `PAM_DESKTOP_PHP`, the PHP that PAM Desktop installed earlier, or `php.exe` on `PATH`; when there is none, the host asks as above, then downloads the newest official PHP 8.5 build (non thread safe, x64) from windows.php.net and installs it under `%LOCALAPPDATA%\PAM Desktop\php\<version>`. The archive must match the SHA-256 that windows.php.net publishes in `releases.json` and the SHA-256 PAM Desktop pins for that build (`windows_php::PINNED`), since `releases.json` comes from the same server as the archive. When windows.php.net lists a newer build than this host knows, the newest pinned build is installed from its `archives/` folder instead; `PAM_DESKTOP_PHP_ALLOW_UNPINNED=1` accepts the unknown build with only the published SHA-256 (and says so). The PHP builds of windows.php.net carry no Authenticode signature, so there is no publisher signature to check. That build needs the Visual C++ Redistributable 2015-2022 (x64). This path has not been run on Windows yet; see [Platforms](platforms.md#windows).

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
| `entry` | string | `index.php` | The script a project directory starts with, as a path inside the project. An unsafe path is ignored. |
| `singleInstance` | boolean | `true` under `run` when `urlSchemes` is not empty, otherwise `false` | A second launch hands its link to the running application and exits. See [Distribution](distribution.md#links-and-single-instance). |
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

The manifest holds more keys that matter when the application is packaged and updated: `description`, `publisher`, `category`, `icon`, `exclude`, `extensions`, `urlSchemes` and the `updates` block. They are described in [Distribution](distribution.md) and [Updates](updates.md).

`packages/desktop/resources/pam-desktop.schema.json` is a JSON Schema of the manifest. The file that `init` writes points its `$schema` at the copy inside `vendor`, so an editor can complete and check the keys.

## `dev` and `run`

| | `dev` | `run` |
| --- | --- | --- |
| Source changes | Restart the PHP worker and keep the window | Not watched |
| Application directories | `.pam-desktop/` inside the project | Per-user directories of the system |
| `PAM_DESKTOP_MODE` seen by PHP | `development` | `production` |
| Errors | Shown with source and call stack | The application reloads by itself, then offers a retry screen |
| The PHP worker exits | The window stays on its last frame with a report that says so, until a source changes or Reload is pressed | The application quits |
| Plain `http://` | Allowed for images and HTTP requests | Refused, except for loopback hosts in HTTP requests |

### Hot reload

In `dev` a background thread checks the project every 150 ms for changed files with the extensions `php`, `pam`, `json`, `css`, `svg`, `png`, `jpg` and `ttf`. It skips directories whose name starts with a dot and the directories `vendor`, `node_modules`, `dist`, `target`, `build`, `storage`, `cache` and `tmp`. When something changed it waits until the tree has been quiet for 80 ms (editors save in several steps), then reloads:

1. The running worker writes a checkpoint and exits.
2. A new worker starts and runs the entry script again.
3. The first frame of the new worker is diffed against the tree that is already on screen, so the window does not flash or reopen.

From the save to the updated window takes about 0.3 s on Linux (measured on the skeleton: 181 to 346 ms, median 315 ms, for a template and for a service), and fixing an error clears its report in 0.2 to 0.25 s. A scan of a few hundred files costs well under a millisecond.

Module state that belonged to the old worker is dropped: pending module results are discarded, and the menu bar, tray, shortcuts and timers are whatever the new worker registers.

### State preservation

The checkpoint holds the `#[State]` properties of every mounted component, keyed by the component's class and position in the tree. A component of the same class mounted at the same position by the new worker receives those values right before its `mount()` runs.

- Only `#[State]` values are restored. Constructor arguments, services and objects are not.
- Mark passwords, secret input and in-flight flags with `#[State(hotReload: false)]`. These properties remain reactive but are never written to the development checkpoint and start with their declared defaults after a reload. For example: `#[State(hotReload: false)] public string $password = '';`.
- A value that the new declaration of the property rejects (you changed its type) is dropped and the property starts fresh.
- The checkpoint is consumed by the reload that follows it. A normal start never restores it.

## Environment variables

Variables you can set before starting the host:

| Variable | Effect |
| --- | --- |
| `PAM_DESKTOP_PHP` | Path of the PHP executable used for the worker. |
| `PAM_DESKTOP_AUTO_INSTALL` | `1` installs PHP without asking when none is found (as `--yes`); for CI and scripts. |
| `PAM_DESKTOP_PAM_VERSION` | The PAM release (`vX.Y.Z`) to install when no PHP is found, instead of the latest one. |
| `PAM_DESKTOP_PHP_ALLOW_UNPINNED` | Windows: `1` accepts a windows.php.net build whose SHA-256 the host does not pin. |
| `PAM_DESKTOP_WORKER` | Path of the `pam-desktop-worker` bootstrap. |
| `PAM_DESKTOP_WINDOW` | `WIDTHxHEIGHT`, for example `960x640`. Overrides the size of the first window. |
| `PAM_DESKTOP_DATA_DIR` | Absolute path that replaces the data directory. |
| `PAM_DESKTOP_CACHE_DIR` | Absolute path that replaces the cache directory. |
| `PAM_DESKTOP_STATE_DIR` | Absolute path that replaces the state directory. |
| `PAM_DESKTOP_LAUNCH_URL` | The URL the application was launched with; `Linking::initial()` reports it. A `url` argument on the command line wins. |
| `PAM_DESKTOP_HOST` | Read by `vendor/bin/pam-desktop` only: the absolute path of the host binary to run, with no download. |
| `GTK_USE_PORTAL` | Linux: `0` keeps the host from asking the desktop portal for the dark mode setting, as it keeps GTK from the portal. |
| `PAM_DESKTOP_DEBUG` | Any value other than empty or `0` logs every pointer press and every event sent to PHP. |
| `PAM_DESKTOP_STRICT` | `1` makes `BuildConfiguration::strict()` true in development as well. |
| `PAM_DESKTOP_FRAME_GC` | `0` leaves PHP's cycle collector on its own schedule instead of running it between frames. |
| `TZ`, `LC_ALL`, `LC_NUMERIC`, `LANG` | Read for `DeviceInfo::$timeZone`, `DeviceInfo::$locale` and currency input formatting. |

A relative path in one of the three directory variables is ignored. `vendor/bin/pam-desktop` keeps the downloaded host in the cache of the user; `PAM_DESKTOP_HOST_CACHE_DIR` names another absolute directory for it. Variables that only affect `pam-desktop build` are listed in [Distribution](distribution.md).

Variables the host sets for the worker (read them, do not set them):

| Variable | Value |
| --- | --- |
| `PAM_DESKTOP_PROJECT_ROOT` | The project directory. |
| `PAM_DESKTOP_APPLICATION_ID` | The application id in use. |
| `PAM_DESKTOP_DATA_DIR`, `PAM_DESKTOP_CACHE_DIR`, `PAM_DESKTOP_STATE_DIR` | The directories in use. |
| `PAM_DESKTOP_PLATFORM` | `linux`, `macos` or `windows`; `Platform::os()` returns it. |
| `PAM_DESKTOP_MODE` | `development` or `production`. |
| `PAM_DESKTOP_LAUNCH_URL` | The link the application was opened with, when there is one. |
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
