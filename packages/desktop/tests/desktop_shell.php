<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Dialog;
use Pam\Desktop\FileFilter;
use Pam\Desktop\Internal\Runtime;
use Pam\Desktop\Internal\ShellEvents;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\KeyEvent;
use Pam\Desktop\Menu;
use Pam\Desktop\MenuItem;
use Pam\Desktop\MenuRole;
use Pam\Desktop\ModuleResultStatus;
use Pam\Desktop\Modules\NativeModuleTransport;
use Pam\Desktop\Notification;
use Pam\Desktop\PropKey;
use Pam\Desktop\Shortcut;
use Pam\Desktop\Tray;
use Pam\Desktop\UI\Column as ShellColumn;
use Pam\Desktop\Window;
use Pam\Desktop\WindowState;

$assert ??= static function (bool $condition, string $message): void {
    if (!$condition) throw new RuntimeException($message);
};

/** A host that records desktop calls and delivers events through the long poll. */
$host = new class implements NativeModuleTransport {
    /** @var list<array{string, array<string, mixed>}> */
    public array $calls = [];
    public ?Closure $waiter = null;
    public int $polls = 0;
    /** @var array<string, string> method => reply payload */
    public array $replies = [];

    public function invoke(int $requestId, string $module, string $method, string $payload, Closure $complete): void
    {
        if ($module !== 'desktop') {
            $complete(ModuleResultStatus::Failure, 'unknown module');

            return;
        }
        if ($method === 'next') {
            $this->polls++;
            $this->waiter = $complete;

            return;
        }
        $this->calls[] = [$method, Wire::decodeMap($payload)];
        $complete(ModuleResultStatus::Success, $this->replies[$method] ?? '');
    }

    /** @param array<string, mixed> $data */
    public function emit(string $type, string $id, array $data = []): void
    {
        $waiter = $this->waiter;
        $this->waiter = null;
        $waiter(ModuleResultStatus::Success, Wire::map(['type' => $type, 'id' => $id, 'data' => json_encode($data ?: new stdClass())]));
    }

    /** @return array<string, mixed> */
    public function last(string $method): array
    {
        foreach (array_reverse($this->calls) as [$name, $payload]) {
            if ($name === $method) {
                return $payload;
            }
        }
        throw new RuntimeException("The host never received {$method}.");
    }
};
ShellEvents::reset();
Shortcut::resetRuntime();
Runtime::setModuleTransport($host);

// --- Application menu ---------------------------------------------------------
$log = [];
App::menu(
    Menu::make(
        'File',
        MenuItem::make('New', static function () use (&$log): void { $log[] = 'new'; }, 'CmdOrCtrl+N'),
        MenuItem::separator(),
        MenuItem::submenu('Recent', MenuItem::make('a.txt', static function () use (&$log): void { $log[] = 'recent'; })),
        MenuItem::checkbox('Autosave', true, static function (bool $checked) use (&$log): void { $log[] = $checked ? 'on' : 'off'; }),
        MenuItem::role(MenuRole::Quit),
    ),
    Menu::edit(),
);
$menu = json_decode($host->last('menu')['items'], true);
$file = $menu[0]['children'];
$assert($menu[0]['label'] === 'File' && $menu[1]['label'] === 'Edit' && count($menu[1]['children']) === 7, 'Menus become top-level submenus.');
$assert($file[0]['shortcut'] === 'CmdOrCtrl+N' && $file[1] === ['separator' => true], 'Items carry shortcuts and separators.');
$assert($file[2]['children'][0]['label'] === 'a.txt' && $file[3]['checked'] === true, 'Submenus and check items are encoded.');
$assert($file[4]['role'] === 'quit' && $file[4]['label'] === 'Quit' && $file[4]['shortcut'] === 'CmdOrCtrl+Q', 'Roles bring their label and shortcut.');
$assert($host->polls === 1 && $host->waiter !== null, 'A menu with handlers starts the event channel once.');

$host->emit('menu', $file[0]['id']);
$host->emit('menu', $file[2]['children'][0]['id']);
$host->emit('menu', $file[3]['id']);
$host->emit('menu', 'bar:unknown');
$assert($log === ['new', 'recent', 'off'], 'Menu activations run the handler of the chosen item: '.json_encode($log));
$assert($host->polls === 5 && $host->waiter !== null, 'The listener re-arms after every event.');

