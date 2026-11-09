<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts;

use Pam\Desktop\Canvas\Canvas;
use Pam\Desktop\Canvas\PathMode;
use Pam\Desktop\Charts\Internal\Color;
use Pam\Desktop\Charts\Internal\Paths;

/**
 * Line chart with one value axis, optional gradient area, grid, legend,
 * highlight crosshair and pointer selection.
 *
 * ```php
 * LineChart::make(width: 328, height: 160)
 *     ->series(Series::make('Entradas', [120.5, 80, 140, 90, 200])->color('#19c5ff'))
 *     ->labels(['Seg', 'Ter', 'Qua', 'Qui', 'Sex'])
 *     ->curve(Curve::Monotone)
 *     ->area()
 *     ->highlight(2);
 * ```
 */
class LineChart extends CartesianChart
{
    protected Curve $curve = Curve::Linear;
    protected bool $area = false;
    protected float $tension = 0.4;
    protected float $lineWidth = self::LINE_WIDTH;
    protected float $pointRadius = 3.0;
    protected bool $markers = false;
    protected ?string $pointBackground = null;
    protected float $areaStartOpacity = 0.35;
    protected float $areaEndOpacity = 0.0;
    protected ?string $areaColor = null;
    /** @var array{0: float, 1: float}|null canvas y of the area gradient's start and end */
    protected ?array $areaGradientSpan = null;

    public function tension(float $value): static
    {
        if (!is_finite($value) || $value < 0 || $value > 1) throw new \InvalidArgumentException('Line tension must be between 0 and 1.');
        $copy = clone $this; $copy->tension = $value; return $copy;
    }

    public function lineWidth(float $value): static
    {
        if (!is_finite($value) || $value <= 0 || $value > 32) throw new \InvalidArgumentException('Line width must be between 0 and 32.');
        $copy = clone $this; $copy->lineWidth = $value; return $copy;
    }

    public function markers(bool $visible, float $radius = 3.0, ?string $background = null): static
    {
        if (!is_finite($radius) || $radius <= 0 || $radius > 32) throw new \InvalidArgumentException('Marker radius must be between 0 and 32.');
        $copy = clone $this; $copy->markers = $visible; $copy->pointRadius = $radius;
        $copy->pointBackground = $background === null ? null : Color::normalize($background);
        return $copy;
    }

    public function areaOpacity(float $start, float $end = 0.0): static
    {
        if (!is_finite($start) || !is_finite($end) || $start < 0 || $start > 1 || $end < 0 || $end > 1) throw new \InvalidArgumentException('Area opacity must be between 0 and 1.');
        $copy = clone $this; $copy->areaStartOpacity = $start; $copy->areaEndOpacity = $end; return $copy;
    }

    /**
     * Area fill colour (Chart.js `backgroundColor`) instead of the line colour,
     * and the canvas y where its gradient starts and ends (Chart.js
     * `createLinearGradient(0, y0, 0, y1)`); by default it spans the plot.
     */
    public function areaFill(?string $color, ?float $gradientStart = null, ?float $gradientEnd = null): static
    {
        $copy = clone $this;
        $copy->areaColor = $color === null ? null : Color::normalize($color);
        $copy->areaGradientSpan = $gradientStart === null || $gradientEnd === null || !is_finite($gradientStart) || !is_finite($gradientEnd)
            ? null : [$gradientStart, $gradientEnd];

        return $copy;
    }

    /**
     * Final so `make()` can safely build subclasses with `new static()`:
     * every LineChart variant shares this exact constructor signature.
     */
    final protected function __construct(float $width, float $height)
    {
        parent::__construct($width, $height);
    }

    public static function make(float|int $width, float|int $height): static
    {
        return new static((float) $width, (float) $height);
    }

    /** Interpolation between points. */
    public function curve(Curve $curve): static
    {
        $copy = clone $this;
        $copy->curve = $curve;

        return $copy;
    }

    /** Fills under each line with a gradient from 35% of the series color to transparent. */
    public function area(bool $enabled = true): static
    {
        $copy = clone $this;
        $copy->area = $enabled;

        return $copy;
    }

