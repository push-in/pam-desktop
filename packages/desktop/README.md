# pam/desktop

The PHP application model for Pam Desktop. It owns window configuration,
commands, typed effects, payload validation, and the versioned worker loop while
the Rust host owns Servo, operating-system integration, security boundaries, and
process lifecycle.

```php
<?php

use Pam\Desktop\Application;
use Pam\Desktop\ApplicationCategory;
use Pam\Desktop\ClientEvent;
use Pam\Desktop\CommandContext;
use Pam\Desktop\CommandResult;
use Pam\Desktop\Manifest;
use Pam\Desktop\Window;
use Pam\Desktop\WindowEffect;

$app = Application::create(
    window: Window::create('My application')->size(1120, 720),
    manifest: Manifest::create('com.example.my-app', 'My application', '0.4.0')
        ->publisher('Example')
        ->category(ApplicationCategory::Development),
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

Protocol 4 supports named windows, JavaScript-to-PHP event handlers,
PHP-to-JavaScript events, targeted effects, bounded command timeouts, explicit
native capabilities and typed application distribution metadata. Protocol
discriminators, statuses, categories, effect kinds, themes and errors are
sequential integer-backed enums. Application command and event names remain
explicit strings chosen by the application.

Validate the package with:

```bash
composer test
composer analyse
```
