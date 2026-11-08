<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts\Internal;

/**
 * Text measurement estimates for layout. Native text metrics are not available
 * when the display list is built, so widths are approximated from glyph counts.
 *
 * @internal
 */
final class Text
{
    private const float AVERAGE_GLYPH_WIDTH = 0.58;

    private function __construct()
    {
    }

    /** Estimated rendered width in the same units as `$size`. */
    public static function width(string $text, float $size): float
    {
        return mb_strlen($text) * $size * self::AVERAGE_GLYPH_WIDTH;
    }

    /** Baseline offset that visually centers text of `$size` on a vertical center. */
    public static function baselineOffset(float $size): float
    {
        return $size * 0.35;
    }

    /** Default number formatting: thousands separator, no decimals for integers. */
    public static function number(float $value): string
    {
        $decimals = abs($value - round($value)) < 1e-9 || abs($value) >= 100 ? 0 : 1;

        return number_format($value, $decimals, ',', '.');
    }
}
