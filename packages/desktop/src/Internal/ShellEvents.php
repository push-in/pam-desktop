<?php

declare(strict_types=1);

namespace Pam\Desktop\Internal;

use Closure;
use Pam\Desktop\Modules\NativeModuleResult;
use Pam\Desktop\Modules\NativeModules;
use Pam\Desktop\ModuleResultStatus;
use Throwable;

/**
 * The event channel of the desktop shell: menu items, shortcuts, the tray,
 * notifications, dropped files and window changes arrive here from the host
 * through one re-armed long poll.
 *
 * @internal
 */
final class ShellEvents
{
    /** @var array<string, array<string, Closure>> type => id => handler */
    private static array $handlers = [];

    /** @var array<string, array<int, Closure>> type => listeners that see every id */
    private static array $listeners = [];

    private static bool $listening = false;
    private static int $nextListener = 1;
    private static int $nextId = 1;

    private function __construct()
    {
    }

    /** A fresh identifier for a handler (`m12`, `s3`…). */
    public static function id(string $prefix): string
    {
        return $prefix.(self::$nextId++);
    }

    /** Routes events of `$type` carrying `$id` to `$handler`. */
    public static function handle(string $type, string $id, Closure $handler): void
    {
        self::$handlers[$type][$id] = $handler;
        self::listen();
    }

    public static function forget(string $type, string $id): void
    {
        unset(self::$handlers[$type][$id]);
    }

    /** Drops every handler of `$type` whose id starts with `$prefix`. */
    public static function forgetPrefix(string $type, string $prefix): void
    {
        foreach (array_keys(self::$handlers[$type] ?? []) as $id) {
            if (str_starts_with((string) $id, $prefix)) {
                unset(self::$handlers[$type][$id]);
            }
        }
    }

    /**
     * Sends a command to the desktop shell.
     *
     * @param array<string, mixed> $payload
     * @param (Closure(array<string, mixed>): void)|null $completed receives the decoded reply
     * @param (Closure(string): void)|null $failed receives the failure message
     */
    public static function send(string $method, array $payload = [], ?Closure $completed = null, ?Closure $failed = null): void
    {
        NativeModules::call(
            'desktop',
            $method,
            $payload,
            static function (NativeModuleResult $result) use ($completed, $failed): void {
                if ($result->status !== ModuleResultStatus::Success) {
                    $failed?->__invoke($result->message());

                    return;
                }
                if ($completed === null) {
                    return;
                }
                try {
                    $values = $result->payload === '' ? [] : Wire::decodeMap($result->payload);
                } catch (Throwable) {
                    $values = [];
                }
                $completed($values);
            },
        );
    }

    /**
     * Serialises a menu for the host and registers its handlers under
     * `$scope` (`bar`, `tray`), replacing the ones of the previous menu.
     *
     * @param list<\Pam\Desktop\MenuItem> $items
     */
    public static function menu(string $scope, array $items): string
    {
        self::forgetPrefix('menu', $scope.':');
        $encoded = json_encode(self::menuLevel($scope, $items), JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES | JSON_THROW_ON_ERROR);
        if ((self::$handlers['menu'] ?? []) !== []) {
            self::listen();
        }

        return $encoded;
    }

    /**
     * @param list<\Pam\Desktop\MenuItem> $items
     * @return list<array<string, mixed>>
     */
    private static function menuLevel(string $scope, array $items): array
    {
        $entries = [];
        foreach ($items as $item) {
            if ($item->separator) {
                $entries[] = ['separator' => true];
                continue;
            }
            if ($item->children !== []) {
                $entries[] = [
                    'label' => $item->label,
                    'enabled' => $item->enabled,
                    'children' => self::menuLevel($scope, $item->children),
                ];
                continue;
            }
            $id = $scope.':'.($item->id ?? self::id('m'));
            $entry = ['id' => $id, 'label' => $item->label, 'enabled' => $item->enabled];
            if ($item->shortcut !== null && $item->shortcut !== '') {
                $entry['shortcut'] = $item->shortcut;
            }
            if ($item->checked !== null) {
                $entry['checked'] = $item->checked;
            }
            if ($item->destructive) {
                $entry['destructive'] = true;
            }
            if ($item->role !== null) {
                $entry['role'] = $item->role->value;
            }
            if ($item->handler !== null) {
                $handler = $item->handler;
                self::$handlers['menu'][$id] = static fn (): mixed => $handler();
            }
            $entries[] = $entry;
        }

        return $entries;
    }

    /**
     * Subscribes to every event of `$type`. The listener receives the id and
     * the decoded JSON data.
     *
     * @param Closure(string, array<array-key, mixed>): mixed $listener
     */
    public static function subscribe(string $type, Closure $listener): int
    {
        $subscription = self::$nextListener++;
        self::$listeners[$type][$subscription] = $listener;
        self::listen();

        return $subscription;
    }

    public static function unsubscribe(string $type, int $subscription): void
    {
        unset(self::$listeners[$type][$subscription]);
    }

    /** @internal Runtime shutdown (tests, hot reload). */
    public static function reset(): void
    {
        self::$handlers = [];
        self::$listeners = [];
        self::$listening = false;
    }

    private static function listen(): void
    {
        if (self::$listening) {
            return;
        }
        self::$listening = true;
        self::poll();
    }

    private static function poll(): void
    {
        NativeModules::call(
            'desktop',
            'next',
            [],
            static function (NativeModuleResult $result): void {
                if ($result->status !== ModuleResultStatus::Success) {
                    // The host has no desktop shell (tests): stop asking.
                    self::$listening = false;

                    return;
                }
                // Ask for the next event first: a handler may never return control soon.
                self::poll();
                try {
                    $event = Wire::decodeMap($result->payload);
                } catch (Throwable) {
                    return;
                }
                $type = is_string($event['type'] ?? null) ? $event['type'] : '';
                $id = is_string($event['id'] ?? null) ? $event['id'] : '';
                $data = json_decode(is_string($event['data'] ?? null) ? $event['data'] : '{}', true);
                self::dispatch($type, $id, is_array($data) ? $data : []);
            },
            static function (): void {
                self::$listening = false;
            },
        );
    }

    /**
     * @internal
     * @param array<array-key, mixed> $data
     */
    public static function dispatch(string $type, string $id, array $data): void
    {
        $handler = self::$handlers[$type][$id] ?? null;
        if ($handler !== null) {
            $handler($data);
        }
        foreach (self::$listeners[$type] ?? [] as $listener) {
            $listener($id, $data);
        }
    }
}
