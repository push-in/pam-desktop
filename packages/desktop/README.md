# pam/desktop

The PHP application model for Pam Desktop. It owns window configuration,
commands, typed effects, payload validation, and the versioned worker loop while
the Rust host owns Servo, operating-system integration, security boundaries, and
process lifecycle.

```php
<?php

use Pam\Desktop\Application;
use Pam\Desktop\CommandContext;
use Pam\Desktop\CommandResult;
use Pam\Desktop\Window;
use Pam\Desktop\WindowEffect;

$app = Application::create(
    window: Window::create('My application')->size(1120, 720),
);

$app->command('greet', static function (CommandContext $command): CommandResult {
    $name = $command->string('name', 'world');

    return CommandResult::success(['message' => "Hello, {$name}!"])
        ->effect(WindowEffect::title("Hello, {$name}"));
});

$app->run();
```

Protocol discriminators, statuses, effect kinds, themes, and errors are
sequential integer-backed enums. Application command names remain explicit
strings chosen by the application.

