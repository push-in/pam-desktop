<?php

declare(strict_types=1);

namespace Pam\Desktop\System;

use Closure;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\ModuleResultStatus;
use Pam\Desktop\Modules\NativeModules;
use RuntimeException;

final class BackgroundTasks
{
    private function __construct()
    {
    }

    /** @param Closure(int): void $callback */
    public static function begin(
        string $name,
        int $timeoutSeconds,
        Closure $callback,
    ): int {
        return NativeModules::call(
            'background',
            'begin',
            [
                'name' => $name,
                'timeoutSeconds' => max(1, min(600, $timeoutSeconds)),
            ],
            static function ($result) use ($callback): void {
                if ($result->status === ModuleResultStatus::Failure) {
                    throw new RuntimeException($result->payload);
                }
                $values = Wire::decodeMap($result->payload);
                $callback((int) ($values['token'] ?? 0));
            },
        );
    }

    public static function end(int $token, ?Closure $callback = null): int
    {
        return NativeModules::call(
            'background',
            'end',
            ['token' => $token],
            static function ($result) use ($callback): void {
                if ($result->status === ModuleResultStatus::Failure) {
                    throw new RuntimeException($result->payload);
                }
                $callback?->__invoke();
            },
        );
    }
}
