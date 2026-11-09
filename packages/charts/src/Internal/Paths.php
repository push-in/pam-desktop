<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts\Internal;

use Pam\Desktop\Canvas\Path;
use Pam\Desktop\Charts\Curve;

/**
 * Builds canvas paths for series lines, areas and bars.
 *
 * @internal
 */
final class Paths
{
    /** Control-point distance of a cubic quarter circle, per unit radius. */
    private const float KAPPA = 0.5522847498;

    /** Points per path command, keeping specs well under the canvas 4,096-character cap. */
    public const int CHUNK = 72;

    private function __construct()
    {
    }

    /**
     * Open paths through the points, straight or monotone-smoothed. Long series
     * are split into several paths so each spec stays within the canvas limit.
     *
     * @param non-empty-list<array{0: float, 1: float}> $points
     * @return non-empty-list<Path>
     */
    public static function line(array $points, Curve $curve, float $tension = 0.4): array
    {
        return array_map(
            static fn (array $chunk): Path => self::through($chunk, $curve, $tension),
            self::chunks($points),
        );
    }

    /**
     * Closed paths through the points, dropped to the baseline on both ends.
     *
     * @param non-empty-list<array{0: float, 1: float}> $points
     * @return non-empty-list<Path>
     */
    public static function area(array $points, Curve $curve, float $baseline, float $tension = 0.4): array
    {
        return array_map(
            static function (array $chunk) use ($curve, $baseline, $tension): Path {
                $first = $chunk[0];
                $last = $chunk[count($chunk) - 1];

                return self::through($chunk, $curve, $tension)
                    ->lineTo($last[0], $baseline)
                    ->lineTo($first[0], $baseline)
                    ->close();
            },
            self::chunks($points),
        );
    }

    /**
     * Bar anchored to the baseline with the far end rounded.
     *
     * Bars pointing up (`$top < $baseline`) round their top corners; bars
     * pointing down round their bottom corners. Radius shrinks to fit.
     */
    public static function bar(float $x, float $top, float $width, float $baseline, float $radius): Path
    {
        $height = abs($baseline - $top);
        $radius = max(0.0, min($radius, $width / 2, $height));
        $direction = $top <= $baseline ? 1.0 : -1.0;
        $inner = $top + $direction * $radius;
        // Circular corners (Chart.js `ctx.arc`), as cubic quarter circles.
        $k = $radius * self::KAPPA;

        return Path::start($x, $baseline)
            ->lineTo($x, $inner)
            ->curveTo($x, $inner - $direction * $k, $x + $radius - $k, $top, $x + $radius, $top)
            ->lineTo($x + $width - $radius, $top)
            ->curveTo($x + $width - $radius + $k, $top, $x + $width, $inner - $direction * $k, $x + $width, $inner)
            ->lineTo($x + $width, $baseline)
            ->close();
    }

    /**
     * Splits points into runs of at most `CHUNK` points; consecutive runs share
     * their boundary point so the drawing stays continuous.
     *
     * @param non-empty-list<array{0: float, 1: float}> $points
     * @return non-empty-list<non-empty-list<array{0: float, 1: float}>>
     */
    private static function chunks(array $points): array
    {
        $count = count($points);

        if ($count <= self::CHUNK) {
            return [$points];
        }

        $chunks = [];

        for ($offset = 0; $offset < $count - 1; $offset += self::CHUNK - 1) {
            $chunk = array_slice($points, $offset, self::CHUNK);

            if ($chunk !== []) {
                $chunks[] = $chunk;
            }
        }

        return $chunks === [] ? [$points] : $chunks;
    }

    /**
     * @param non-empty-list<array{0: float, 1: float}> $points
     */
    private static function through(array $points, Curve $curve, float $tension): Path
    {
        $path = Path::start($points[0][0], $points[0][1]);

        if ($curve === Curve::Smooth) {
            foreach (SmoothCurve::segments($points, $tension) as $segment) $path = $path->curveTo(...$segment);
            return $path;
        }

        if ($curve === Curve::Monotone) {
            foreach (MonotoneCurve::segments($points) as $segment) {
                $path = $path->curveTo(...$segment);
            }

            return $path;
        }

        foreach (array_slice($points, 1) as $point) {
            $path = $path->lineTo($point[0], $point[1]);
        }

        return $path;
    }
}
