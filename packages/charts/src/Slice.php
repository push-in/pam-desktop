<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts;

use InvalidArgumentException;

/**
 * One donut slice: a label with a non-negative share of the whole.
 */
final readonly class Slice
{
    private function __construct(
        public string $label,
        public float $value,
        public ?string $color,
    ) {
    }

    public static function make(string $label, float|int $value): self
    {
        $value = (float) $value;

        if (!is_finite($value) || $value < 0.0) {
            throw new InvalidArgumentException('Slice values must be finite and non-negative.');
        }

        return new self(Series::validLabel($label), $value, null);
    }

    /**
     * Slice color as `#rgb`, `#rrggbb` or `#rrggbbaa`; omit to take the palette color.
     */
    public function color(string $color): self
    {
        return new self($this->label, $this->value, Internal\Color::normalize($color));
    }
}
