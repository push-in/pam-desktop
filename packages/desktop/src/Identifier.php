<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final class Identifier
{
    private function __construct()
    {
    }

    public static function assert(string $value, string $label): void
    {
        if (preg_match('/^[a-z][a-z0-9._-]{0,63}$/i', $value) !== 1) {
            throw new InvalidArgumentException(
                "{$label} must begin with a letter and contain at most 64 ASCII letters, numbers, dots, dashes, or underscores.",
            );
        }
    }
}
