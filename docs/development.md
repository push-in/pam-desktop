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
| Worker | A PHP fatal error that ends the worker: memory or time limit | Yes |

A fatal error means the screen is stale or was never drawn. A non-fatal error means one handler failed and the interface on screen is still the last good frame.

### In development (`pam-desktop dev`)

A **non-fatal** error shows a toast at the bottom of the window. The application stays usable. Pressing the toast opens the full report.

A **fatal** error shows the full report over the window:

- a title for what failed: **Boot error**, **Render error**, **Template error** (a template that does not compile), **Syntax error** (PHP that does not parse), **PHP worker stopped**, and so on;
- the type of the error and its message;
- where it was raised, with the source around the failing line, in the `.pam.php` file you wrote (never in the compiled cache);
- the call stack, with the frames of the framework folded away until you expand them;
- **Dismiss**, **Copy** (the report as plain text, on the clipboard) and **Reload** (restarts the PHP worker).

When several distinct errors are pending, the report steps through them. An error that repeats is counted instead of queued again; at most 50 distinct errors are kept. A dismissed error stays silent until the next reload. Escape dismisses everything while the full report is open.

An error raised inside a template, by a method an expression calls or by a component, is reported as that error, where it was raised: its type, its message and its line, with the template line as the next frame of the stack. An error of the template itself (an expression that does not parse, an unknown component) points at the template line and column.

When the PHP worker ends (a fatal error, `exit()`, a crash), the window keeps its last frame and the report says so: a fatal error shows its message and line; otherwise **PHP worker stopped** says what to do. Saving a file or pressing **Reload** starts a new worker.

Fixing the source is enough: the file watcher reloads the application and the overlay clears, in about a quarter of a second.

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

The baseline lives in `target/visual/baseline` and is not committed: fonts, the GTK version and the renderer change pixels, so a baseline is only valid on the machine that recorded it. On one machine the captures are reproducible: two runs of the same build differ in no pixel. The exceptions have a tolerance: the scenarios that never stop moving (`motion`, `media`, the spinner of `controls`, the seconds counter of `modules`, the sliding line of `text`) and `images`, whose status line names the picture the network delivered last. The scenarios, their sizes and tolerances are the table at the top of the script.

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

### Template diagnostics

When a component loads, its template is checked against what the renderer uses. An attribute that a built-in tag (`View`, `Column`, `Row`, `Grid`, `Text`, `Span`, `Pressable`, the `Touchable*` tags, `Icon`, `Image`) does not know is an error with the file, line and the attribute meant, instead of being ignored:

```text
PAM2401 unknown attribute Text.color, did you mean textColor? at src/Screens/Card.pam.php:12:5.
```

Common slips are named directly (`color` → `textColor`, `maxLines` → `numberOfLines`, `src` → `source`, `onClick` → `on:press`); other names get the closest known attribute. In `pam-desktop dev` the error overlay shows it at boot, and the PHP tests of an app fail on it. Bindings (`:name`), `p-*`, `on:*`, `@*`, `data-*`, `aria-*` and `variant:*` are not checked here; components with a contract check their own props.

`vendor/bin/pam-desktop check` reports all of these without starting the host or opening a window, and does not stop at the first one. It runs the entry file of the application (the `entry` of `pam-desktop.json`, `index.php` by default) up to `App::run()`, so every `App::components()` root it declares is checked (DesktopUI's too), compiles each component into a private temporary cache (the application's `.pam-desktop` cache is left alone) and prints one line per diagnostic, then a summary:

```text
$ vendor/bin/pam-desktop check
src/Screens/Badge.pam.php:21:1 PAM2403 Expected a native numeric CSS value in src/Screens/Badge.pam.php, got banana
src/Screens/Profile.pam.php:17:9 PAM2401 unknown attribute Text.color, did you mean textColor?
src/Screens/Profile.pam.php:18:9 PAM2401 action save does not exist on App\Screens\Profile
pam-desktop check: 3 diagnostics (12 components in 2 roots).
```

