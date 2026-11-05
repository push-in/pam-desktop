<?php

declare(strict_types=1);

namespace Pam\Desktop\System;

use Pam\Desktop\Internal\Runtime;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\NativeOperation;

final class Vibration
{
    private function __construct()
    {
    }

    public static function vibrate(int $milliseconds = 30): int
    {
        return Runtime::callNative(
            NativeOperation::Vibrate,
            Wire::map(['milliseconds' => max(1, min(10_000, $milliseconds))]),
            static function (): void {
            },
        );
    }
}
