# pushinbr/pam-desktop

The PHP application model for Pam Desktop. It owns windows, commands, events,
jobs, plugins, native-shell policy, typed effects, payload validation and the
versioned worker loop. The Rust host owns Servo, operating-system handles,
security boundaries, plugin processes and lifecycle.

The default authoring model follows PAM Native: declare the application once,
write commands as ordinary typed methods, and inject only the services a use
case needs.

```php
<?php

use Pam\Desktop\App;
use Pam\Desktop\Attributes\Command;
use Pam\Desktop\Attributes\Desktop;
use Pam\Desktop\Events;
use Pam\Desktop\WindowHandle;

#[Desktop(id: 'com.example.my-app', name: 'My application')]
final class MyApp extends App
{
    #[Command]
    public function greet(
        WindowHandle $window,
        Events $events,
        string $name = 'world',
    ): array {
        $window->title("Hello, {$name}");
        $events->emit('greeting.completed', compact('name'));

        return ['message' => "Hello, {$name}!"];
    }
}

MyApp::run();
```

The immutable builders shown in previous releases remain supported as the
advanced API and compile to the same protocol.

Protocol 6 supports:

- named windows and targeted effects;
- JavaScript-to-PHP event handlers and PHP-to-JavaScript events;
- bounded command timeouts, cancellation and crash recovery;
- explicit native capabilities and signed-update policy;
- native menus, tray, global shortcuts and dynamic shell effects;
- supervised periodic PHP jobs;
- composable PHP plugins;
- process-isolated Rust plugins invoked through `window.pam.plugins`.

All coded variants—statuses, kinds, categories, themes, policies and
errors—are sequential integer-backed enums. Application command, event, job and
plugin names remain explicit validated identifiers chosen by the application.

The PHP API has public version `1`:

```php
assert(Application::API_VERSION === 1);
assert(Application::PROTOCOL_VERSION === 6);
```

Every public symbol, method signature, constant, enum case and promoted
property is checked against `compat/php-api-v1.txt`. Additive changes require an
intentional snapshot review; removals or signature changes require a new major
API version.

Rust extensions use the separate `pam-desktop-plugin` SDK and never load a
dynamic-library ABI into the Servo host. Create one with:

```bash
pam desktop plugin new system.info .
pam desktop plugin build system.info .
```

Validate the package with:

```bash
composer test
composer analyse
```
