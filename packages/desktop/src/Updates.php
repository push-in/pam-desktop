<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Closure;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\Modules\NativeModuleResult;
use Pam\Desktop\Modules\NativeModules;
use Throwable;

/**
 * Signed updates of the application, declared by `updates` in
 * `pam-desktop.json`.
 *
 * ```php
 * Updates::onChange(fn (UpdateState $state) => $this->update = $state);
 * Updates::check();
 * // …once the state is available:
 * Updates::download();
 * // …once it is ready (the application closes and starts again updated):
 * Updates::install();
 * ```
 *
 * With the `notify` policy the host checks by itself when the application
 * starts; with `automatic` it also downloads and installs.
 */
final class Updates
{
    /** @var array<int, Closure(UpdateState): mixed> */
    private static array $listeners = [];

    private static bool $listening = false;
    private static int $nextListener = 1;

    private function __construct()
    {
    }

    /**
     * Reads the current state.
     *
     * @param Closure(UpdateState): mixed $completed
     */
    public static function state(Closure $completed): void
    {
        self::send('state', $completed);
    }

    /**
     * Asks the update feed for a newer release. The outcome reaches
     * `onChange` listeners and `$completed`: `available`, `up-to-date` or
     * `failed`.
     *
     * @param (Closure(UpdateState): mixed)|null $completed
     */
    public static function check(?Closure $completed = null): void
    {
        self::send('check', $completed);
    }

    /**
     * Downloads and verifies the available release; progress reaches
     * `onChange` listeners until the state is `ready`.
     *
     * @param (Closure(UpdateState): mixed)|null $completed
     */
    public static function download(?Closure $completed = null): void
    {
        self::send('download', $completed);
    }

    /**
     * Installs the downloaded release: the application closes right after
     * and starts again updated. Save what must not be lost before calling.
     *
     * @param (Closure(UpdateState): mixed)|null $completed
     */
    public static function install(?Closure $completed = null): void
    {
        self::send('install', $completed);
    }

    /**
     * Runs `$listener` whenever the state changes, whoever changed it: a
     * call from the application or the update policy working by itself.
     * Returns a subscription for `offChange`.
     *
     * @param Closure(UpdateState): mixed $listener
     */
    public static function onChange(Closure $listener): int
    {
        $subscription = self::$nextListener++;
        self::$listeners[$subscription] = $listener;
        if (!self::$listening) {
            self::$listening = true;
            self::poll();
        }

        return $subscription;
    }

    public static function offChange(int $subscription): void
    {
        unset(self::$listeners[$subscription]);
    }

    /** @internal Runtime shutdown (tests, hot reload). */
    public static function resetRuntime(): void
    {
        self::$listeners = [];
        self::$listening = false;
    }

    /** @param (Closure(UpdateState): mixed)|null $completed */
    private static function send(string $method, ?Closure $completed): void
    {
        NativeModules::call(
            'updates',
            $method,
            [],
            static function (NativeModuleResult $result) use ($completed): void {
                if ($completed === null) {
                    return;
                }
                $completed($result->succeeded() ? self::decode($result->payload) : UpdateState::failed($result->message()));
            },
        );
    }

    private static function poll(): void
    {
        NativeModules::call(
            'updates',
            'next',
            [],
            static function (NativeModuleResult $result): void {
                // The host has no updates module (tests), or nobody listens
                // any more: stop asking.
                if (!$result->succeeded() || self::$listeners === []) {
                    self::$listening = false;

                    return;
                }
                // Ask for the next change first: a listener may not return soon.
                self::poll();
                $state = self::decode($result->payload);
                foreach (self::$listeners as $listener) {
                    $listener($state);
                }
            },
        );
    }

    private static function decode(string $payload): UpdateState
    {
        try {
            return UpdateState::fromArray($payload === '' ? [] : Wire::decodeMap($payload));
        } catch (Throwable) {
            return new UpdateState();
        }
    }
}
