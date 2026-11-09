<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts;

use InvalidArgumentException;
use Pam\Desktop\Canvas\Canvas;
use Pam\Desktop\Canvas\PathMode;
use Pam\Desktop\Charts\Internal\Color;
use Pam\Desktop\Charts\Internal\LinearScale;
use Pam\Desktop\Charts\Internal\Paths;

/**
 * Vertical bars anchored to the baseline with rounded far ends, grouped or
 * stacked when several series are present.
 *
 * ```php
 * BarChart::make(width: 328, height: 160)
 *     ->series(Series::make('Pix', [12, 18, 9, 22]))
 *     ->series(Series::make('Cartão', [8, 6, 11, 4]))
 *     ->labels(['Jan', 'Fev', 'Mar', 'Abr'])
 *     ->stacked();
 * ```
 */
final class BarChart extends CartesianChart
{
    public const float BAR_GAP = 2.0;
    public const float GROUP_PADDING_RATIO = 0.2;

    private BarLayout $layout = BarLayout::Grouped;
    private float $barRadius = 4.0;
    private ?float $barThickness = null;
    /** Chart.js bar charts start their value axis at 0 by default. */
    protected bool $beginAtZero = true;

    /** Chart.js bar scales are `offset`: categories centred in equal bands. */
    protected function categoryOffset(): bool
    {
        return true;
    }

    public static function make(float|int $width, float|int $height): self
    {
        return new self((float) $width, (float) $height);
    }

    /** One bar per series side by side (default). */
    public function grouped(bool $enabled = true): self
    {
        return $this->layout($enabled ? BarLayout::Grouped : BarLayout::Stacked);
    }

    /** Series stacked on one bar per category. */
    public function stacked(bool $enabled = true): self
    {
        return $this->layout($enabled ? BarLayout::Stacked : BarLayout::Grouped);
    }

    public function layout(BarLayout $layout): self
    {
        $copy = clone $this;
        $copy->layout = $layout;

        return $copy;
    }

    /**
     * Fixed bar width in dp (Chart.js `barThickness`), centered in its slot and
     * never wider than the slot allows; null restores the proportional width.
     */
    public function barThickness(float|int|null $thickness): self
    {
        if ($thickness !== null && (!is_finite((float) $thickness) || $thickness < 1 || $thickness > 400)) {
            throw new InvalidArgumentException('Bar thickness must be between 1 and 400 dp.');
        }

        $copy = clone $this;
        $copy->barThickness = $thickness === null ? null : (float) $thickness;

        return $copy;
    }

    /** Corner radius of the far end of each bar, in dp. */
    public function barRadius(float|int $radius): self
    {
        if ($radius < 0 || $radius > 64) {
            throw new InvalidArgumentException('Bar radius must be between 0 and 64 dp.');
        }

        $copy = clone $this;
        $copy->barRadius = (float) $radius;

        return $copy;
    }

    public function indexAt(float $x): ?int
    {
        $count = $this->pointCount();

        if ($count === 0) {
            return null;
        }

        [$px, , $pw] = $this->plotRect();

        if ($x < $px || $x > $px + $pw) {
            return null;
        }

        return min($count - 1, (int) floor(($x - $px) / ($pw / $count)));
    }

    protected function stackedExtremes(): array
    {
        if ($this->layout === BarLayout::Grouped) {
            return parent::stackedExtremes();
        }

        $extremes = [];

        for ($i = 0, $count = $this->pointCount(); $i < $count; $i++) {
            $positive = 0.0;
            $negative = 0.0;

            foreach ($this->series as $series) {
                $value = $series->values[$i] ?? 0.0;

                if ($value >= 0.0) {
                    $positive += $value;
                } else {
                    $negative += $value;
                }
            }

            $extremes[] = [$negative, $positive];
        }

        return $extremes;
    }

    protected function draw(Canvas $canvas): Canvas
    {
        $canvas = $this->paintSurface($canvas);
        [$px, $py, $pw, $ph] = $this->plotRect();
        $canvas = $this->paintLegend($canvas, $px, $pw);

        if (!$this->hasData()) {
            return $this->paintEmpty($canvas, $px, $py, $pw, $ph);
        }

        $scale = $this->scale();
        $count = $this->pointCount();
        $slot = $pw / $count;
        $padding = max(self::BAR_GAP, $slot * self::GROUP_PADDING_RATIO);
        $centers = [];

        for ($i = 0; $i < $count; $i++) {
            $centers[] = round($px + $slot * ($i + 0.5), 2);
        }

        if ($this->highlight !== null && $this->highlight < $count) {
            $canvas = $canvas->roundRect(
                $px + $slot * $this->highlight,
                $py,
                $slot,
                $ph,
                $this->barRadius,
                Color::alpha($this->theme->ink, 0.06),
            );
        }

        $canvas = $this->paintGrid($canvas, $scale, $px, $py, $pw, $ph);
        $canvas = $this->layout === BarLayout::Stacked
            ? $this->paintStacked($canvas, $scale, $px, $py, $ph, $slot, $padding)
            : $this->paintGrouped($canvas, $scale, $px, $py, $ph, $slot, $padding);

        return $this->paintLabels($canvas, $centers, $px, $pw, $this->height - $this->theme->axis->paddingBottom
            - ($this->legend === LegendPosition::Bottom ? self::LEGEND_HEIGHT : 0.0) - 4.0);
    }

