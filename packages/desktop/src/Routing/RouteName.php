<?php

declare(strict_types=1);

namespace Pam\Desktop\Routing;

use BackedEnum;
use InvalidArgumentException;

use function is_string;

/** @internal */
final class RouteName
{
    private function __construct()
    {
    }

    public static function value(string|BackedEnum $name): string
    {
        $value = $name instanceof BackedEnum ? $name->value : $name;
        // Navigation uses textual keys internally. Keep integer-backed enums
        // stable by value, including when their PHP case names are renamed.
        if ($name instanceof BackedEnum && is_int($value) && $value > 0) {
            return 'route-'.$value;
        }
        if (!is_string($value) || preg_match('/^[A-Za-z][A-Za-z0-9_.-]{0,127}$/', $value) !== 1) {
            throw new InvalidArgumentException('Route names must be safe strings or enum cases backed by safe strings or positive integers.');
        }

        return $value;
    }
}
