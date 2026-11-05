<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum ReleaseChannel: int
{
    case Stable = 1;
    case Beta = 2;
    case Nightly = 3;

    public function label(): string
    {
        return match ($this) {
            self::Stable => 'stable',
            self::Beta => 'beta',
            self::Nightly => 'nightly',
        };
    }
}