App::menu(Menu::make('File', MenuItem::make('Other', static function () use (&$log): void { $log[] = 'other'; })));
$host->emit('menu', $file[0]['id']);
$assert($log === ['new', 'recent', 'off'], 'Replacing the menu drops the previous handlers.');

// --- Shortcuts ----------------------------------------------------------------
$search = Shortcut::register('CmdOrCtrl+K', static function () use (&$log): void { $log[] = 'search'; });
$global = Shortcut::register('Ctrl+Alt+Space', static fn () => null, global: true);
$items = json_decode($host->last('shortcuts')['items'], true);
$assert(
    $items === [['id' => $search, 'shortcut' => 'CmdOrCtrl+K', 'global' => false], ['id' => $global, 'shortcut' => 'Ctrl+Alt+Space', 'global' => true]],
    'Every registration publishes the whole shortcut table.',
);
$host->emit('shortcut', $search);
$assert(end($log) === 'search', 'A shortcut runs its handler.');
Shortcut::unregister($search);
$assert(count(json_decode($host->last('shortcuts')['items'], true)) === 1, 'Unregistering republishes the table.');
$host->emit('shortcut', $search);
$assert(count($log) === 4, 'An unregistered shortcut no longer runs.');
Shortcut::unregisterAll();

// --- Window -------------------------------------------------------------------
Window::title('Invoices');
$assert($host->last('window') === ['command' => 'title', 'title' => 'Invoices'], 'Window commands carry their arguments.');
Window::progress(1.7);
$assert($host->last('window') === ['command' => 'progress', 'value' => 1.0], 'Progress is clamped to 0..1.');
Window::progress(null);
$assert($host->last('window') === ['command' => 'progress'], 'A null progress removes the indicator.');
$closing = 0;
Window::onCloseRequested(static function () use (&$closing): void { $closing++; });
$assert($host->last('window') === ['command' => 'interceptClose', 'enabled' => true], 'A close handler makes the host ask first.');
$host->emit('window', 'close-requested');
$assert($closing === 1, 'The close request reaches the handler.');
$states = [];
$subscription = Window::onStateChange(static function (WindowState $state) use (&$states): void { $states[] = $state; });
$host->emit('window', 'state', ['maximized' => true, 'focused' => false, 'width' => 1280, 'height' => 720.5]);
$assert(
    count($states) === 1 && $states[0]->maximized && !$states[0]->focused && $states[0]->width === 1280.0 && $states[0]->height === 720.5,
    'Window state changes arrive as WindowState.',
);
Window::offStateChange($subscription);
$host->emit('window', 'state', ['maximized' => false]);
$assert(count($states) === 1, 'An unsubscribed listener is not called.');
$host->replies['windowState'] = Wire::map(['state' => '{"fullScreen":true,"width":800,"height":600}']);
$current = null;
Window::state(static function (WindowState $state) use (&$current): void { $current = $state; });
$assert($current instanceof WindowState && $current->fullScreen && $current->width === 800.0, 'Window::state reads the current state.');
$dropped = [];
Window::onFilesDrop(static function (array $paths) use (&$dropped): void { $dropped = $paths; });
$host->emit('drop', 'files', ['paths' => ['/tmp/a.png', 7, '/tmp/b.png']]);
$assert($dropped === ['/tmp/a.png', '/tmp/b.png'], 'Dropped files keep only path strings.');

// --- Dialogs ------------------------------------------------------------------
$host->replies['openFile'] = Wire::map(['paths' => '["/home/a.md","/home/b.md"]']);
$picked = 'unset';
Dialog::openFile(static function (?string $path) use (&$picked): void { $picked = $path; }, 'Open', [new FileFilter('Docs', '.MD', 'txt')]);
$request = $host->last('openFile');
$assert($picked === '/home/a.md' && $request['title'] === 'Open', 'openFile reports the first chosen path.');
$assert(json_decode($request['filters'], true) === [['name' => 'Docs', 'extensions' => ['md', 'txt']]], 'Filters are normalised.');
$many = [];
Dialog::openFiles(static function (array $paths) use (&$many): void { $many = $paths; });
$assert($many === ['/home/a.md', '/home/b.md'] && $host->last('openFile')['multiple'] === true, 'openFiles reports every path.');
$host->replies['saveFile'] = Wire::map(['paths' => '[]']);
$saved = 'unset';
Dialog::saveFile(static function (?string $path) use (&$saved): void { $saved = $path; }, 'report.pdf');
$assert($saved === null && $host->last('saveFile')['defaultName'] === 'report.pdf', 'A cancelled dialog reports null.');

