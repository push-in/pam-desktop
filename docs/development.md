# Development

This page covers the tools you use while building an application or working on the host: the error overlay, screenshots on a virtual display, the test suites, static analysis, continuous integration and the debugging switches.

Hot reload and state preservation are described in [Getting started](getting-started.md#hot-reload).

## The error overlay

An uncaught error in PHP never leaves a blank window. The worker reports it to the host with its phase, the source around the failing line and the call stack, and the host draws the report itself, above the application. Because the overlay is host chrome, it works even when PHP never produced a first frame.

Every report is also written to the standard error of the host, prefixed with `[pam-desktop]`.

`examples/errors` raises one error of each kind.

### Fatal and non-fatal errors

| Phase | Raised from | Fatal |
| --- | --- | --- |
| Boot | `App::run()` and what runs before the first render | Yes |
| Render | A `render()` method, a theme or the encoder | Yes |
| Event | An element, back, lifecycle or dimension event handler | No |
| Module callback | The callback of a module call that has no failure handler | No |
| Unhandled promise rejection | A rejected promise nobody handled | No |

A fatal error means the screen is stale or was never drawn. A non-fatal error means one handler failed and the interface on screen is still the last good frame.

### In development (`pam-desktop dev`)

A **non-fatal** error shows a toast at the bottom of the window. The application stays usable. Pressing the toast opens the full report.

A **fatal** error shows the full report over the window:

- the type of the error and its message;
- where it was raised, with the source around the failing line;
- the call stack, with the frames of the framework folded away until you expand them;
- **Dismiss**, **Copy** (the report as plain text, on the clipboard) and **Reload** (restarts the PHP worker).

When several distinct errors are pending, the report steps through them. An error that repeats is counted instead of queued again; at most 50 distinct errors are kept. A dismissed error stays silent until the next reload. Escape dismisses everything while the full report is open.

Fixing the source is enough: the file watcher reloads the application and the overlay clears.

### In production (`pam-desktop run`)

Nothing technical is shown to the user.

- A non-fatal error is logged and otherwise ignored.
- A fatal error makes the host reload the application by itself: after 250 ms, then 500 ms, then 1 s.
- If the application still fails after three reloads, the host shows a plain screen that says something went wrong, with a button to try again.
- A frame that reaches the screen counts as a recovery and resets the attempts.

### Handling errors yourself

`App::onError()` registers a listener that runs for every uncaught error, for example to log it:

```php
use Pam\Desktop\App;
use Pam\Desktop\Diagnostics\RuntimeError;

App::onError(function (Throwable $error, RuntimeError $report): void {
    error_log(sprintf('[%s] %s at %s:%d', $report->phase->value, $report->message, $report->file, $report->line));
});
```

Most module APIs take a failure closure, which keeps an expected failure (no network, a missing file) out of the overlay; see [Modules](modules.md#failures).

## Screenshots on a virtual display

`scripts/shot.sh` runs an application on a private X display and captures its window, so nothing appears on your real screen. It is how the examples are checked.

```bash
cargo build -p pam-desktop-host

scripts/shot.sh examples/counter /tmp/counter.png
scripts/shot.sh examples/controls /tmp/controls.png 520x600 4
```

```text
scripts/shot.sh <app dir> <output.png> [WIDTHxHEIGHT] [seconds] [-- xdotool arguments… [then more…]]
```

| Argument | Default | Meaning |
| --- | --- | --- |
| `<app dir>` | required | The application to run. |
| `<output.png>` | required | Where the capture is written. |
| `WIDTHxHEIGHT` | `960x640` | Size of the window and of the capture. |
| `seconds` | `4` | How long to wait before interacting and capturing. |
| `-- …` | none | `xdotool` commands to run before the capture. |

To interact before the capture, pass `xdotool` commands after `--`. Separate several commands with a literal `then`:

```bash
scripts/shot.sh examples/counter /tmp/clicked.png 960x640 4 -- mousemove 80 150 click 1
scripts/shot.sh examples/controls /tmp/typed.png 520x600 4 -- mousemove 200 290 click 1 then type 11987654321
```

What the script does:

- starts `Xvfb` on display `:99` if it is not running (set `PAM_SHOT_DISPLAY` for another display);
- disables the desktop portals (`GDK_DEBUG=no-portals`, `GTK_USE_PORTAL=0`) so that file dialogs stay on the virtual display; dark mode then comes from the GTK settings, not from the portal;
- gives the application fresh data, cache and state directories, so every capture starts clean;
- runs `target/debug/pam-desktop run <app dir>` (set `PAM_SHOT_MODE=dev` for development mode) with `PAM_DESKTOP_WINDOW` set to the requested size;
- captures the window with ImageMagick's `import`, stops the host and prints the first lines of its log.

It needs `Xvfb`, `xdpyinfo`, `xdotool` and ImageMagick.

### Visual regression

`scripts/visual.sh` captures every example with `shot.sh`, some of them after a click or a scroll, and compares the captures with a baseline:

```bash
cargo build -p pam-desktop-host
scripts/visual.sh record            # on a commit you trust
# change the code, build again
scripts/visual.sh check             # all scenarios, about two minutes
scripts/visual.sh check lists chat  # only these
scripts/visual.sh list
```

`check` prints one line per scenario with the number of pixels that differ and exits with an error when any scenario has more than it allows. The capture is kept in `target/visual/current` and an image with the differences highlighted in `target/visual/diff`.

The baseline lives in `target/visual/baseline` and is not committed: fonts, the GTK version and the renderer change pixels, so a baseline is only valid on the machine that recorded it. On one machine the captures are reproducible: two runs of the same build differ in no pixel, except in the scenarios that never stop moving (`motion`, `media`, the spinner of `controls`, the seconds counter of `modules` and the sliding line of `text`), which have a tolerance. The scenarios, their sizes and tolerances are the table at the top of the script.

## Tests

### Rust

```bash
cargo test --locked --workspace
```

- `pam-desktop-wire` and `pam-desktop-engine` test the frame formats, layout and the tree diff.
- `pam-desktop-ui` tests the render core against a backend that records every command (`testing.rs`). They need no display and no PHP.
- `pam-desktop-host` tests the host modules and the command line: SQLite, secure storage, the HTTP client against a loopback server, the application paths, the manifest, `init`, packaging and updates.

The backend of the current system is built with the workspace; the other two compile to nothing. To test two crates with a bound on the parallel build jobs:

```bash
cargo test -j4 -p pam-desktop-ui -p pam-desktop-host
```

### PHP

```bash
php packages/desktop/tests/run.php
```

The PHP suite runs the component core without a host: rendering, the encoder, navigation, lists, gestures, the desktop shell classes and the module APIs against test transports. `composer test --working-dir=packages/desktop` runs the same script.

### Static analysis

```bash
composer install --working-dir=packages/desktop
composer analyse --working-dir=packages/desktop
```

`composer analyse` runs PHPStan with `packages/desktop/phpstan.neon`: level 9 over the paths that file lists. That is a subset of the package, not all of `src`: the classes written for the desktop (`Window`, `Menu`, `Tray`, `Dialog`, `Shortcut`, `Notification`, `Updates`, `UI\Window`, `UI\WebView`, toasts) and the parts of the shared core that are held to that level.

### Formatting and lints

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
```

### Testing your own components

The package ships helpers in `Pam\Desktop\Testing`. `mount()` runs a component through its real lifecycle without a host:

```php
use function Pam\Desktop\Testing\mount;

$counter = mount(new Counter());
$counter->render();

assert(str_contains($counter->text(), 'Count: 0'));

$counter->unmount();
```

`text()` returns the text of the rendered tree; `render()` renders again and returns the root element.

## Continuous integration

Two workflows live in `.github/workflows/`. **Neither has run yet**: what follows is what they are written to do.

### `ci.yml`

Runs on pushes to `main`, on pull requests, by hand, and as a job of the release workflow.

The Linux job (Ubuntu 24.04, PHP 8.5, `libgtk-4-dev`, `libwebkitgtk-6.0-4`, `gstreamer1.0-plugins-good`, `xvfb`, `xdotool`) runs these checks, which you can run locally in the same order:

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo check --locked -p pam-desktop-host --no-default-features

composer validate --strict composer.json
composer validate --strict packages/desktop/composer.json
composer install --working-dir=packages/desktop --no-interaction --prefer-dist
composer test --working-dir=packages/desktop
composer analyse --working-dir=packages/desktop

sh -n scripts/package-host.sh scripts/test-host-cache.sh
php -l packages/desktop/bin/pam-desktop
scripts/test-host-cache.sh
```

It then runs `composer install` at the root, builds the host, starts `examples/counter` under `xvfb-run` and waits for a window named "PAM Desktop counter", and packs the host twice with `scripts/package-host.sh` to check that both archives are identical.

A second job runs on a macOS runner and on a Windows runner: `cargo clippy`, `cargo test --locked --workspace`, `cargo build --locked -p pam-desktop-host` and the PHP suite. It starts no application.

### `release.yml`

Runs when a tag `v*` is pushed:

1. Fails unless the tag is the version in `Cargo.toml` and the `HOST_VERSION` of `packages/desktop/bin/pam-desktop`.
2. Builds the host in release mode for `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin`, `x86_64-apple-darwin` and `x86_64-pc-windows-msvc`. The Linux hosts are built on Ubuntu 24.04.
3. Packs each one with `scripts/package-host.sh` and attests the archive and its checksum.
4. Runs the checks of `ci.yml` beside the builds.
5. Once the checks and every build passed, publishes `pam-desktop-<version>-<target>.tar.gz` and its `.sha256` on the GitHub release of the tag. These are the files `vendor/bin/pam-desktop` downloads.

### Packing a host by hand

```bash
scripts/package-host.sh <host binary> <output directory> <version> <target>
```

The script writes `pam-desktop-<version>-<target>.tar.gz` and its `.sha256` in the output directory. The archive holds `pam-desktop-<version>-<target>/bin/pam-desktop` (`pam-desktop.exe` for a Windows target) next to `LICENSE` and `NOTICE`, with fixed owners, modes and dates (`SOURCE_DATE_EPOCH`, zero by default), so the same binary always gives the same archive. A binary that runs on the build machine must report the version it is packed as.

`scripts/test-host-cache.sh` checks how the launcher prunes its cache: it keeps the host of the installed version and the two most recent other versions, and removes download directories left behind for more than a day.

## Debugging switches

| Switch | Effect |
| --- | --- |
| `PAM_DESKTOP_DEBUG=1` | Logs every pointer press and every event sent to PHP on the host's standard error. |
| `PAM_DESKTOP_WINDOW=800x600` | Forces the size of the first window. |
| `PAM_DESKTOP_STATE_DIR`, `PAM_DESKTOP_DATA_DIR`, `PAM_DESKTOP_CACHE_DIR` | Absolute paths that isolate a run from the application's real files. |
| `PAM_DESKTOP_PHP=/path/to/php` | Runs the worker with another PHP executable. |
| `PAM_DESKTOP_WORKER=/path/to/pam-desktop-worker` | Runs another copy of the worker bootstrap. |
| `PAM_DESKTOP_STRICT=1` | Makes `BuildConfiguration::strict()` true in development. |
| `PAM_DESKTOP_FRAME_GC=0` | Leaves PHP's cycle collector on its own schedule. |
| `PAM_DESKTOP_FULL_PRESENT=1` | Windows backend: presents every frame whole instead of the changed regions. |
| `PAM_DESKTOP_BACKDROP_BLUR=0` | Windows backend: paints views with `backdropBlur` without the blur. |
| `GTK_USE_PORTAL=0` | Linux: dark mode is read from the GTK settings instead of the desktop portal. |
| `PAM_DESKTOP_HOST=/path/to/pam-desktop` | `vendor/bin/pam-desktop` runs that host instead of the downloaded one. |

Other things that help:

- **Standard error is the log.** PHP warnings, `echo` and `var_dump` output, `error_log()` and the host's own `[pam-desktop]` messages all appear where you started `pam-desktop`.
- **A rejected frame is reported, not fatal.** `[pam-desktop] frame rejected: …` followed by a complete re-render points at an encoding problem.
- **Development state lives in the project.** Delete `.pam-desktop/` to start from nothing: stored values, databases, caches, the navigation stack and the appearance preference.
- `pam-desktop --version` prints the version of the host.

## Related

- [Getting started](getting-started.md) for `dev`, `run` and the environment.
- [Architecture](architecture.md) for how errors and frames travel between the two processes.
- [Platforms](platforms.md) for cross-checking the macOS and Windows backends.
