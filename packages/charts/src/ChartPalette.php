<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts;

use InvalidArgumentException;

/**
 * Ordered series colors.
 *
 * Categorical palettes keep a fixed hue order so the same series index always
 * gets the same hue. Sequential palettes ramp one hue from light to dark for
 * magnitudes. Status colors (success, warning, danger) are intentionally
 * absent: keep them for status, never for series identity.
 */
final readonly class ChartPalette
{
    /** Brand-neutral categorical hues, in fixed order. */
    public const array CATEGORICAL = [
        '#19c5ff',
        '#a78bfa',
        '#f472b6',
        '#2dd4bf',
        '#818cf8',
        '#e879f9',
    ];

    /**
     * @param list<string> $colors normalized `#rrggbbaa` colors
     */
    private function __construct(public array $colors)
    {
    }

    /**
     * Fixed-order categorical palette; pass a primary color to make it the first hue.
     */
    public static function categorical(?string $primary = null): self
    {
        $colors = self::CATEGORICAL;

        if ($primary !== null) {
            $primary = Internal\Color::normalize($primary);
            $colors = [$primary, ...array_values(array_filter(
                $colors,
                static fn (string $color): bool => Internal\Color::normalize($color) !== $primary,
            ))];
        }

        return self::make(...$colors);
    }

    /**
     * One hue in `$steps` opacities from light (35%) to full ink, for magnitudes.
     */
    public static function sequential(string $color, int $steps = 5): self
    {
        if ($steps < 1 || $steps > 32) {
            throw new InvalidArgumentException('Sequential palettes need 1 to 32 steps.');
        }

        $colors = [];

        for ($i = 0; $i < $steps; $i++) {
            $alpha = $steps === 1 ? 1.0 : 0.35 + (0.65 * $i / ($steps - 1));
            $colors[] = Internal\Color::alpha($color, $alpha);
        }

        return new self($colors);
    }

    public static function make(string ...$colors): self
    {
        if ($colors === []) {
            throw new InvalidArgumentException('Palettes need at least one color.');
        }

        return new self(array_values(array_map(
            static fn (string $color): string => Internal\Color::normalize($color),
            $colors,
        )));
    }

    /** Color for a series index; wraps around when the palette is exhausted. */
    public function color(int $index): string
    {
        return $this->colors[max(0, $index) % count($this->colors)];
    }
}
