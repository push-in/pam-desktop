<div align="center">

# Pam Desktop

### PHP in control. Rust at the boundary. Servo on screen.

A direct Servo host for building native desktop applications whose application
logic remains elegant, typed PHP.

[![Documentation](https://img.shields.io/badge/docs-desktop-5b50d6?style=flat-square)](https://push-in.github.io/pam-docs/desktop/overview/)
![Version](https://img.shields.io/badge/version-1.2.1-68ded2?style=flat-square)
![Status](https://img.shields.io/badge/Linux-stable-22c55e?style=flat-square)
![Servo](https://img.shields.io/badge/Servo-0.4.0-5b50d6?style=flat-square)
![PHP](https://img.shields.io/badge/PHP-8.4-777BB4?style=flat-square&logo=php&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-1.88%2B-000000?style=flat-square&logo=rust&logoColor=white)
![License](https://img.shields.io/badge/license-MIT-22c55e?style=flat-square)

**[Documentation](https://push-in.github.io/pam-docs/desktop/overview/) ·
[Capabilities](https://push-in.github.io/pam-docs/desktop/capabilities/) ·
[Security](https://push-in.github.io/pam-docs/desktop/security/) ·
[Distribution](https://push-in.github.io/pam-docs/desktop/distribution/) ·
[Plugins](https://push-in.github.io/pam-docs/desktop/plugins/) ·
[Contributing](https://push-in.github.io/pam-docs/community/contributing/)**

</div>

---

Pam Desktop is intentionally separate from the Pam server core. This repository
owns the native window, Servo integration, secure local bridge, shared protocol,
and the `pushinbr/pam-desktop` Composer package. Pam remains the PHP worker runtime.

It is a desktop stack with an opinion: PHP should own application logic, Rust
should own the security and process boundary, and Servo should render the local
experience directly. No Electron-sized Node runtime, no ambient filesystem
access, and no pretending that a browser tab is an application architecture.

PAM Desktop pairs expressive authoring with explicit native authority:
capabilities default to off, commands are registered deliberately, file access
uses named roots or opaque grants, plugins run behind versioned contracts, and
releases carry integrity metadata. It is built to feel powerful without making
the security model vague.

Version 1.0 freezes the public PHP, JavaScript and Rust plugin APIs and turns
the Linux host into a reproducible, verifiable release:

- local HTML, CSS, JavaScript and assets rendered directly by Servo;
- explicit commands and bidirectional events through `window.pam`;
- timeouts, `AbortSignal` cancellation, crash detection and worker recovery;
- multiple independent Servo/Winit windows with targeted effects;
- typed, immutable window configuration, events and effects in PHP;
- development hot reload for assets, PHP and Composer changes;
- PHP-declared filesystem roots with read/write policy;
- native dialogs, clipboard and notifications behind independent permissions;
- capability-scoped SQLite with bundled engine, WAL, prepared parameters and atomic transactions;
- authenticated binary file streaming with browser backpressure and no base64 expansion;
- lazy supervised PHP worker pools for explicitly parallel or background commands;
- privacy-bounded Linux system, memory, network, power and battery snapshots;
- opaque, process-lifetime grants for selected and dropped files;
- typed reverse-DNS application manifests, integer-backed categories and
  validated PNG/SVG icons;
- atomic self-contained Linux bundles with Pam, PHP libraries, the Servo host,
  vendored application code and per-file SHA-256 integrity metadata;
- portable `.tar.gz` distribution, per-user install/uninstall scripts and
  optional native `.deb` packages;
- a stable Linux x86-64 host contract and reproducible official archive;
- an XDG-aware, atomic per-user host installer that never invokes `sudo`;
- preserved experimental Windows/macOS packager code, outside the 1.x support
  and release guarantee;
- an immutable PHP update policy with pinned Ed25519 public keys;
- signed update feeds, bounded HTTPS downloads, exact size/SHA-256
  verification and atomic install with one-version rollback;
- frozen `pam.updater` status, check, download and install operations plus
  notify/automatic background policies;
- composable PHP plugins that register commands, events and jobs;
- process-isolated Rust plugins with a versioned SDK, declared exports,
  deadlines, cancellation and crash recovery;
- `pam desktop plugin new/build` for generating and bundling native extensions;
- immutable PHP menu trees, checkbox state, application tray and optional
  close-to-tray behavior;
- global shortcuts with typed press/release events and graceful platform
  fallback;
- supervised PHP background jobs with initial delay, timeout and integer-backed
  skip/wait overlap policy;
- dynamic menu, checkbox and tray-visibility effects;
- one supervised Pam worker with bounded, versioned JSON-lines messages;
- a random loopback gateway with origin and token enforcement;
- no Node runtime and no unrestricted ambient native API.

Pam Desktop 1.x is stable for Linux x86-64 applications built against public
API version 1 and protocol 6. Servo 0.4 is still evolving, so engine behavior
does not imply feature-for-feature Electron compatibility. The PAM contracts
around it are protected by executable compatibility fixtures.

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

Application code reads like application code—not a serialized host manifest:

```php
<?php

declare(strict_types=1);

namespace App;

use Pam\Desktop\App;
use Pam\Desktop\Attributes\Command;
use Pam\Desktop\Attributes\Desktop;
use Pam\Desktop\Events;
use Pam\Desktop\WindowHandle;
use Pam\Desktop\WindowTheme;

#[Desktop(
    id: 'com.pushin.my-app',
    name: 'My desktop app',
    version: '1.0.0',
    description: 'A PHP-first native desktop application.',
    theme: WindowTheme::Dark,
)]
final class MyApp extends App
{
    #[Command]
    public function greet(
        WindowHandle $window,
        Events $events,
        string $name = 'mundo',
    ): array {
        $window->title("Olá, {$name}");
        $events->emit(new GreetingCompleted($name));

        return ['message' => "Olá, {$name}."];
    }
}

final readonly class GreetingCompleted
{
    public function __construct(public string $name) {}
}
```

The entry point is one line:

```php
App\MyApp::run();
```

`#[Command]` maps payload fields to typed parameters, resolves application
services from the container, collects native effects, derives typed event
names, and normalizes ordinary PHP return values. There is no context parsing,
effect tree, or protocol envelope on the happy path.

Need complete control? The immutable `Application`, `Manifest`, `Window`,
`Capabilities`, `Shell`, and `CommandResult` builders remain the stable
low-level API. The convention layer compiles into those same contracts; it is
not a second runtime.

In the trusted local frontend:

```js
console.assert(window.pam.apiVersion === 1);

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

const update = await window.pam.updater.check();
if (update.state === 4) {
    await window.pam.updater.download();
}

const system = await window.pam.plugins.invoke("system.info", "snapshot");
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

For data-intensive applications, continue with [Native SQLite](docs/database.md),
[binary streaming](docs/streaming.md), [native HTTP](docs/http.md), [Linux secrets](docs/secrets.md),
[authorized processes](docs/processes.md),
[file watching](docs/file-watching.md), [Linux desktop portals](docs/desktop-portals.md),
[command execution lanes](docs/execution.md),
[Linux system information](docs/system-information.md),
the [native windows guide](docs/windows.md), [Linux lifecycle](docs/lifecycle.md) and the
[runtime diagnostics guide](docs/diagnostics.md), then enforce the
[performance engineering contract](docs/performance.md).

Native shell events use the same ordered channel: `pam.menu.selected`,
`pam.tray.activated`, and `pam.shortcut.changed`. Continue with the
[Native shell guide](docs/native-shell.md), [Background jobs guide](docs/background-jobs.md),
and [Plugin guide](docs/plugins.md).

## Package and update

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

Linux portable archives contain `install.sh` and `uninstall.sh` for a per-user
installation, and Debian packaging requires `dpkg-deb`. The current release
pipeline generates Linux x86-64 artifacts only. Every runtime bundle contains a
`manifest.json` with protocol, application, runtime and target metadata plus
the byte size and SHA-256 digest of every shipped file. See the
[Linux distribution guide](docs/distribution.md).

Updates remain disabled unless PHP pins the feed and Ed25519 public key:

```php
use Pam\Desktop\Manifest;
use Pam\Desktop\UpdatePolicy;
use Pam\Desktop\Updates;

$manifest = Manifest::create('com.pushin.my-app', 'My desktop app', '1.0.0')
    ->updates(
        Updates::from(
            'https://updates.example.com/my-app/stable.json',
            '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
        )->policy(UpdatePolicy::Notify),
    );
```

Generate the offline signing seed once, keep it outside the repository, then
publish a feed from already-built artifacts:

```bash
pam desktop update-key --output ~/.config/pam/keys/my-app.key
pam desktop publish-update . \
  --key ~/.config/pam/keys/my-app.key \
  --output dist/stable.json \
  --published-at 2026-07-23T14:00:00Z \
  --artifact linux,x86_64,portable,dist/my-app.tar.gz,https://cdn.example.com/my-app.tar.gz
```

The private seed is created with owner-only permissions and is never embedded
in PHP or printed. See the [Updates and signing guide](docs/updates.md).

## Build the host

The Rust workspace pins Servo 0.4.0 and carries a committed `Cargo.lock`.
On Ubuntu 22.04 or compatible distributions:

```bash
sudo apt-get install -y \
  build-essential clang libclang-dev libfontconfig1-dev pkg-config
cargo build --locked --release -p pam-desktop
```

Tagged releases publish only
`pam-desktop-<version>-x86_64-unknown-linux-gnu.tar.gz` and its adjacent
SHA-256 file. Verify, extract and install it without root privileges:

```bash
sha256sum --check pam-desktop-1.0.0-x86_64-unknown-linux-gnu.tar.gz.sha256
tar -xzf pam-desktop-1.0.0-x86_64-unknown-linux-gnu.tar.gz
cd pam-desktop-1.0.0-x86_64-unknown-linux-gnu
./install.sh
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
├── pam-desktop-plugin/    process-isolated Rust plugin SDK
├── pam-desktop-protocol/  shared Rust contracts and integer discriminators
└── pam-desktop-shell/     Servo, Winit, gateway and process supervision
packages/
└── desktop/               public PHP application API and worker loop
compat/
├── php-api-v1.txt         frozen PHP symbols and signatures
├── protocol-v6/           golden worker/plugin transport fixtures
└── rust-plugin-v1/        Rust SDK compile-compatibility consumer
docs/
├── architecture.md        process, security and extension boundaries
├── authoring.md           convention-first applications and DI
├── background-jobs.md     supervised PHP scheduling and lifecycle events
├── capabilities.md        PHP policy and frontend native APIs
├── database.md            capability-scoped native SQLite
├── distribution.md        Linux bundles, archives and Debian packages
├── desktop-portals.md     user-mediated URI, screenshot and PDF integration
├── declarative-windows-and-menus.md typed windows and action menus
├── file-watching.md       capability-scoped change notifications
├── http.md                confined native HTTPS transport
├── lifecycle.md           single instance, deep links and associations
├── native-shell.md        menus, tray, global shortcuts and shell effects
├── performance.md         budgets, benchmark harness and release tuning
├── processes.md           allowlisted process execution without a shell
├── secrets.md             encrypted Linux Secret Service integration
├── plugins.md             PHP composition and process-isolated Rust SDK
├── streaming.md           backpressured binary file streams
├── stability.md           1.x support, SemVer and compatibility policy
├── typed-commands.md       method/class commands, DTOs and typed events
└── updates.md             signing, feeds and atomic automatic updates
packaging/linux/            rootless host installation templates
scripts/                    reproducible Linux host packaging and verification
```

Read [Architecture](docs/architecture.md) before expanding native capabilities.

## Validate

```bash
cargo fmt --all -- --check
cargo test --locked --workspace --no-default-features --features gateway
cargo clippy --locked --workspace --all-targets --no-default-features --features gateway -- -D warnings
cargo check --locked -p pam-desktop
composer test --working-dir=packages/desktop
composer analyse --working-dir=packages/desktop
composer validate --strict packages/desktop/composer.json
scripts/test-host-archive.sh dist/pam-desktop-1.0.0-x86_64-unknown-linux-gnu.tar.gz
```

## Roadmap

| Version | Delivery |
| --- | --- |
| **0.2** | **Events, deadlines, cancellation, crash recovery, multiple windows and hot reload — implemented** |
| **0.3** | **Authorized filesystem, dialogs, clipboard, notifications and drag and drop — implemented** |
| **0.4** | **Self-contained Linux build, icons, manifest and installers — implemented** |
| **0.5** | **Windows and macOS, signing and automatic updates — implemented** |
| **0.6** | **PHP/Rust plugins, menus, tray, global shortcuts and background jobs — implemented** |
| **1.0** | **Stable API, executable compatibility suite and production-grade Linux distribution — implemented** |

## License

Pam Desktop is MIT licensed. Servo and transitive dependencies retain their own
licenses.
