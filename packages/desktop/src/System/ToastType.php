<?php

declare(strict_types=1);

namespace Pam\Desktop\System;

/** The meaning of a card toast; it picks the accent colour. */
enum ToastType: int
{
    case Success = 1;
    case Error = 2;
    case Info = 3;

    /** @internal */
    public function accentColor(): int
    {
        return match ($this) {
            self::Success => 0xFF69C779,
            self::Error => 0xFFFE6301,
            self::Info => 0xFF87CEFA,
        };
    }
}
