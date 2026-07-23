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
use Pam\Desktop\BackgroundJob;
use Pam\Desktop\Capabilities;
use Pam\Desktop\ClientEvent;
use Pam\Desktop\CommandContext;
use Pam\Desktop\CommandResult;
use Pam\Desktop\EffectKind;
use Pam\Desktop\EventContext;
use Pam\Desktop\FileSystemRoot;
use Pam\Desktop\GlobalShortcut;
use Pam\Desktop\JobContext;
use Pam\Desktop\JobOverlapPolicy;
use Pam\Desktop\Manifest;
use Pam\Desktop\Menu;
use Pam\Desktop\MenuItem;
use Pam\Desktop\Plugin;
use Pam\Desktop\ResponseStatus;
use Pam\Desktop\RustPlugin;
use Pam\Desktop\Shell;
use Pam\Desktop\ShellEffect;
use Pam\Desktop\Tray;
use Pam\Desktop\TrayCloseBehavior;
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

expect(Application::API_VERSION === 1, 'The stable PHP API version should be 1.');
expect(Application::PROTOCOL_VERSION === 6, 'The stable PHP protocol should remain version 6.');

try {
    Shell::none()
        ->menu(Menu::create('orphan', 'Orphan', MenuItem::command('show', 'Show')))
        ->toArray();
    expect(false, 'A Linux native menu without a tray must be rejected.');
} catch (RuntimeException) {
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
    ->shell(
        Shell::none()
            ->menu(
                Menu::create(
                    'tray',
                    'Pam Desktop',
                    MenuItem::command('show', 'Show window', 'CmdOrCtrl+Shift+KeyP'),
                    MenuItem::checkbox('background', 'Background mode', true),
                    MenuItem::separator(),
                    MenuItem::command('quit', 'Quit'),
                ),
            )
            ->tray(
                Tray::create('tray', 'Pam Desktop')
                    ->closeBehavior(TrayCloseBehavior::Hide),
            )
            ->shortcut(GlobalShortcut::create('show', 'CmdOrCtrl+Shift+KeyP')),
    )
    ->job(
        'heartbeat',
        BackgroundJob::every(60_000)
            ->runOnStart()
            ->timeout(5_000)
            ->overlap(JobOverlapPolicy::Skip),
        static fn (JobContext $job): CommandResult => CommandResult::success([
            'runId' => $job->runId,
        ]),
    )
    ->rustPlugin(
        RustPlugin::executable('system', 'plugins/bin/system')
            ->arguments('--quiet')
            ->timeout(5_000),
    )
    ->commandTimeout(12_000);

$application->plugin(new class implements Plugin {
    public function identifier(): string
    {
        return 'fixture';
    }

    public function register(Application $application): void
    {
        $application->command(
            'plugin.ping',
            static fn (CommandContext $context): array => ['window' => $context->windowId],
        );
    }
});

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
    'version' => 6,
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
expect($boot['payload']['shell']['menus'][0]['id'] === 'tray', 'The tray menu should be serialized.');
expect($boot['payload']['shell']['menus'][0]['items'][1]['kind'] === 2, 'Checkbox must be integer 2.');
expect($boot['payload']['shell']['tray']['closeBehavior'] === 2, 'Hide behavior must be integer 2.');
expect($boot['payload']['shell']['shortcuts'][0]['id'] === 'show', 'The shortcut should be serialized.');
expect($boot['payload']['backgroundJobs'][0]['overlap'] === 1, 'Skip overlap must be integer 1.');
expect($boot['payload']['rustPlugins'][0]['id'] === 'system', 'Rust plugins should be declared.');
expect($boot['payload']['phpPlugins'] === ['fixture'], 'PHP plugins should be visible to the host.');

$greeting = $application->dispatch([
    'version' => 6,
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
    'version' => 6,
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
    'version' => 6,
    'id' => 4,
    'kind' => 1,
    'windowId' => 'main',
    'command' => 'missing',
    'payload' => null,
]);
expect($missing['status'] === ResponseStatus::Failure->value, 'Unknown commands should fail.');
expect($missing['error']['code'] === 3, 'UnknownCommand must remain integer 3.');

$job = $application->dispatch([
    'version' => 6,
    'id' => 5,
    'kind' => 1,
    'windowId' => 'main',
    'command' => '@pam/job',
    'payload' => [
        'id' => 'heartbeat',
        'runId' => 42,
        'startedAtMs' => 1_700_000_000_000,
    ],
]);
expect($job['payload']['runId'] === 42, 'Background jobs should receive a typed run context.');

$plugin = $application->dispatch([
    'version' => 6,
    'id' => 6,
    'kind' => 1,
    'windowId' => 'main',
    'command' => 'plugin.ping',
    'payload' => null,
]);
expect($plugin['payload']['window'] === 'main', 'PHP plugins should register application commands.');

$shellEffect = ShellEffect::menuChecked('background', false)->toArray();
expect($shellEffect['kind'] === 6, 'Menu checked effects must use integer kind 6.');

echo "pam/desktop protocol tests passed\n";
