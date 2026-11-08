<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts\Internal;

use InvalidArgumentException;

/**
 * CSS hex color helpers. Every color leaves here as lowercase `#rrggbbaa`.
 *
 * @internal
 */
final class Color
{
    private function __construct()
    {
    }

    /**
     * Accepts `#rgb`, `#rrggbb` and `#rrggbbaa`; anything else is rejected.
     */
    public static function normalize(string $color): string
    {
        $hex = strtolower(ltrim($color, '#'));

        if (!str_starts_with($color, '#') || preg_match('/^[0-9a-f]+$/D', $hex) !== 1) {
            throw new InvalidArgumentException("Invalid chart color \"{$color}\".");
        }

        return match (strlen($hex)) {
            3 => '#' . $hex[0] . $hex[0] . $hex[1] . $hex[1] . $hex[2] . $hex[2] . 'ff',
            6 => '#' . $hex . 'ff',
            8 => '#' . $hex,
            default => throw new InvalidArgumentException("Invalid chart color \"{$color}\"."),
        };
    }

    /**
     * Converts a packed `0xAARRGGBB` integer (the form template attributes arrive in) to `#rrggbbaa`.
     */
    public static function fromArgb(int $argb): string
    {
        $argb &= 0xFFFFFFFF;
        $alpha = ($argb >> 24) & 0xFF;
        $rgb = $argb & 0xFFFFFF;

        return '#' . str_pad(dechex($rgb), 6, '0', STR_PAD_LEFT) . str_pad(dechex($alpha), 2, '0', STR_PAD_LEFT);
    }

    /**
     * Normalizes a color given as a CSS hex string or a packed ARGB integer.
     */
    public static function fromMixed(string|int $color): string
    {
        return is_int($color) ? self::fromArgb($color) : self::normalize($color);
    }

    /**
     * Replaces the alpha channel (0..1) of a color.
     */
    public static function alpha(string $color, float $alpha): string
    {
        $alpha = max(0.0, min(1.0, $alpha));
        $byte = (int) round($alpha * 255);

        return substr(self::normalize($color), 0, 7) . str_pad(dechex($byte), 2, '0', STR_PAD_LEFT);
    }

    /**
     * Alpha channel of a color as 0..1.
     */
    public static function opacity(string $color): float
    {
        return hexdec(substr(self::normalize($color), 7, 2)) / 255;
    }
}