    public function indexAt(float $x): ?int
    {
        $count = $this->pointCount();

        if ($count === 0) {
            return null;
        }

        [$px, , $pw] = $this->plotRect();

        if ($x < $px - self::PADDING || $x > $px + $pw + self::PADDING) {
            return null;
        }

        if ($count === 1) {
            return 0;
        }

        $ratio = max(0.0, min(1.0, ($x - $px) / $pw));

        return (int) round($ratio * ($count - 1));
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
        $centers = $this->centers($px, $pw);
        $baseline = round($scale->position(0.0, $py + $ph, $py), 2);
        $canvas = $this->paintGrid($canvas, $scale, $px, $py, $pw, $ph);

        foreach ($this->series as $index => $series) {
            $points = $this->points($series, $scale, $centers, $py, $ph);

            if ($points === []) {
                continue;
            }

            $color = $this->theme->seriesColor($series->color, $index);

            if ($this->area && count($points) > 1) {
                foreach (Paths::area($points, $this->curve, $baseline, $this->tension) as $path) {
                    [$fromY, $toY] = $this->areaGradientSpan ?? [$py, $py + $ph];
                    $fill = $this->areaColor ?? $color;
                    $canvas = $canvas->gradientPath($path, 0, $fromY, 0, $toY, Color::alpha($fill, $this->areaStartOpacity), Color::alpha($fill, $this->areaEndOpacity));
                }
            }

            if (count($points) === 1) {
                $canvas = $canvas->circle($points[0][0], $points[0][1], self::MARKER_RADIUS, $color);
            } else {
                foreach (Paths::line($points, $this->curve, $this->tension) as $path) {
                    $canvas = $canvas->path($path, $color, $this->lineWidth, PathMode::Stroke);
                }
            }

            if ($this->markers && count($points) > 1) {
                foreach ($points as [$x, $y]) $canvas = $canvas->circle($x, $y, $this->pointRadius + 1, $color)->circle($x, $y, max(0.1, $this->pointRadius - 1), $this->pointBackground ?? $this->theme->surface);
            }

            if ($this->valueLabels) {
                foreach ($points as $i => [$x, $y]) {
                    $canvas = $this->paintValueLabel($canvas, $series->values[$i], $x, $y);
                }
            }
        }

        $canvas = $this->paintHighlight($canvas, $scale, $centers, $py, $ph);

        $canvas = $this->paintLabels($canvas, $centers, $px, $pw, $this->height - $this->theme->axis->paddingBottom
            - ($this->legend === LegendPosition::Bottom ? self::LEGEND_HEIGHT : 0.0) - 4.0);
        return $this->paintTooltip($canvas, $centers);
    }

    /** @param list<float> $centers */
    private function paintTooltip(Canvas $canvas, array $centers): Canvas
    {
        $index = $this->highlight;
        if (!$this->tooltip || $index === null || !isset($centers[$index])) return $canvas;
        $title = $this->labels[$index] ?? (string) ($index + 1);
        $lines = [];
        foreach ($this->series as $series) {
            if (isset($series->values[$index])) $lines[] = $series->label.': '.$this->format($series->values[$index]);
        }
        $width = Internal\Text::width($title, 13);
        foreach ($lines as $line) $width = max($width, Internal\Text::width($line, 12));
        $width = min($this->width - 8, $width + 24);
        $height = 40.0 + count($lines) * 18;
        $x = max(4.0, min($this->width - $width - 4, $centers[$index] + 12));
        $y = 8.0;
        $canvas = $canvas->roundRect($x, $y, $width, $height, 12, '#0F172AF5')
            ->label($title, $x + 12, $y + 23, 13, $this->theme->ink, \Pam\Desktop\Canvas\TextAlign::Left, \Pam\Desktop\Canvas\FontWeight::Bold);
        foreach ($lines as $lineIndex => $line) $canvas = $canvas->text($line, $x + 12, $y + 43 + $lineIndex * 18, 12, $this->theme->ink);
        return $canvas;
    }

    /**
     * @return list<float>
     */
    protected function centers(float $px, float $pw): array
    {
        $count = $this->pointCount();

        if ($count === 1) {
            return [round($px + $pw / 2, 2)];
        }

        $centers = [];

        for ($i = 0; $i < $count; $i++) {
            $centers[] = round($px + $pw * $i / ($count - 1), 2);
        }

        return $centers;
    }

    /**
     * @param list<float> $centers
     * @return list<array{0: float, 1: float}>
     */
    private function points(Series $series, Internal\LinearScale $scale, array $centers, float $py, float $ph): array
    {
        $points = [];

        foreach ($series->values as $i => $value) {
            $points[] = [$centers[$i], round($scale->position($value, $py + $ph, $py), 2)];
        }

        return $points;
    }

    /**
     * @param list<float> $centers
     */
    private function paintHighlight(Canvas $canvas, Internal\LinearScale $scale, array $centers, float $py, float $ph): Canvas
    {
        $index = $this->highlight;

        if ($index === null || !isset($centers[$index])) {
            return $canvas;
        }

        $x = $centers[$index];
        $canvas = $canvas->dashedLine($x, $py, $x, $py + $ph, $this->theme->muted, 1, 2, 3);

        foreach ($this->series as $seriesIndex => $series) {
            if (!isset($series->values[$index])) {
                continue;
            }

            $value = $series->values[$index];
            $y = round($scale->position($value, $py + $ph, $py), 2);
            $color = $this->theme->seriesColor($series->color, $seriesIndex);
            $canvas = $canvas
                ->circle($x, $y, self::MARKER_RADIUS + 2.0, $this->theme->surface)
                ->circle($x, $y, self::MARKER_RADIUS, $color);

            if (!$this->valueLabels) {
                $canvas = $this->paintValueLabel($canvas, $value, $x, $y, true);
            }
        }

        return $canvas;
    }
}
