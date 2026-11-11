<?php

declare(strict_types=1);

namespace Pam\Desktop\Modules;

use Closure;
use Pam\Desktop\Internal\Runtime;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\ModuleResultStatus;

final class NativeModules
{
    private function __construct()
    {
    }

    /**
     * Installs an alternate module transport, primarily for deterministic
     * hosts and the official testing package. Pass null to restore native I/O.
     */
    public static function useTransport(?NativeModuleTransport $transport): void
    {
        Runtime::setModuleTransport($transport);
    }

    /**
     * @param array<string, string|int|float|bool|WireBytes> $payload Binary
     *        values go in a WireBytes; plain strings must be UTF-8.
     * @param Closure(NativeModuleResult): void $callback
     * @param (Closure(NativeModuleException): void)|null $onFailure Receives
     *        failure results instead of $callback. Without it, $callback sees
     *        the failure and an exception it throws reaches App::onError().
     */
    public static function call(
        string $module,
        string $method,
        array $payload,
        Closure $callback,
        ?Closure $onFailure = null,
    ): int {
        return self::callRaw($module, $method, Wire::map($payload), $callback, $onFailure);
    }

    /**
     * @param Closure(NativeModuleResult): void $callback
     * @param (Closure(NativeModuleException): void)|null $onFailure
     */
    public static function callRaw(
        string $module,
        string $method,
        string $payload,
        Closure $callback,
        ?Closure $onFailure = null,
    ): int {
        return Runtime::call(
            $module,
            $method,
            $payload,
            static function (ModuleResultStatus $status, string $result) use ($callback, $onFailure, $module, $method): void {
                if ($status === ModuleResultStatus::Failure && $onFailure !== null) {
                    $onFailure(new NativeModuleException($module, $method, $result));

                    return;
                }
                $callback(new NativeModuleResult($status, $result));
            },
        );
    }
}
