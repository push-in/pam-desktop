<?php

declare(strict_types=1);

spl_autoload_register(static function (string $class): void {
    $prefix = 'Pam\\Desktop\\';
    if (!str_starts_with($class, $prefix)) {
        return;
    }

    require __DIR__.'/../src/'.str_replace('\\', '/', substr($class, strlen($prefix))).'.php';
});

use Pam\Desktop\Application;
use Pam\Desktop\ApplicationCategory;
use Pam\Desktop\Capabilities;
use Pam\Desktop\ClientEvent;
use Pam\Desktop\CommandContext;
use Pam\Desktop\CommandResult;
use Pam\Desktop\EffectKind;
use Pam\Desktop\EventContext;
use Pam\Desktop\FileSystemRoot;
use Pam\Desktop\Manifest;
use Pam\Desktop\ResponseStatus;
use Pam\Desktop\UpdatePolicy;
use Pam\Desktop\Updates;
use Pam\Desktop\Window;
use Pam\Desktop\WindowEffect;
use Pam\Desktop\WindowTheme;

function expect(bool $condition, string $message): void
{
    if (!$condition) {
        throw new RuntimeException($message);
    }
}

try {
    Manifest::create('Invalid Application ID', 'Pam', '0.4.0');
    expect(false, 'Unsafe application identifiers must be rejected.');
} catch (InvalidArgumentException) {
}

try {
    Manifest::create('com.pushin.pam', 'Pam', '0.4.0')->excludeFromBundle('vendor');
    expect(false, 'Required bundle paths must not be excludable.');
} catch (InvalidArgumentException) {
}

try {
    Updates::from('http://updates.pushin.dev/latest.json', str_repeat('a', 64));
    expect(false, 'Remote update endpoints must require HTTPS.');
} catch (InvalidArgumentException) {
}

try {
    Updates::from('https://updates.pushin.dev/latest.json', str_repeat('A', 64));
    expect(false, 'Update public keys must use canonical lowercase hexadecimal.');
} catch (InvalidArgumentException) {
}

$application = Application::create(
    Window::create('Pam Desktop')
        ->size(1024, 680)
        ->minimumSize(640, 480)
        ->theme(WindowTheme::Dark),
    Manifest::create('com.pushin.pam', 'Pam Desktop', '0.4.0')
        ->description('Typed PHP on a native desktop runtime.')
        ->publisher('Pushin')
        ->category(ApplicationCategory::Development)
        ->excludeFromBundle('storage/cache')
        ->updates(
            Updates::from(
                'https://updates.pushin.dev/pam/stable.json',
                str_repeat('a', 64),
            )->policy(UpdatePolicy::Notify),
        ),
)
    ->window(
        'settings',
        Window::create('Settings')
            ->entry('resources/settings.html')
            ->size(720, 520)
            ->minimumSize(640, 480)
            ->visible(false),
    )
    ->capabilities(
        Capabilities::none()
            ->filesystem(FileSystemRoot::readWrite('data', 'storage'))
            ->dialogs()
            ->clipboard()
            ->notifications()
            ->dragAndDrop(),
    )
    ->commandTimeout(12_000);

$application->command(
    'greet',
    static function (CommandContext $command): CommandResult {
        $name = $command->string('name', 'world');

        return CommandResult::success(['message' => "Hello, {$name}!"])
            ->effect(WindowEffect::title("Hello, {$name}"))
            ->event(new ClientEvent('greeting.completed', [
                'name' => $name,
                'sourceWindow' => $command->windowId,
            ]));
    },
);

$application->on(
    'settings.open',
    static fn (EventContext $event): CommandResult => CommandResult::success([
        'sourceWindow' => $event->windowId,
    ])->effect(WindowEffect::visible(true, 'settings')),
);

$boot = $application->dispatch([
    'version' => 5,
    'id' => 1,
    'kind' => 1,
    'windowId' => 'main',
    'command' => '@pam/boot',
    'payload' => null,
]);
expect($boot['status'] === ResponseStatus::Success->value, 'Boot should succeed.');
expect($boot['payload']['windows'][0]['theme'] === 3, 'The dark theme must be serialized as integer 3.');
expect($boot['payload']['windows'][0]['width'] === 1024, 'The configured width should be retained.');
expect($boot['payload']['windows'][1]['id'] === 'settings', 'The child window should be registered.');
expect($boot['payload']['commandTimeoutMs'] === 12_000, 'The timeout should be serialized.');
expect($boot['payload']['manifest']['identifier'] === 'com.pushin.pam', 'The app ID should be serialized.');
expect($boot['payload']['manifest']['category'] === 1, 'Development must be integer 1.');
expect($boot['payload']['manifest']['icon'] === 'resources/icon.svg', 'The default icon should be portable.');
expect($boot['payload']['manifest']['bundleExcludes'] === ['storage/cache'], 'Bundle exclusions should be retained.');
expect($boot['payload']['manifest']['updates']['policy'] === 2, 'Notify update policy must be integer 2.');
expect($boot['payload']['manifest']['updates']['channel'] === 'stable', 'The stable update channel is the default.');
expect($boot['payload']['capabilities']['filesystemRoots'][0]['name'] === 'data', 'The root should be named.');
expect($boot['payload']['capabilities']['filesystemRoots'][0]['access'] === 3, 'ReadWrite must be integer 3.');
expect($boot['payload']['capabilities']['dialogs'] === true, 'Dialogs should be enabled explicitly.');
expect($boot['payload']['capabilities']['clipboardRead'] === true, 'Clipboard read should be enabled.');
expect($boot['payload']['capabilities']['notifications'] === true, 'Notifications should be enabled.');
expect($boot['payload']['capabilities']['dragAndDrop'] === true, 'Drag and drop should be enabled.');

$greeting = $application->dispatch([
    'version' => 5,
    'id' => 2,
    'kind' => 1,
    'windowId' => 'main',
    'command' => 'greet',
    'payload' => ['name' => 'David'],
]);
expect($greeting['payload']['message'] === 'Hello, David!', 'The handler should receive typed payload data.');
expect($greeting['effects'][0]['kind'] === EffectKind::SetWindowTitle->value, 'The title effect should be emitted.');
expect($greeting['effects'][0]['windowId'] === 'main', 'Effects should target a window explicitly.');
expect($greeting['events'][0]['name'] === 'greeting.completed', 'Client events should be emitted.');

$event = $application->dispatch([
    'version' => 5,
    'id' => 3,
    'kind' => 1,
    'windowId' => 'main',
    'command' => '@pam/event',
    'payload' => [
        'name' => 'settings.open',
        'payload' => null,
    ],
]);
expect($event['status'] === ResponseStatus::Success->value, 'Registered events should succeed.');
expect($event['effects'][0]['kind'] === EffectKind::SetWindowVisible->value, 'Events may produce effects.');
expect($event['effects'][0]['windowId'] === 'settings', 'Event effects should target child windows.');

$missing = $application->dispatch([
    'version' => 5,
    'id' => 4,
    'kind' => 1,
    'windowId' => 'main',
    'command' => 'missing',
    'payload' => null,
]);
expect($missing['status'] === ResponseStatus::Failure->value, 'Unknown commands should fail.');
expect($missing['error']['code'] === 3, 'UnknownCommand must remain integer 3.');

echo "pam/desktop protocol tests passed\n";
