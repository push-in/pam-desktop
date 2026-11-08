<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts\Internal;

use InvalidArgumentException;

/**
 * Linear value scale with "nice" 1-2-5 tick steps.
 *
 * @internal
 */
final readonly class LinearScale
{
    private function __construct(
        public float $min,
        public float $max,
        public float $step,
    ) {
    }

    /**
     * Builds a scale covering `[min, max]`, expanded outwards to multiples of a
     * nice step so that about `$intervals` tick intervals fit.
     */
    public static function nice(float $min, float $max, int $intervals = 3): self
    {
        if (!is_finite($min) || !is_finite($max)) {
            throw new InvalidArgumentException('Scale bounds must be finite.');
        }

        if ($min > $max) {
            [$min, $max] = [$max, $min];
        }

        if ($max - $min < 1e-9) {
            $max = $min === 0.0 ? 1.0 : $min + abs($min);
            $min = min(0.0, $min);
        }

        $intervals = max(1, min(20, $intervals));
        $step = self::niceStep(($max - $min) / $intervals);

        return new self(
            self::round(floor($min / $step) * $step),
            self::round(ceil($max / $step) * $step),
            $step,
        );
    }

    /**
     * Tick values from min to max, inclusive.
     *
     * @return list<float>
     */
    public function ticks(): array
    {
        $ticks = [];
        $count = (int) round(($this->max - $this->min) / $this->step);

        for ($i = 0; $i <= $count; $i++) {
            $ticks[] = self::round($this->min + $i * $this->step);
        }

        return $ticks;
    }

    /**
     * Maps a value to a coordinate between `$start` (min) and `$end` (max).
     */
    public function position(float $value, float $start, float $end): float
    {
        $range = $this->max - $this->min;
        $ratio = $range <= 0.0 ? 0.0 : ($value - $this->min) / $range;

        return $start + ($end - $start) * max(0.0, min(1.0, $ratio));
    }

    private static function niceStep(float $raw): float
    {
        if ($raw <= 0.0) {
            return 1.0;
        }

        $magnitude = 10 ** floor(log10($raw));
        $normalized = $raw / $magnitude;

        $factor = match (true) {
            $normalized <= 1.0 => 1.0,
            $normalized <= 2.0 => 2.0,
            $normalized <= 5.0 => 5.0,
            default => 10.0,
        };

        return self::round($factor * $magnitude);
    }

    private static function round(float $value): float
    {
        return round($value, 10);
    }
}
