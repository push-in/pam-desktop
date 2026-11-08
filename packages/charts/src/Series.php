<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts;

use InvalidArgumentException;

/**
 * One named sequence of numeric values drawn by line, area and bar charts.
 */
final readonly class Series
{
    /**
     * @param list<float> $values
     */
    private function __construct(
        public string $label,
        public array $values,
        public ?string $color,
    ) {
    }

    /**
     * @param array<array-key, int|float> $values finite numbers, in x order
     */
    public static function make(string $label, array $values): self
    {
        return new self(self::validLabel($label), self::validValues($values), null);
    }

    /**
     * Series color as `#rgb`, `#rrggbb` or `#rrggbbaa`; omit to take the palette color.
     */
    public function color(string $color): self
    {
        return new self($this->label, $this->values, Internal\Color::normalize($color));
    }

    public function count(): int
    {
        return count($this->values);
    }

    public function isEmpty(): bool
    {
        return $this->values === [];
    }

    public function min(): float
    {
        return $this->values === [] ? 0.0 : min($this->values);
    }

    public function max(): float
    {
        return $this->values === [] ? 0.0 : max($this->values);
    }

    /**
     * @param array<array-key, mixed> $values
     * @return list<float>
     */
    public static function validValues(array $values): array
    {
        if (count($values) > 4096) {
            throw new InvalidArgumentException('Chart series accept at most 4,096 values.');
        }

        $normalized = [];

        foreach ($values as $value) {
            if (!is_int($value) && !is_float($value)) {
                throw new InvalidArgumentException('Chart values must be numbers.');
            }

            $float = (float) $value;

            if (!is_finite($float)) {
                throw new InvalidArgumentException('Chart values must be finite.');
            }

            $normalized[] = $float;
        }

        return $normalized;
    }

    public static function validLabel(string $label): string
    {
        if (strlen($label) > 256 || str_contains($label, "\0")) {
            throw new InvalidArgumentException('Chart labels must be at most 256 bytes.');
        }

        return $label;
    }
}
