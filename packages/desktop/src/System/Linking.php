<?php

declare(strict_types=1);

namespace Pam\Desktop\System;

use Closure;
use InvalidArgumentException;
use Pam\Desktop\Internal\Runtime;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\ModuleResultStatus;
use Pam\Desktop\Modules\NativeModules;
use Pam\Desktop\NativeOperation;
use Pam\Desktop\Navigation\Navigator;
use LogicException;
use RuntimeException;

final class Linking
{
    private static int $nextSubscription = 1;

    /** @var array<int, Closure(string): void> */
    private static array $subscriptions = [];

    /** @var array<int, array{0: string, 1: Closure(string): void}> */
    private static array $claims = [];

    private static bool $polling = false;

    private function __construct()
    {
    }

    /** @param null|Closure(string): void $failure */
    public static function open(
        string $url,
        ?Closure $opened = null,
        ?Closure $failure = null,
    ): int
    {
        return Runtime::callNative(
            NativeOperation::OpenUrl,
            Wire::map(['url' => $url]),
            static function (ModuleResultStatus $status, string $payload) use ($opened, $failure): void {
                if ($status === ModuleResultStatus::Failure) {
                    if ($failure !== null) {
                        $failure($payload);

                        return;
                    }
                    throw new RuntimeException($payload);
                }

                $opened?->__invoke();
            },
        );
    }

    /**
     * @param Closure(bool): void $callback
     * @param null|Closure(string): void $failure
     */
    public static function canOpen(
        string $url,
        Closure $callback,
        ?Closure $failure = null,
    ): int
    {
        return Runtime::callNative(
            NativeOperation::CanOpenUrl,
            Wire::map(['url' => $url]),
            static function (ModuleResultStatus $status, string $payload) use ($callback, $failure): void {
                if ($status === ModuleResultStatus::Failure) {
                    if ($failure !== null) {
                        $failure($payload);

                        return;
                    }
                    throw new RuntimeException($payload);
                }

                $values = Wire::decodeMap($payload);
                $callback((bool) ($values['supported'] ?? false));
            },
        );
    }

    /**
     * @param Closure(?string): void $callback
     * @param null|Closure(string): void $failure
     */
    public static function initial(Closure $callback, ?Closure $failure = null): int
    {
        return NativeModules::call(
            'linking',
            'initialUrl',
            [],
            static function ($result) use ($callback, $failure): void {
                if ($result->status === ModuleResultStatus::Failure) {
                    if ($failure !== null) {
                        $failure($result->payload);

                        return;
                    }
                    throw new RuntimeException($result->payload);
                }
                $values = Wire::decodeMap($result->payload);
                $url = trim((string) ($values['url'] ?? ''));
                $callback($url !== '' ? $url : null);
            },
        );
    }

    /** @param Closure(string): void $callback */
    public static function listen(Closure $callback): int
    {
        if (self::$subscriptions !== []) {
            throw new LogicException('Only one Linking listener may be active.');
        }
        $subscription = self::$nextSubscription++;
        self::$subscriptions[$subscription] = $callback;
        self::poll();

        return $subscription;
    }

    public static function unsubscribe(int $subscription): void
    {
        unset(self::$subscriptions[$subscription]);
    }

    /**
     * Takes the incoming links that start with `$prefix` (an OAuth redirect
     * URI) before the `listen()` listener sees any: a library can wait for
     * its own callback without owning the application's links. Returns an
     * id for `release()`.
     *
     * @param Closure(string): void $handler
     */
    public static function claim(string $prefix, Closure $handler): int
    {
        if (trim($prefix) === '') {
            throw new InvalidArgumentException('A link claim needs a prefix.');
        }
        $id = self::$nextSubscription++;
        self::$claims[$id] = [$prefix, $handler];
        self::poll();

        return $id;
    }

    public static function release(int $claim): void
    {
        unset(self::$claims[$claim]);
    }

    /** @internal */
    public static function resetRuntime(): void
    {
        self::$subscriptions = [];
        self::$claims = [];
        self::$polling = false;
        self::$nextSubscription = 1;
    }

    /** @param null|Closure(string, bool): void $callback */
    /** The launch URL already routed by {@see routeLaunchUrl()} (initial() skips it once). */
    private static ?string $routedLaunchUrl = null;
    private static bool $launchUrlTaken = false;

    /**
     * The URL the app was launched with, as the host reported it at boot
     * (`PAM_DESKTOP_LAUNCH_URL`), once per process: later PHP requests (hot
     * reload) start without it.
     */
    public static function takeLaunchUrl(): ?string
    {
        if (self::$launchUrlTaken) {
            return null;
        }
        self::$launchUrlTaken = true;
        $url = getenv('PAM_DESKTOP_LAUNCH_URL');
        if (!is_string($url) || trim($url) === '') {
            return null;
        }
        putenv('PAM_DESKTOP_LAUNCH_URL');

        return trim($url);
    }

    /**
     * Opens the launch URL on $navigator before the first frame, like React
     * Navigation's linking resolves the initial state: a deep-linked cold
     * start renders its screen first instead of the initial route followed
     * by a navigation. The asynchronous initial-URL delivery of
     * {@see listenAndRoute()} then reports it without opening it again.
     */
    public static function routeLaunchUrl(Navigator $navigator): void
    {
        $url = self::takeLaunchUrl();
        if ($url !== null && $navigator->openInitial($url)) {
            self::$routedLaunchUrl = $url;
        }
    }

    public static function listenAndRoute(
        Navigator $navigator,
        ?Closure $callback = null,
    ): int {
        self::initial(static function (?string $url) use ($navigator, $callback): void {
            if ($url === null) {
                return;
            }
            if (self::$routedLaunchUrl !== null && $url === self::$routedLaunchUrl) {
                self::$routedLaunchUrl = null;
                $callback?->__invoke($url, true);

                return;
            }
            $handled = $navigator->open($url);
            $callback?->__invoke($url, $handled);
        });

        return self::listen(static function (string $url) use ($navigator, $callback): void {
            $handled = $navigator->open($url);
            $callback?->__invoke($url, $handled);
        });
    }

    /** One wait for the next link at a time, while anyone listens or claims. */
    private static function poll(): void
    {
        if (self::$polling || (self::$subscriptions === [] && self::$claims === [])) {
            return;
        }
        self::$polling = true;
        NativeModules::call(
            'linking',
            'nextUrl',
            [],
            static function ($result): void {
                self::$polling = false;
                if ($result->status === ModuleResultStatus::Failure) {
                    self::$subscriptions = [];
                    self::$claims = [];
                    throw new RuntimeException($result->payload);
                }
                $values = Wire::decodeMap($result->payload);
                $url = trim((string) ($values['url'] ?? ''));
                if ($url !== '') {
                    self::deliver($url);
                }
                self::poll();
            },
        );
    }

    private static function deliver(string $url): void
    {
        foreach (self::$claims as [$prefix, $handler]) {
            if (str_starts_with($url, $prefix)) {
                $handler($url);

                return;
            }
        }
        foreach (self::$subscriptions as $callback) {
            $callback($url);
        }
    }
}
