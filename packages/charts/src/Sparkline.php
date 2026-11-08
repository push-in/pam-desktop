<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts;

use Pam\Desktop\Canvas\Canvas;
use Pam\Desktop\Canvas\PathMode;
use Pam\Desktop\Charts\Internal\Color;
use Pam\Desktop\Charts\Internal\Paths;

/**
 * Tiny axis-less line for inline trends.
 *
 * ```php
 * Sparkline::make(width: 96, height: 32)->values([3, 5, 4, 8, 6])->curve(Curve::Monotone)->area();
 * ```
 */
final class Sparkline extends Chart
{
    public const float LINE_WIDTH = 2.0;
    public const float MARKER_RADIUS = 4.0;

    /** @var list<float> */
    private array $values = [];

    private ?string $color = null;
    private bool $area = false;
    private bool $endMarker = false;
    private Curve $curve = Curve::Linear;

    public static function make(float|int $width, float|int $height): self
    {
        return new self((float) $width, (float) $height);
    }

    /**
     * @param array<array-key, int|float> $values
     */
    public function values(array $values): self
    {
        $copy = clone $this;
        $copy->values = Series::validValues($values);

        return $copy;
    }

    /** Line color; defaults to the theme primary. */
    public function color(string $color): self
    {
        $copy = clone $this;
        $copy->color = Color::normalize($color);

        return $copy;
    }

    public function area(bool $enabled = true): self
    {
        $copy = clone $this;
        $copy->area = $enabled;

        return $copy;
    }

    /** Dot on the last value. */
    public function endMarker(bool $enabled = true): self
    {
        $copy = clone $this;
        $copy->endMarker = $enabled;

        return $copy;
    }

    public function curve(Curve $curve): self
    {
        $copy = clone $this;
        $copy->curve = $curve;

        return $copy;
    }

    protected function draw(Canvas $canvas): Canvas
    {
        $canvas = $this->paintSurface($canvas);
        $color = $this->color ?? $this->theme->primary;
        $inset = self::MARKER_RADIUS;
        $values = $this->values;
        $count = count($values);

        if ($values === []) {
            return $canvas->line($inset, $this->height / 2, $this->width - $inset, $this->height / 2, $this->theme->grid, 1);
        }

        $min = min(0.0, min($values));
        $max = max($values);

        if ($max - $min < 1e-9) {
            $max = $min + 1.0;
        }

        $top = $inset;
        $bottom = $this->height - $inset;
        $points = [];

        foreach ($values as $i => $value) {
            $x = $count === 1 ? $this->width / 2 : $inset + ($this->width - 2 * $inset) * $i / ($count - 1);
            $y = $bottom - ($bottom - $top) * ($value - $min) / ($max - $min);
            $points[] = [round($x, 2), round($y, 2)];
        }

        $baseline = round($bottom - ($bottom - $top) * (0.0 - $min) / ($max - $min), 2);

        if ($this->area && $count > 1) {
            foreach (Paths::area($points, $this->curve, $baseline) as $path) {
                $canvas = $canvas->gradientPath($path, 0, $top, 0, $bottom, Color::alpha($color, 0.35), Color::alpha($color, 0.0));
            }
        }

        if ($count === 1) {
            $canvas = $canvas->circle($points[0][0], $points[0][1], self::MARKER_RADIUS, $color);
        } else {
            foreach (Paths::line($points, $this->curve) as $path) {
                $canvas = $canvas->path($path, $color, self::LINE_WIDTH, PathMode::Stroke);
            }
        }

        if ($this->endMarker && $count > 1) {
            [$x, $y] = $points[$count - 1];
            $canvas = $canvas
                ->circle($x, $y, self::MARKER_RADIUS + 2.0, $this->theme->surface)
                ->circle($x, $y, self::MARKER_RADIUS, $color);
        }

        return $canvas;
    }
}
