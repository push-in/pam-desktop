<?php

declare(strict_types=1);

namespace Pam\Desktop\System;

use Pam\Desktop\HapticFeedback;
use Pam\Desktop\Internal\Runtime;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\NativeOperation;

final class Haptics
{
    private function __construct()
    {
    }

    public static function trigger(HapticFeedback $feedback): int
    {
        return Runtime::callNative(
            NativeOperation::Haptic,
            Wire::map(['feedback' => $feedback->value]),
            static function (): void {
            },
        );
    }
}
