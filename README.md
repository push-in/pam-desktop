<div align="center">

# Pam Desktop

### PHP in control. Rust at the boundary. Servo on screen.

A direct Servo host for building native desktop applications whose application
logic remains elegant, typed PHP.

![Status](https://img.shields.io/badge/status-experimental-f59e0b?style=flat-square)
![Servo](https://img.shields.io/badge/Servo-0.4.0-5b50d6?style=flat-square)
![PHP](https://img.shields.io/badge/PHP-8.4-777BB4?style=flat-square&logo=php&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-1.88%2B-000000?style=flat-square&logo=rust&logoColor=white)
![License](https://img.shields.io/badge/license-MIT-22c55e?style=flat-square)

</div>

---

Pam Desktop is intentionally separate from the Pam server core. This repository
owns the native window, Servo integration, secure local bridge, shared protocol,
and the `pam/desktop` Composer package. Pam remains the PHP worker runtime.

The first contract is deliberately small:

- local HTML, CSS, JavaScript and assets rendered directly by Servo;
- explicit JavaScript-to-PHP commands through `window.pam.invoke`;
- typed, immutable window configuration and effects in PHP;
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
pam-desktop doctor .
pam-desktop dev .
```

The generated Hello World demonstrates the complete round trip: a Servo-rendered
form invokes `greet`, PHP returns a payload and a typed window-title effect, and
the Rust host applies both without exposing a generic native bridge.

Application code stays compact:

```php
<?php

declare(strict_types=1);

use Pam\Desktop\Application;
use Pam\Desktop\CommandContext;
use Pam\Desktop\CommandResult;
use Pam\Desktop\Window;
use Pam\Desktop\WindowEffect;
use Pam\Desktop\WindowTheme;

$app = Application::create(
    window: Window::create('My desktop app')
        ->size(1120, 720)
        ->minimumSize(720, 520)
        ->theme(WindowTheme::Dark),
);

$app->command('greet', static function (CommandContext $command): CommandResult {
    $name = $command->string('name', 'mundo');

    return CommandResult::success(['message' => "Olá, {$name}."])
        ->effect(WindowEffect::title("Olá, {$name}"));
});

$app->run();
```

In the trusted local frontend:

```js
const result = await window.pam.invoke("greet", { name: "David" });
console.log(result.message);
```

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

Pam and `pam-desktop` must both be available on `PATH`. For a custom Pam binary,
set `PAM_BINARY=/absolute/path/to/pam`.

## Repository map

```text
crates/
├── pam-desktop-protocol/  shared Rust contracts and integer discriminators
└── pam-desktop-shell/     Servo, Winit, gateway and Pam worker supervision
packages/
└── desktop/               public PHP application API and worker loop
docs/
└── architecture.md        process, security and extension boundaries
```

Read [Architecture](docs/architecture.md) before expanding native capabilities.

## Validate

```bash
cargo fmt --all -- --check
cargo test --locked -p pam-desktop-protocol
cargo test --locked -p pam-desktop --no-default-features --features gateway
cargo check --locked -p pam-desktop
php packages/desktop/tests/run.php
composer validate --strict packages/desktop/composer.json
```

## License

Pam Desktop is MIT licensed. Servo and transitive dependencies retain their own
licenses.
