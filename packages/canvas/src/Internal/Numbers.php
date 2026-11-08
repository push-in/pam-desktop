<?php

declare(strict_types=1);

namespace Pam\Desktop\Canvas\Internal;

use InvalidArgumentException;

/**
 * Compact, locale-independent number formatting for string-encoded arguments
 * (path specs and point lists).
 *
 * @internal
 */
final class Numbers
{
    /**
     * Formats a coordinate with at most three decimals and no trailing zeros.
     */
    public static function format(float $value): string
    {
        if (!is_finite($value)) {
            throw new InvalidArgumentException('Canvas numbers must be finite.');
        }

        $text = number_format($value, 3, '.', '');
        $text = rtrim(rtrim($text, '0'), '.');

        return $text === '-0' || $text === '' ? '0' : $text;
    }
}
