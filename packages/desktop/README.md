# pushinbr/pam-desktop

The PHP application model for Pam Desktop. It owns windows, commands, events,
jobs, plugins, native-shell policy, typed effects, payload validation and the
versioned worker loop. The Rust host owns Servo, operating-system handles,
security boundaries, plugin processes and lifecycle.

```php
<?php

use Pam\Desktop\Application;
use Pam\Desktop\ApplicationCategory;
use Pam\Desktop\BackgroundJob;
use Pam\Desktop\CommandContext;
use Pam\Desktop\CommandResult;
use Pam\Desktop\GlobalShortcut;
use Pam\Desktop\JobContext;
use Pam\Desktop\Menu;
use Pam\Desktop\MenuItem;
use Pam\Desktop\Shell;
use Pam\Desktop\Tray;
use Pam\Desktop\TrayCloseBehavior;
use Pam\Desktop\Window;
use Pam\Desktop\WindowEffect;

$app = Application::make(
    id: 'com.example.my-app',
    name: 'My application',
    window: Window::create('My application')
        ->load('resources/index.html')
        ->size(1120, 720),
)
    ->publisher('Example')
    ->category(ApplicationCategory::Development)
    ->shell(
        Shell::none()
            ->menu(Menu::create(
                'application',
                'Application',
                MenuItem::command('show', 'Show', 'CmdOrCtrl+Shift+KeyP'),
                MenuItem::separator(),
                MenuItem::command('quit', 'Quit'),
            ))
            ->tray(
                Tray::create('application', 'My application')
                    ->closeBehavior(TrayCloseBehavior::Hide),
            )
            ->shortcut(GlobalShortcut::create('show', 'CmdOrCtrl+Shift+KeyP')),
    )
    ->commandTimeout(10_000);

$app->command('greet', static function (CommandContext $command): CommandResult {
    $name = $command->string('name', 'world');

    return CommandResult::success(['message' => "Hello, {$name}!"])
        ->effect(WindowEffect::title("Hello, {$name}", $command->windowId));
});

$app->job(
    'heartbeat',
    BackgroundJob::every(30_000)->timeout(3_000),
    static fn (JobContext $job): array => ['runId' => $job->runId],
);

$app->run();
```

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