    private function paintGrouped(Canvas $canvas, LinearScale $scale, float $px, float $py, float $ph, float $slot, float $padding): Canvas
    {
        $seriesCount = count($this->series);
        $barWidth = max(1.0, ($slot - 2 * $padding - ($seriesCount - 1) * self::BAR_GAP) / $seriesCount);
        if ($this->barThickness !== null) {
            $barWidth = min($this->barThickness, max(1.0, ($slot - ($seriesCount - 1) * self::BAR_GAP) / $seriesCount));
            $padding = ($slot - $seriesCount * $barWidth - ($seriesCount - 1) * self::BAR_GAP) / 2;
        }
        $baseline = round($scale->position(0.0, $py + $ph, $py), 2);

        foreach ($this->series as $index => $series) {
            $color = $this->theme->seriesColor($series->color, $index);

            foreach ($series->values as $i => $value) {
                $x = round($px + $slot * $i + $padding + $index * ($barWidth + self::BAR_GAP), 2);
                $top = round($scale->position($value, $py + $ph, $py), 2);
                // Chart.js `_calculateBarValuePixels`: a bar starting on the
                // drawn baseline stops half its 1px grid line short of it.
                $chartJs = $this->theme->axis->fitEdgeLabels;
                $base = $chartJs && $top !== $baseline ? $baseline + ($top < $baseline ? -0.5 : 0.5) : $baseline;
                // Chart.js `inflateAmount: 'auto'`: bars of a fixed thickness
                // grow 0.33px on every side so neighbours leave no AA seam.
                $inflate = $chartJs && $this->barThickness !== null ? 0.33 : 0.0;
                $up = $top <= $base ? 1.0 : -1.0;
                $canvas = $canvas->path(Paths::bar($x - $inflate, $top - $up * $inflate, round($barWidth + 2 * $inflate, 2), $base + $up * $inflate, $this->barRadius), $color, 0, PathMode::Fill);

                if ($this->valueLabels || $this->highlight === $i) {
                    $canvas = $this->paintValueLabel($canvas, $value, $x + $barWidth / 2, min($top, $baseline), $this->highlight === $i);
                }
            }
        }

        return $canvas;
    }

    private function paintStacked(Canvas $canvas, LinearScale $scale, float $px, float $py, float $ph, float $slot, float $padding): Canvas
    {
        $barWidth = round(max(1.0, $slot - 2 * $padding), 2);
        if ($this->barThickness !== null) {
            $barWidth = round(min($this->barThickness, $slot), 2);
            $padding = ($slot - $barWidth) / 2;
        }
        $baseline = round($scale->position(0.0, $py + $ph, $py), 2);
        $count = $this->pointCount();

        for ($i = 0; $i < $count; $i++) {
            $x = round($px + $slot * $i + $padding, 2);
            $positive = 0.0;
            $negative = 0.0;
            $total = 0.0;
            $segments = [];

            foreach ($this->series as $index => $series) {
                $value = $series->values[$i] ?? 0.0;

                if ($value === 0.0) {
                    continue;
                }

                $from = $value > 0.0 ? $positive : $negative;
                $to = $from + $value;

                if ($value > 0.0) {
                    $positive = $to;
                } else {
                    $negative = $to;
                }
                $total += $value;
                $segments[] = [$this->theme->seriesColor($series->color, $index), $from, $to, $value > 0.0];
            }

            $topmostPositive = null;
            $bottommostNegative = null;

            foreach ($segments as $segmentIndex => [, , , $isPositive]) {
                if ($isPositive) {
                    $topmostPositive = $segmentIndex;
                } else {
                    $bottommostNegative = $segmentIndex;
                }
            }

            foreach ($segments as $segmentIndex => [$color, $from, $to, $isPositive]) {
                $start = round($scale->position($from, $py + $ph, $py), 2);
                $end = round($scale->position($to, $py + $ph, $py), 2);
                $isEnd = $segmentIndex === ($isPositive ? $topmostPositive : $bottommostNegative);
                if ($this->theme->axis->fitEdgeLabels) {
                    // Chart.js: a segment starting on the baseline stops half
                    // its grid line short; fixed-thickness bars inflate 0.33px.
                    $up = $end <= $start ? 1.0 : -1.0;
                    if ($from === 0.0 && $start === $baseline) {
                        $start -= $up * 0.5;
                    }
                    $inflate = $this->barThickness !== null ? 0.33 : 0.0;
                    [$sx, $sw, $start, $end] = [$x - $inflate, $barWidth + 2 * $inflate, $start + $up * $inflate, $end - $up * $inflate];
                } else {
                    [$sx, $sw] = [$x, $barWidth];
                }

                $canvas = $isEnd
                    ? $canvas->path(Paths::bar($sx, $end, $sw, $start, $this->barRadius), $color, 0, PathMode::Fill)
                    : $canvas->fillRect($sx, min($start, $end), $sw, abs($end - $start), $color);
            }

            if ($segments !== [] && ($this->valueLabels || $this->highlight === $i)) {
                $top = round($scale->position(max($positive, 0.0), $py + $ph, $py), 2);
                $canvas = $this->paintValueLabel($canvas, $total, $x + $barWidth / 2, $top, $this->highlight === $i);
            }
        }

        return $canvas;
    }
}
