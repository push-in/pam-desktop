<?php

declare(strict_types=1);

namespace Pam\DesktopSsh;

use Closure;
use Pam\Desktop\Modules\NativeModuleException;
use Pam\Desktop\Modules\NativeModuleResult;
use Pam\Desktop\Modules\NativeModules;
use Pam\Desktop\Modules\WireBytes;

/** @internal Calls the host's `ssh` module. */
final class Native
{
    private function __construct()
    {
    }

    /**
     * @param array<string, string|int|bool|WireBytes> $payload
     * @param Closure(array<string, string|int|float|bool>): void $done
     * @param (Closure(SshException): void)|null $failed Without it, the failure is thrown.
     */
    public static function call(string $method, array $payload, Closure $done, ?Closure $failed = null): void
    {
        NativeModules::call(
            'ssh',
            $method,
            $payload,
            static function (NativeModuleResult $result) use ($done): void {
                $done($result->payload === '' ? [] : $result->values());
            },
            static function (NativeModuleException $error) use ($failed): void {
                $exception = SshException::fromNative($error);
                if ($failed === null) {
                    throw $exception;
                }
                $failed($exception);
            },
        );
    }

    /** @param array<string, string|int|float|bool> $values */
    public static function string(array $values, string $key): string
    {
        $value = $values[$key] ?? '';

        return is_string($value) ? $value : '';
    }

    /** @param array<string, string|int|float|bool> $values */
    public static function int(array $values, string $key, int $default = 0): int
    {
        $value = $values[$key] ?? $default;

        return is_int($value) ? $value : $default;
    }

    /** @param array<string, string|int|float|bool> $values */
    public static function bool(array $values, string $key): bool
    {
        return ($values[$key] ?? false) === true;
    }
}
