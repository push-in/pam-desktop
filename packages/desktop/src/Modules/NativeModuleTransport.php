<?php

declare(strict_types=1);

namespace Pam\Desktop\Modules;

use Closure;
use Pam\Desktop\ModuleResultStatus;

interface NativeModuleTransport
{
    /** @param Closure(ModuleResultStatus, string): void $complete */
    public function invoke(
        int $requestId,
        string $module,
        string $method,
        string $payload,
        Closure $complete,
    ): void;
}
