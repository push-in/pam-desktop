<?php

declare(strict_types=1);

namespace Pam\Desktop\Style;

/**
 * Chromium's fixed-point layout length (1/64 px) and the conversions Blink
 * makes from CSS values, so a native layout lands on the same fractions as
 * the web page it reproduces.
 */
final class LayoutUnit
{
    private const DENOMINATOR = 64;

    private function __construct()
    {
    }

    /**
     * A unitless or percentage `line-height` (`1.45`, `145%`, `1.5em`) as
     * Blink computes it: the number becomes a percentage (a float), the font
     * size a LayoutUnit (FromFloatRound), their product over 100 is
     * evaluated in float (MinimumValueForLength: `LayoutUnit × float` is a
     * float, then `/ 100.0f`) and converted back by truncation
     * (`LayoutUnit(float)`), e.g. 13.12px × 1.45 = 19.03125, 12px ×
     * calc(1/.75) = 16 and 12.8px × calc(1/.75) = 17.046875 (measured).
     */
    public static function lineHeight(float $fontSize, float $factor): float
    {
        $fixedFontSize = round($fontSize * self::DENOMINATOR) / self::DENOMINATOR;
        $percent = self::float($factor * 100.0);
        $value = self::float(self::float($fixedFontSize * $percent) / 100.0);

        return self::truncate($value);
    }

    /** `LayoutUnit(float)`: toward zero on the 1/64 px grid. */
    public static function truncate(float $value): float
    {
        $units = $value * self::DENOMINATOR;

        return ($units < 0 ? ceil($units) : floor($units)) / self::DENOMINATOR;
    }

    /** IEEE single precision, the type Blink computes these values in. */
    private static function float(float $value): float
    {
        $unpacked = unpack('g', pack('g', $value));

        return is_array($unpacked) ? (float) $unpacked[1] : $value;
    }
}