| Code | Diagnostic |
| --- | --- |
| `PAM2401` | Template contract: unknown action, prop, event or built-in attribute, missing required prop. A language 1 template may call any method, so there only a missing one is reported; language 2 also needs `#[Action]`. |
| `PAM2402` | Template or component compile error. |
| `PAM2403` | Invalid CSS in a `<style>` block, `app.css` or an imported stylesheet (the first one of each component). |
| `PAM2404` | Route error from `routes.php`: unknown route, missing or mistyped parameter in a literal `navigate()`, `redirect()` or `<Link>`. |
| `PAM2405` | Template expression that cannot run: a syntax error (`{{ $count + }}`), or a bare call to a method the component does not have or that is not public (`p-if="missing()"`). Calls on values (`$row->label()`) are only known at runtime. |
| `PAM2400` | The entry file failed before `App::run()`. |

It exits 0 when there is nothing to report and 1 otherwise, so a CI step can run it before the slower suites. `--json` prints `{"ok", "components", "roots", "diagnostics": [{"file", "line", "column", "code", "message"}]}` instead; `--src=<dir>` (repeatable) checks those directories instead of the entry's roots, and `--entry=<file>` names another entry file.

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

sh -n scripts/package-host.sh scripts/test-host-cache.sh scripts/check-glibc.sh
php -l packages/desktop/bin/pam-desktop
scripts/test-host-cache.sh
```

It then runs `composer install` at the root, builds the host, starts `examples/counter` under `xvfb-run` and waits for a window named "PAM Desktop counter", and packs the host twice with `scripts/package-host.sh` to check that both archives are identical.

The Linux baseline job runs on Ubuntu 22.04 (glibc 2.35, GTK 4.6, the oldest supported system): `cargo clippy` and `cargo test` of the GTK backend and the host, a host build, and `scripts/check-glibc.sh target/debug/pam-desktop 2.35`, which fails when the host needs a newer glibc. The backend is built against GTK 4.6 (`gtk4` feature `v4_6`); a GTK function newer than that is declared in `crates/pam-desktop-gtk/src/compat.rs`, looked up at run time, and needs a GTK 4.6 fallback.

A second job runs on a macOS runner and on a Windows runner: `cargo clippy`, `cargo test --locked --workspace`, `cargo build --locked -p pam-desktop-host` and the PHP suite. It starts no application.

### `release.yml`

Runs when a tag `v*` is pushed:

1. Fails unless the tag is the version in `Cargo.toml` and the `HOST_VERSION` of `packages/desktop/bin/pam-desktop`.
2. Builds the host in release mode for `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin`, `x86_64-apple-darwin` and `x86_64-pc-windows-msvc`. The Linux hosts are built on Ubuntu 22.04 (glibc 2.35, GTK 4.6), the oldest supported system, and the build fails if a host needs a newer glibc.
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
| `PAM_DESKTOP_PROFILE=1` | Times each interaction on standard error: the PHP handler and render of every event and flush, the host's layout of each committed frame, the native view work it caused and the frame painted after it (`[pam-desktop profile] @<ms> ...`, the wall clock modulo 100 s lines worker and host up). GTK lists the heaviest backend calls (`create_view 1.3×120`: milliseconds × calls). |
| `PAM_DESKTOP_DEBUG=1` | Logs every pointer press, every key press (with its modifiers and the focused node) and every event sent to PHP on the host's standard error. |
| `PAM_DESKTOP_WINDOW=800x600` | Forces the size of the first window. |
| `PAM_DESKTOP_STATE_DIR`, `PAM_DESKTOP_DATA_DIR`, `PAM_DESKTOP_CACHE_DIR` | Absolute paths that isolate a run from the application's real files. |
| `PAM_DESKTOP_PHP=/path/to/php` | Runs the worker with another PHP executable. |
| `PAM_DESKTOP_CAPTURE=/path/shot.png` | macOS: writes the content of the first visible window to a PNG `PAM_DESKTOP_CAPTURE_AFTER` seconds (3 by default) after it shows. It renders the layer tree in the process, so it needs no screen recording permission and works on a locked session, but leaves out layer shadows, masks and Core Image filters. |
| `PAM_DESKTOP_WORKER=/path/to/pam-desktop-worker` | Runs another copy of the worker bootstrap. |
| `PAM_DESKTOP_STRICT=1` | Makes `BuildConfiguration::strict()` true in development. |
| `PAM_DESKTOP_FRAME_GC=0` | Leaves PHP's cycle collector on its own schedule. |
| `PAM_DESKTOP_TEXT_GAMMA` | GTK: `0` draws text with GSK glyphs instead of the default Chromium glyph atlas. macOS: `1` draws glyphs as Chromium's Skia masks (opt-in). See [Text rendering](text-rendering.md). |
| `PAM_DESKTOP_FULL_PRESENT=1` | Windows backend: presents every frame whole instead of the changed regions. |
| `PAM_DESKTOP_BACKDROP_BLUR=0` | Windows backend: paints views with `backdropBlur` without the blur. |
| `GTK_USE_PORTAL=0` | Linux: dark mode is read from the GTK settings instead of the desktop portal. |
| `PAM_DESKTOP_HOST=/path/to/pam-desktop` | `vendor/bin/pam-desktop` runs that host instead of the downloaded one. |
| `PAM_DESKTOP_DUMP_FRAMES=/path/frames.json` | Writes every node's frame to a JSON file after each layout change; see [Layout parity with a web build](#layout-parity-with-a-web-build). `{window}` in the path becomes the window id. Off by default, at no cost. |

Other things that help:

- **Standard error is the log.** PHP warnings, `echo` and `var_dump` output, `error_log()` and the host's own `[pam-desktop]` messages all appear where you started `pam-desktop`.
- **A rejected frame is reported, not fatal.** `[pam-desktop] frame rejected: …` followed by a complete re-render points at an encoding problem.
- **Development state lives in the project.** Delete `.pam-desktop/` to start from nothing: stored values, databases, caches, the navigation stack and the appearance preference.
- `pam-desktop --version` prints the version of the host.

## Layout parity with a web build

When a desktop screen reimplements a web or Electron page, a screenshot diff says that pixels differ but not which box moved. Two tools compare the boxes themselves.

1. **Native frames.** Start the application with `PAM_DESKTOP_DUMP_FRAMES=/tmp/frames.json`. After each layout change the host rewrites the file (atomically) with one entry per node:

   ```json
   {"window": 1, "scale": 1.0, "nodes": [
     {"id": 7, "parent": 3, "kind": "Text", "testId": "", "label": "", "text": "Clientes",
      "layout": [245.0, 298.421875, 821.046875, 24.0], "content": [245.0, 298.421875, 821.046875, 24.0],
      "painted": [245.0, 298.0, 821.0, 24.0]}
   ]}
   ```

   `layout` is the frame in window coordinates on the 1/64 px grid Chromium uses (LayoutUnit), `content` the box inside a text's padding and border, and `painted` the frame snapped to device pixels as it is drawn. Scrolled content is in window coordinates at its current offset. Without the variable nothing is written and nothing is computed.

2. **Chromium boxes.** `scripts/parity/electron-dom.cjs` loads a page in Electron and writes the `getBoundingClientRect()` box of every element that holds text (its border box and its content box) with `scripts/parity/dom-rects.js`, which also works in Playwright or Puppeteer:

   ```sh
   xvfb-run -a npx electron scripts/parity/electron-dom.cjs "http://127.0.0.1:5173/#/orders" /tmp/dom.json --size 1200x800 --wait 3000
   ```

   Requests outside the page's origin are blocked and the pointer is parked in a corner, so hover styles stay off. Serve the web build with the same data the desktop build uses (a fixture answered by the HTTP layer works well).

3. **Compare.** `scripts/parity/compare_frames.py /tmp/frames.json /tmp/dom.json` matches texts in reading order (case-insensitively, so `text-transform` does not matter) and prints, for each pair, how far every edge is from Chromium's and marks with `*` the edges that land on another device pixel, which is what a screenshot comparison sees. It exits with 1 while any edge is marked, so it can gate a check.

   ```text
   'Ana Silva'      native y= 401.9219 h= 15.0000  chromium y= 402.4219 h= 15.0000  top=-0.500 bottom=-0.500
   'RISCO BAIXO'    native y= 657.4219 h= 16.0000  chromium y= 657.9219 h= 16.0000  top*=-0.500 bottom*=-0.500
   ```

   Useful options: `--x-min 216` ignores a shared sidebar, `--edges top,bottom` checks only vertical edges (a native text often spans its column while the web element is shrink-wrapped, so its left and right edges differ without any pixel moving), `--dom-box border` compares border boxes instead of content boxes, `--all` lists equal pairs too.

   A difference that starts at one text and continues below it points at the box right above: in the example the rows of a `<table>` start half a pixel lower in Chromium because a collapsed border is shared half and half between rows.

## Related

- [Getting started](getting-started.md) for `dev`, `run` and the environment.
- [Architecture](architecture.md) for how errors and frames travel between the two processes.
- [Platforms](platforms.md) for cross-checking the macOS and Windows backends.
