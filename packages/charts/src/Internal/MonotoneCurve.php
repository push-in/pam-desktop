<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts\Internal;

/**
 * Monotone cubic interpolation (Fritsch–Carlson) expressed as cubic Bézier segments.
 *
 * The produced control points never leave the vertical range of the segment they
 * belong to, so smoothed lines do not overshoot the data.
 *
 * @internal
 */
final class MonotoneCurve
{
    private function __construct()
    {
    }

    /**
     * @param list<array{0: float, 1: float}> $points x ascending
     * @return list<array{0: float, 1: float, 2: float, 3: float, 4: float, 5: float}>
     *         [c1x, c1y, c2x, c2y, x, y] per segment
     */
    public static function segments(array $points): array
    {
        $count = count($points);

        if ($count < 2) {
            return [];
        }

        $deltas = [];
        $widths = [];

        for ($i = 0; $i < $count - 1; $i++) {
            $width = $points[$i + 1][0] - $points[$i][0];
            $widths[] = $width;
            $deltas[] = $width <= 0.0 ? 0.0 : ($points[$i + 1][1] - $points[$i][1]) / $width;
        }

        $tangents = self::tangents($deltas, $count);
        $segments = [];

        for ($i = 0; $i < $count - 1; $i++) {
            $third = $widths[$i] / 3;
            $segments[] = [
                $points[$i][0] + $third,
                $points[$i][1] + $tangents[$i] * $third,
                $points[$i + 1][0] - $third,
                $points[$i + 1][1] - $tangents[$i + 1] * $third,
                $points[$i + 1][0],
                $points[$i + 1][1],
            ];
        }

        return $segments;
    }

    /**
     * @param list<float> $deltas
     * @return list<float>
     */
    private static function tangents(array $deltas, int $count): array
    {
        $tangents = [];
        $tangents[0] = $deltas[0];

        for ($i = 1; $i < $count - 1; $i++) {
            $tangents[$i] = $deltas[$i - 1] * $deltas[$i] <= 0.0
                ? 0.0
                : ($deltas[$i - 1] + $deltas[$i]) / 2;
        }

        $tangents[$count - 1] = $deltas[$count - 2];

        for ($i = 0; $i < $count - 1; $i++) {
            if ($deltas[$i] === 0.0) {
                $tangents[$i] = 0.0;
                $tangents[$i + 1] = 0.0;
                continue;
            }

            $alpha = $tangents[$i] / $deltas[$i];
            $beta = $tangents[$i + 1] / $deltas[$i];
            $norm = $alpha * $alpha + $beta * $beta;

            if ($norm > 9.0) {
                $scale = 3.0 / sqrt($norm);
                $tangents[$i] = $scale * $alpha * $deltas[$i];
                $tangents[$i + 1] = $scale * $beta * $deltas[$i];
            }
        }

        return array_values($tangents);
    }
}