// --- Notifications and tray ---------------------------------------------------
$activated = 0;
$id = Notification::show('Export finished', 'report.pdf', static function () use (&$activated): void { $activated++; });
$assert($host->last('notify') == ['id' => $id, 'title' => 'Export finished', 'body' => 'report.pdf'], 'Notifications carry title and body: '.json_encode($host->last('notify')));
$host->emit('notification', $id);
$host->emit('notification', $id);
$assert($activated === 1, 'A notification activates its handler once.');
$trayLog = [];
Tray::show(
    Menu::make('Tray', MenuItem::make('Open', static function () use (&$trayLog): void { $trayLog[] = 'open'; })),
    'Running',
    onActivate: static function () use (&$trayLog): void { $trayLog[] = 'activate'; },
);
$tray = $host->last('tray');
$trayItems = json_decode($tray['items'], true);
$assert($tray['visible'] === true && $tray['tooltip'] === 'Running' && str_starts_with($trayItems[0]['id'], 'tray:'), 'The tray carries its menu.');
$host->emit('tray', 'activate');
$host->emit('menu', $trayItems[0]['id']);
$assert($trayLog === ['activate', 'open'], 'Tray clicks and tray menu items run their handlers.');
Tray::hide();
$host->emit('menu', $trayItems[0]['id']);
$assert($trayLog === ['activate', 'open'] && $host->last('tray') === ['visible' => false], 'Hiding the tray drops its handlers.');

// --- Element API --------------------------------------------------------------
$chosen = [];
$element = ShellColumn::make()
    ->cursor('pointer')
    ->tooltip('Open')
    ->windowDrag()
    ->contextMenu(
        MenuItem::make('Rename', static function () use (&$chosen): void { $chosen[] = 'rename'; }),
        MenuItem::separator(),
        MenuItem::submenu('More', MenuItem::make('Archive', static function () use (&$chosen): void { $chosen[] = 'archive'; })),
        MenuItem::make('Delete', static function () use (&$chosen): void { $chosen[] = 'delete'; }, destructive: true, enabled: false),
    )
    ->onKeyDown(static function (KeyEvent $event) use (&$chosen): void { $chosen[] = $event->key.($event->control ? '+ctrl' : ''); });
$properties = $element->properties();
$assert(
    $properties[PropKey::Cursor->value] === 'pointer' && $properties[PropKey::Tooltip->value] === 'Open'
        && $properties[PropKey::WindowDrag->value] === true && $properties[PropKey::OnKeyDown->value] === true
        && $properties[PropKey::OnMenuAction->value] === true,
    'Desktop properties and handler markers are set on the element.',
);

Runtime::setModuleTransport(null);
ShellEvents::reset();
Shortcut::resetRuntime();

// --- WebView (part of the desktop core, not a plugin) ---------------------------
$web = \Pam\Desktop\UI\WebView::make('https://example.com/help')
    ->allowedHosts('Example.com', 'cdn.example.com', 'example.com')
    ->injectedJavaScript('window.ready = true;')
    ->userAgent(null)
    ->onMessage(static fn (string $message, string $url): string => $message.'@'.$url);
$webProperties = $web->properties();
$assert(
    $webProperties[PropKey::WebViewSource->value] === 'https://example.com/help'
        && $webProperties[PropKey::WebViewJavaScriptEnabled->value] === true
        && $webProperties[PropKey::WebViewAllowedHosts->value] === "example.com\ncdn.example.com"
        && $webProperties[PropKey::WebViewInjectedJavaScript->value] === 'window.ready = true;'
        && $webProperties[PropKey::WebViewUserAgent->value] === ''
        && $webProperties[PropKey::OnWebViewMessage->value] === true,
    'WebView sends its source, policy and bridge properties.',
);
$assert(\Pam\Desktop\UI\WebView::make('<h1>Hi</h1>')->properties()[PropKey::WebViewSource->value] === '<h1>Hi</h1>', 'Inline HTML is a valid WebView source.');
foreach (['ftp://example.com/', 'javascript:alert(1)', ''] as $invalid) {
    $rejected = false;
    try {
        \Pam\Desktop\UI\WebView::make($invalid);
    } catch (InvalidArgumentException) {
        $rejected = true;
    }
    $assert($rejected, "WebView must reject the source {$invalid}.");
}
