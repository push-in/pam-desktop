<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Closure;
use Pam\Desktop\Internal\ShellEvents;

/**
 * The icon of the application in the system tray (menu bar extras on macOS,
 * notification area on Windows, status notifier on Linux).
 *
 * ```php
 * Tray::show(
 *     tooltip: 'Sync is running',
 *     menu: Menu::make('Sync',
 *         MenuItem::make('Open', Window::show(...)),
 *         MenuItem::role(MenuRole::Quit),
 *     ),
 *     onActivate: Window::focus(...),
 * );
 * ```
 */
final class Tray
{
    private function __construct()
    {
    }

    /**
     * @param string|null $icon path of a PNG file; the application icon when null
     * @param Closure|null $onActivate the icon was clicked
     */
    public static function show(?Menu $menu = null, string $tooltip = '', ?string $icon = null, ?Closure $onActivate = null): void
    {
        if ($onActivate === null) {
            ShellEvents::forget('tray', 'activate');
        } else {
            ShellEvents::handle('tray', 'activate', static fn (): mixed => $onActivate());
        }
        ShellEvents::send('tray', [
            'visible' => true,
            'tooltip' => $tooltip,
            'icon' => $icon ?? '',
            'items' => ShellEvents::menu('tray', $menu->items ?? []),
        ]);
    }

    public static function hide(): void
    {
        ShellEvents::forget('tray', 'activate');
        ShellEvents::forgetPrefix('menu', 'tray:');
        ShellEvents::send('tray', ['visible' => false]);
    }
}
