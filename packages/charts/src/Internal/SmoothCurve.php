<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts\Internal;

/** Distance-weighted cubic interpolation with configurable tension. */
final class SmoothCurve
{
    /** @param list<array{0: float, 1: float}> $points @return list<array{float, float, float, float, float, float}> */
    public static function segments(array $points, float $tension): array
    {
        $controls = [];
        foreach ($points as $index => [$x, $y]) {
            [$px, $py] = $points[max(0, $index - 1)];
            [$nx, $ny] = $points[min(count($points) - 1, $index + 1)];
            $before = hypot($x - $px, $y - $py);
            $after = hypot($nx - $x, $ny - $y);
            $sum = $before + $after;
            $fa = $sum > 0 ? $tension * $before / $sum : 0;
            $fb = $sum > 0 ? $tension * $after / $sum : 0;
            $controls[] = [$x - $fa * ($nx - $px), $y - $fa * ($ny - $py), $x + $fb * ($nx - $px), $y + $fb * ($ny - $py)];
        }
        $segments = [];
        for ($index = 1; $index < count($points); $index++) {
            $segments[] = [$controls[$index - 1][2], $controls[$index - 1][3], $controls[$index][0], $controls[$index][1], $points[$index][0], $points[$index][1]];
        }
        return $segments;
    }
}
