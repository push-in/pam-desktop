# pam/desktop

The PHP application model for Pam Desktop. It owns window configuration,
commands, typed effects, payload validation, and the versioned worker loop while
the Rust host owns Servo, operating-system integration, security boundaries, and
process lifecycle.

```php
<?php

use Pam\Desktop\Application;
use Pam\Desktop\ClientEvent;
use Pam\Desktop\CommandContext;
use Pam\Desktop\CommandResult;
use Pam\Desktop\Window;
use Pam\Desktop\WindowEffect;

$app = Application::create(
    window: Window::create('My application')->size(1120, 720),
)
    ->window(
        'settings',
        Window::create('Settings')
            ->entry('resources/settings.html')
            ->visible(false),
    )
    ->commandTimeout(10_000);

$app->command('greet', static function (CommandContext $command): CommandResult {
    $name = $command->string('name', 'world');

    return CommandResult::success(['message' => "Hello, {$name}!"])
        ->effect(WindowEffect::title("Hello, {$name}", $command->windowId))
        ->event(new ClientEvent(
            'greeting.completed',
            ['name' => $name],
            $command->windowId,
        ));
});

$app->run();
```

Protocol 2 supports named windows, JavaScript-to-PHP event handlers with
`Application::on`, PHP-to-JavaScript `ClientEvent` values, targeted effects and
bounded command timeouts. Protocol discriminators, statuses, effect kinds,
themes, and errors are sequential integer-backed enums. Application command and
event names remain explicit strings chosen by the application.

Validate the package with:

```bash
composer test
composer analyse
```
