<div align="center">

# Pam Desktop

### PHP in control. Rust at the boundary. Servo on screen.

A direct Servo host for building native desktop applications whose application
logic remains elegant, typed PHP.

![Version](https://img.shields.io/badge/version-0.4.0-68ded2?style=flat-square)
![Status](https://img.shields.io/badge/status-alpha-f59e0b?style=flat-square)
![Servo](https://img.shields.io/badge/Servo-0.4.0-5b50d6?style=flat-square)
![PHP](https://img.shields.io/badge/PHP-8.4-777BB4?style=flat-square&logo=php&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-1.88%2B-000000?style=flat-square&logo=rust&logoColor=white)
![License](https://img.shields.io/badge/license-MIT-22c55e?style=flat-square)

</div>

---

Pam Desktop is intentionally separate from the Pam server core. This repository
owns the native window, Servo integration, secure local bridge, shared protocol,
and the `pam/desktop` Composer package. Pam remains the PHP worker runtime.

Version 0.4 adds a distributable Linux application contract:

- local HTML, CSS, JavaScript and assets rendered directly by Servo;
- explicit commands and bidirectional events through `window.pam`;
- timeouts, `AbortSignal` cancellation, crash detection and worker recovery;
- multiple independent Servo/Winit windows with targeted effects;
- typed, immutable window configuration, events and effects in PHP;
- development hot reload for assets, PHP and Composer changes;
- PHP-declared filesystem roots with read/write policy;
- native dialogs, clipboard and notifications behind independent permissions;
- opaque, process-lifetime grants for selected and dropped files;
- typed reverse-DNS application manifests, integer-backed categories and
  validated PNG/SVG icons;
- atomic self-contained Linux bundles with Pam, PHP libraries, the Servo host,
  vendored application code and per-file SHA-256 integrity metadata;
- portable `.tar.gz` distribution, per-user install/uninstall scripts and
  optional native `.deb` packages;
- one supervised Pam worker with bounded, versioned JSON-lines messages;
- a random loopback gateway with origin and token enforcement;
- no Node runtime and no unrestricted ambient native API.

Pam Desktop currently targets controlled prototypes and product exploration.
Servo 0.4 is still evolving, so this is not yet a drop-in Electron replacement.

## Create an application

From a Pam checkout that has this repository beside it:

```bash
pam init hello-desktop --template desktop
cd hello-desktop
pam desktop doctor .
pam desktop dev .
pam desktop build .
```

The generated Hello World demonstrates commands, PHP-to-JavaScript events,
timeouts, hot-reload status, a second Runtime Inspector window and a Native Lab
for authorized filesystem, dialogs, clipboard, notifications and dropped
files. The public experience stays under `pam desktop`; `pam-desktop` is the
internal host binary.

Application code stays compact:

```php
<?php

declare(strict_types=1);

use Pam\Desktop\Application;
use Pam\Desktop\ApplicationCategory;
use Pam\Desktop\Capabilities;
use Pam\Desktop\ClientEvent;
use Pam\Desktop\CommandContext;
use Pam\Desktop\CommandResult;
use Pam\Desktop\FileSystemRoot;
use Pam\Desktop\Manifest;
use Pam\Desktop\Window;
use Pam\Desktop\WindowEffect;
use Pam\Desktop\WindowTheme;

$app = Application::create(
    window: Window::create('My desktop app')
        ->size(1120, 720)
        ->minimumSize(720, 520)
        ->theme(WindowTheme::Dark),
    manifest: Manifest::create('com.pushin.my-app', 'My desktop app', '0.4.0')
        ->description('A PHP-first native desktop application.')
        ->publisher('My team')
        ->category(ApplicationCategory::Development),
)
    ->window(
        'settings',
        Window::create('Settings')
            ->entry('resources/settings.html')
            ->visible(false),
    )
    ->capabilities(
        Capabilities::none()
            ->filesystem(FileSystemRoot::readWrite('data', __DIR__.'/storage'))
            ->dialogs()
            ->clipboard()
            ->notifications()
            ->dragAndDrop(),
    )
    ->commandTimeout(10_000);

$app->command('greet', static function (CommandContext $command): CommandResult {
    $name = $command->string('name', 'mundo');

    return CommandResult::success(['message' => "Olá, {$name}."])
        ->effect(WindowEffect::title("Olá, {$name}", $command->windowId))
        ->event(new ClientEvent(
            name: 'greeting.completed',
            payload: ['name' => $name],
            windowId: $command->windowId,
        ));
});

$app->run();
```

In the trusted local frontend:

```js
window.pam.on("greeting.completed", ({ name }) => {
    console.log(`Event received for ${name}`);
});

const result = await window.pam.invoke(
    "greet",
    { name: "David" },
    { timeout: 5_000 },
);
console.log(result.message);

await window.pam.fs.writeText(
    { root: "data", path: "greeting.txt" },
    result.message,
);

const selected = await window.pam.dialog.openFile({
    filters: [{ name: "Text", extensions: ["txt", "md"] }],
});
if (selected) {
    console.log(await window.pam.fs.readText(selected));
}
```

`window.pam.emit(name, payload, options)` sends a typed application event to
PHP. Both `invoke` and `emit` accept `timeout` and `signal`; cancellation is
forwarded to the host, the compromised worker is terminated, and a fresh worker
is prepared for the next request without replaying the interrupted command.

Native capabilities default to disabled. Files outside named roots are exposed
only after an explicit operating-system dialog or drag and drop. The host
returns an opaque `grantId`, never the ambient filesystem path. Read the
[Capabilities guide](docs/capabilities.md) for the complete frontend API,
integer contracts and limits.

## Package for Linux

The application manifest stays beside windows and capabilities in PHP. Build
the default directory and portable archive atomically:

```bash
pam desktop build .
```

Additional formats and output control:

```bash
pam desktop build . --output dist --format deb
pam desktop build . --format all --force
```

The portable archive contains `install.sh` and `uninstall.sh` for a per-user
installation. Debian packaging requires `dpkg-deb`. Every bundle contains a
`manifest.json` with protocol, application, runtime and target metadata plus
the byte size and SHA-256 digest of every shipped file. See the
[Linux distribution guide](docs/distribution.md).

## Build the host

The Rust workspace pins Servo 0.4.0 and carries a committed `Cargo.lock`.
On Ubuntu 22.04 or compatible distributions:

```bash
sudo apt-get install -y \
  build-essential clang libclang-dev libfontconfig1-dev pkg-config
cargo build --locked --release -p pam-desktop
```

Install a development binary:

```bash
cargo install --locked --path crates/pam-desktop-shell
```

Pam and the internal `pam-desktop` host must both be available on `PATH` or
installed beside each other. `pam desktop` locates the host and passes its own
absolute path as `PAM_BINARY`. Development overrides remain available through
`PAM_DESKTOP_BINARY` and `PAM_BINARY`.

## Repository map

```text
crates/
├── pam-desktop-protocol/  shared Rust contracts and integer discriminators
└── pam-desktop-shell/     Servo, Winit, gateway and Pam worker supervision
packages/
└── desktop/               public PHP application API and worker loop
docs/
├── architecture.md        process, security and extension boundaries
├── capabilities.md        PHP policy and frontend native APIs
└── distribution.md        Linux bundles, manifests and installers
```

Read [Architecture](docs/architecture.md) before expanding native capabilities.

## Validate

```bash
cargo fmt --all -- --check
cargo test --locked -p pam-desktop-protocol
cargo test --locked -p pam-desktop --no-default-features --features gateway
cargo check --locked -p pam-desktop
composer test --working-dir=packages/desktop
composer analyse --working-dir=packages/desktop
composer validate --strict packages/desktop/composer.json
```

## Roadmap

| Version | Delivery |
| --- | --- |
| **0.2** | **Events, deadlines, cancellation, crash recovery, multiple windows and hot reload — implemented** |
| **0.3** | **Authorized filesystem, dialogs, clipboard, notifications and drag and drop — implemented** |
| **0.4** | **Self-contained Linux build, icons, manifest and installers — implemented** |
| 0.5 | Windows and macOS, signing and automatic updates |
| 0.6 | PHP/Rust plugins, menus, tray, global shortcuts and background jobs |
| 1.0 | Stable API, compatibility suite and multi-platform distribution |

## License

Pam Desktop is MIT licensed. Servo and transitive dependencies retain their own
licenses.
