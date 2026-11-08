<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Closure;
use InvalidArgumentException;
use Pam\Desktop\Internal\ShellEvents;

/**
 * Keyboard shortcuts of the application.
 *
 * ```php
 * Shortcut::register('CmdOrCtrl+K', $this->openSearch(...));
 * Shortcut::register('CmdOrCtrl+Shift+Space', Window::focus(...), global: true);
 * ```
 *
 * A combination joins modifiers (`Ctrl`, `Cmd`, `Alt`, `Shift`, `Super`,
 * `CmdOrCtrl`) and one key (`A`, `7`, `F5`, `Enter`, `Escape`, `Space`,
 * `ArrowUp`, `Plus`…) with `+`. Global shortcuts fire even when another
 * application has the focus, where the system allows it.
 */
final class Shortcut
{
    /** @var array<string, array{shortcut: string, global: bool}> id => registration */
    private static array $registered = [];

    private function __construct()
    {
    }

    /** Registers `$handler` for the combination and returns its id, for `unregister`. */
    public static function register(string $shortcut, Closure $handler, bool $global = false): string
    {
        $shortcut = trim($shortcut);
        if ($shortcut === '') {
            throw new InvalidArgumentException('A shortcut needs a key combination.');
        }
        $id = ShellEvents::id('s');
        self::$registered[$id] = ['shortcut' => $shortcut, 'global' => $global];
        ShellEvents::handle('shortcut', $id, static fn (): mixed => $handler());
        self::publish();

        return $id;
    }

    public static function unregister(string $id): void
    {
        if (!isset(self::$registered[$id])) {
            return;
        }
        unset(self::$registered[$id]);
        ShellEvents::forget('shortcut', $id);
        self::publish();
    }

    public static function unregisterAll(): void
    {
        foreach (array_keys(self::$registered) as $id) {
            ShellEvents::forget('shortcut', $id);
        }
        self::$registered = [];
        self::publish();
    }

    /** @internal */
    public static function resetRuntime(): void
    {
        self::$registered = [];
    }

    private static function publish(): void
    {
        $items = [];
        foreach (self::$registered as $id => $registration) {
            $items[] = ['id' => $id] + $registration;
        }
        ShellEvents::send('shortcuts', ['items' => json_encode($items, JSON_THROW_ON_ERROR)]);
    }
}
