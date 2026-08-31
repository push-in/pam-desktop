<!-- pam:product-page:start -->
<div align="center">

# PAM Desktop

**PHP in control. Rust at the boundary. Servo on screen.**

Build secure desktop applications with typed PHP logic, a Rust capability boundary, and a direct Servo host—without Electron or Node.js.

[![Release](https://img.shields.io/github/v/release/push-in/pam-desktop?style=flat-square&label=stable)](https://github.com/push-in/pam-desktop/releases)
[![CI](https://img.shields.io/github/actions/workflow/status/push-in/pam-desktop/ci.yml?branch=main&style=flat-square&label=CI)](https://github.com/push-in/pam-desktop/actions)
![PHP](https://img.shields.io/badge/PHP-8.5-777BB4?style=flat-square&logo=php&logoColor=white)
![License](https://img.shields.io/github/license/push-in/pam-desktop?style=flat-square)

**[Documentation](https://push-in.github.io/pam-docs/desktop/overview/) · [Why this exists](#why-this-exists) · [What you can build](#what-you-can-build) · [Quick start](#quick-start) · [Issues](https://github.com/push-in/pam-desktop/issues)**

</div>

---

## Why this exists

Build secure desktop applications with typed PHP logic, a Rust capability boundary, and a direct Servo host—without Electron or Node.js.

| | |
| --- | --- |
| **Role** | Desktop product |
| **Execution path** | Rust · Servo · PAM Runtime |
| **This repository owns** | Windows, lifecycle, secure bridge, capabilities, plugins, distribution |
| **Boundary** | PAM remains the worker runtime; filesystem and OS authority default to off |

## What you can build

- Secure internal tools and operations consoles
- Offline-first desktop products
- Cross-platform application shells with explicit native authority

## Quick start

```bash
pam init my-desktop --template desktop
cd my-desktop
pam composer require pushinbr/pam-desktop
pam desktop:dev
```

The **[PAM documentation](https://push-in.github.io/pam-docs/desktop/overview/)** covers prerequisites, production setup, and the complete workflow. PAM projects keep normal manifests and lockfiles; product features stay in the package that owns them.
<!-- pam:product-page:end -->

Pam Desktop is intentionally separate from the Pam server core. This repository
owns the native window, Servo integration, secure local bridge, shared protocol,
and the `pushinbr/pam-desktop` Composer package. Pam remains the PHP worker runtime.

It is a desktop stack with an opinion: PHP should own application logic, Rust
should own the security and process boundary, and Servo should render the local
experience directly. No Electron-sized Node runtime, no ambient filesystem
access, and no pretending that a browser tab is an application architecture.

Repository policy requires `cargo clean` after every local certification or
release build. CI enforces the same cleanup in `always()` steps; temporary
clean-room projects, package workspaces and downloaded toolchains must also be
removed after their evidence has been captured.

PAM Desktop pairs expressive authoring with explicit native authority:
capabilities default to off, commands are registered deliberately, file access
uses named roots or opaque grants, plugins run behind versioned contracts, and
releases carry integrity metadata. It is built to feel powerful without making
the security model vague.

The Composer launcher keeps the verified native host in the user cache. Each
invocation preserves the active host plus the two most recently used semantic
versions and removes older hosts. Interrupted download directories older than a
day are also removed, preventing Servo host archives from accumulating without
bound while retaining immediate rollback versions. Set `PAM_DESKTOP_CACHE_DIR`
to place this bounded cache in an explicit absolute directory.

## Create an application

From a Pam checkout that has this repository beside it:

```bash
pam init hello-desktop --template desktop
cd hello-desktop
pam desktop doctor
pam desktop dev
pam desktop build
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

New to the stack? The
**[five-minute Notes tutorial](https://push-in.github.io/pam-docs/desktop/quickstart/)**
walks from scaffold to typed command, injected service, frontend event, native
window effect and portable Linux package. The
**[desktop mental model](https://push-in.github.io/pam-docs/desktop/mental-model/)**
then explains exactly what PHP, Servo, Rust and the PAM worker each own.

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
[persistent full-text search](docs/search.md),
[visual regression testing](docs/visual-testing.md),
[permission auditing](docs/permission-audit.md),
[command execution lanes](docs/execution.md),
[Linux system information](docs/system-information.md),
the [native windows guide](docs/windows.md), [Linux lifecycle](docs/lifecycle.md) and the
[runtime diagnostics guide](docs/diagnostics.md), then enforce the
[performance engineering contract](docs/performance.md).

IDE-class applications can enable the complete [Native Workstation profile](docs/workstation.md):
process isolation, recovery, command registry, virtualized data surfaces, accessibility and release gates.

Native shell events use the same ordered channel: `pam.menu.selected`,
`pam.tray.activated`, and `pam.shortcut.changed`. Continue with the
[Native shell guide](docs/native-shell.md), [Background jobs guide](docs/background-jobs.md),
and [Plugin guide](docs/plugins.md).

## Package and update

The application manifest stays beside windows and capabilities in PHP. Build
the default directory and portable archive atomically:

```bash
pam desktop build
```

Additional formats and output control:

```bash
pam desktop build --output dist --format deb
pam desktop build --format all --force
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
pam desktop publish-update \
  --key ~/.config/pam/keys/my-app.key \
  --output dist/stable.json \
  --published-at 2026-07-23T14:00:00Z \
  --artifact linux,x86_64,portable,dist/my-app.tar.gz,https://cdn.example.com/my-app.tar.gz
```

The private seed is created with owner-only permissions and is never embedded
in PHP or printed. See the [Updates and signing guide](docs/updates.md).

## Build the host

The Rust workspace pins Servo 0.5.0 and carries a committed `Cargo.lock`.
On Ubuntu 22.04 or compatible distributions:

```bash
sudo apt-get install -y \
  build-essential clang libclang-dev libegl1 libfontconfig1-dev \
  libgl1-mesa-dri libxkbcommon-x11-0 pkg-config
cargo build --locked --release -p pam-desktop
```

Tagged releases publish target-labelled Linux x86-64, macOS arm64 and Windows
x64 host archives for the Composer bootstrap, with adjacent SHA-256 files.
Linux additionally ships rootless installation scripts and reproducibility and
footprint evidence. Windows/macOS host delivery is preview support until their
graphical installer/signing clean-room gates join the stable matrix.
Verify, extract and install the portable archive without root privileges:

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
├── permission-audit.md    release capability risk policy and JSON contract
├── processes.md           allowlisted process execution without a shell
├── secrets.md             encrypted Linux Secret Service integration
├── plugins.md             PHP composition and process-isolated Rust SDK
├── streaming.md           backpressured binary file streams
├── stability.md           1.x support, SemVer and compatibility policy
├── visual-testing.md      scoped pixel goldens and CI evidence
├── workstation-certification.md 50-contract release evidence map
├── typed-commands.md       method/class commands, DTOs and typed events
└── updates.md             signing, feeds and recoverable automatic updates
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
scripts/test-host-reproducibility-evidence.sh
scripts/test-desktop-footprint-evidence.sh
scripts/clean-room-desktop.sh
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
