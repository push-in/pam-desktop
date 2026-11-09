<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts;

use Closure;
use InvalidArgumentException;
use Pam\Desktop\Canvas\Canvas;
use Pam\Desktop\Canvas\CanvasEventKind;
use Pam\Desktop\Canvas\FontWeight;
use Pam\Desktop\Canvas\TextAlign;
use Pam\Desktop\Charts\Internal\LinearScale;
use Pam\Desktop\Charts\Internal\Text;

/**
 * Shared behaviour of charts with categories on x and one value axis on y:
 * series, x labels, nice y ticks, grid, legend, highlight and pointer selection.
 */
abstract class CartesianChart extends Chart
{
    public const float LINE_WIDTH = 2.0;
    public const float MARKER_RADIUS = 4.0;
    public const float PADDING = 8.0;
    public const float AXIS_LABEL_SIZE = 10.0;
    public const float VALUE_LABEL_SIZE = 11.0;
    public const float X_LABEL_HEIGHT = 18.0;
    public const float LEGEND_HEIGHT = 20.0;

    /** @var list<Series> */
    protected array $series = [];

    /** @var list<string> */
    protected array $labels = [];

    protected int $grid = 3;
    /** Chart.js `beginAtZero`: line charts fit their data, bar charts start at 0. */
    protected bool $beginAtZero = false;
    protected bool $axis = true;
    protected ?int $highlight = null;
    protected bool $valueLabels = false;
    protected bool $tooltip = false;
    protected LegendPosition $legend = LegendPosition::None;

    /** @var (Closure(float): string)|null */
    protected ?Closure $valueFormatter = null;

    /** @var (Closure(?int): void)|null */
    protected ?Closure $onSelect = null;

    /** Adds one series. Series without a color take the next palette hue. */
    public function series(Series $series): static
    {
        if (count($this->series) >= 16) {
            throw new InvalidArgumentException('Charts accept at most 16 series.');
        }

        $copy = clone $this;
        $copy->series[] = $series;

        return $copy;
    }

    /**
     * Category labels drawn under the plot, one per x position.
     *
     * @param array<array-key, string|int|float> $labels
     */
    public function labels(array $labels): static
    {
        $copy = clone $this;
        $copy->labels = array_values(array_map(
            static fn (string|int|float $label): string => Series::validLabel((string) $label),
            $labels,
        ));

        return $copy;
    }

    /** Number of horizontal grid lines above the baseline (0 hides the grid). */
    public function grid(int $lines): static
    {
        $copy = clone $this;
        $copy->grid = max(0, min(20, $lines));

        return $copy;
    }

    /**
     * Chart.js `beginAtZero`. Off (line and area charts), the value axis
     * covers the data's own minimum and maximum, widened to nice ticks
     * (980..2040 draws 800..2200 with 7 intervals); on (bar charts), it
     * always includes 0.
     */
    public function beginAtZero(bool $begin = true): static
    {
        $copy = clone $this;
        $copy->beginAtZero = $begin;

        return $copy;
    }

    /** Shows or hides the value axis tick labels. */
    public function axis(bool $visible = true): static
    {
        $copy = clone $this;
        $copy->axis = $visible;

        return $copy;
    }

    /** Draws the formatted value next to every point or bar. */
    public function valueLabels(bool $visible = true): static
    {
        $copy = clone $this;
        $copy->valueLabels = $visible;

        return $copy;
    }

    public function tooltip(bool $visible = true): static
    {
        $copy = clone $this;
        $copy->tooltip = $visible;
        return $copy;
    }

    public function legend(LegendPosition $position): static
    {
        $copy = clone $this;
        $copy->legend = $position;

        return $copy;
    }

    /**
     * Formats axis ticks and value labels.
     *
     * @param Closure(float): string $formatter
     */
    public function valueFormatter(Closure $formatter): static
    {
        $copy = clone $this;
        $copy->valueFormatter = $formatter;

        return $copy;
    }

    /** Index of the x position to emphasise, or null for none. */
    public function highlight(?int $index): static
    {
        $copy = clone $this;
        $copy->highlight = $index === null ? null : max(0, $index);

        return $copy;
    }

    /**
     * Called with the nearest x index while the pointer is down, and with null
     * when the gesture is cancelled. Re-render with `highlight()` to show it.
     *
     * @param Closure(?int): void $handler
     */
    public function onSelect(Closure $handler): static
    {
        $copy = clone $this;
        $copy->onSelect = $handler;

        return $copy;
    }

    /** Number of x positions: the longest series. */
    public function pointCount(): int
    {
        $count = 0;

        foreach ($this->series as $series) {
            $count = max($count, $series->count());
        }

        return $count;
    }

    /** The nice value scale for the current data, as drawn. */
    public function scale(): LinearScale
    {
        $min = INF;
        $max = -INF;

        foreach ($this->stackedExtremes() as [$low, $high]) {
            $min = min($min, $low);
            $max = max($max, $high);
        }
        if (!is_finite($min) || !is_finite($max)) {
            $min = $max = 0.0;
        }
        if ($this->beginAtZero) {
            $min = min($min, 0.0);
            $max = max($max, 0.0);
        } elseif ($max - $min < 1e-9 && $max !== 0.0) {
            // One value: Chart.js widens the range by 5% each way.
            $offset = abs($max * 0.05);
            [$min, $max] = [$min - $offset, $max + $offset];
        }

        return LinearScale::nice($min, $max, max(1, $this->grid));
    }

    /** Resolves the x index nearest to a pointer x coordinate (dp), or null when outside. */
    abstract public function indexAt(float $x): ?int;

    public function format(float $value): string
    {
        $formatter = $this->valueFormatter;

        return $formatter === null ? Text::number($value) : $formatter($value);
    }

    protected function pointerHandler(): ?Closure
    {
        $onSelect = $this->onSelect;

        if ($onSelect === null) {
            return null;
        }

        return function (CanvasEventKind $kind, float $x, float $y) use ($onSelect): void {
            match ($kind) {
                CanvasEventKind::PointerDown, CanvasEventKind::PointerMove => $onSelect($this->indexAt($x)),
                CanvasEventKind::PointerCancel => $onSelect(null),
                CanvasEventKind::PointerUp => null,
            };
        };
    }

    /**
     * Per-x minimum and maximum of the values this chart stacks. Charts that do
     * not stack return the extremes of each series individually.
     *
     * @return list<array{0: float, 1: float}>
     */
    protected function stackedExtremes(): array
    {
        return array_values(array_map(
            static fn (Series $series): array => [$series->min(), $series->max()],
            array_filter($this->series, static fn (Series $series): bool => !$series->isEmpty()),
        ));
    }

    /**
     * Plot rectangle inside paddings, axis labels, x labels and legend.
     *
     * @return array{0: float, 1: float, 2: float, 3: float} x, y, width, height
     */
    protected function plotRect(): array
    {
        $axis = $this->theme->axis;
        $left = $axis->paddingLeft + $this->axisLabelWidth();
        $right = $axis->paddingRight;
        $top = $axis->paddingTop + ($this->legend === LegendPosition::Top ? self::LEGEND_HEIGHT : 0.0);
        if ($axis->fitEdgeLabels) {
            // Chart.js: half a value label above the top tick, the category
            // band under the plot, and the edge labels centred on their points.
            $top = max($axis->paddingTop, $this->axis ? 0.6 * $axis->fontSize + $axis->tickPadding : 0.0)
                + ($this->legend === LegendPosition::Top ? self::LEGEND_HEIGHT : 0.0);
            if ($this->labels !== []) {
                [$left, $right] = $this->fitCategoryPadding($left, $right);
            }
        }
        $top += $this->valueLabels || ($this->highlight !== null && !$this->tooltip) ? self::VALUE_LABEL_SIZE + 4.0 : 0.0;
        $bottom = $axis->paddingBottom
            + ($this->labels !== [] ? ($axis->fitEdgeLabels ? $axis->tickLength + 2 * $axis->tickPadding + 1.2 * $axis->fontSize : $axis->xLabelHeight) : 0.0)
            + ($this->legend === LegendPosition::Bottom ? self::LEGEND_HEIGHT : 0.0);

        return [
            $left,
            $top,
            max(1.0, $this->width - $left - $right),
            max(1.0, $this->height - $top - $bottom),
        ];
    }

    /**
     * Whether categories sit in the middle of equal bands (Chart.js `offset`,
     * the bar default) instead of on the plot edges.
     */
    protected function categoryOffset(): bool
    {
        return false;
    }

    /**
     * Chart.js `Scale._calculatePadding` + `fitBoxes` for the category axis:
     * the first and last labels may stick out of the plot by what is left of
     * half their width (+ `ticks.padding`) after the tick's own offset from
     * the plot edge, scaled by `width / (width - offset)`. The layout repeats
     * until the plot width settles, like Chart.js' two fit passes.
     *
     * @return array{0: float, 1: float} left and right plot insets
     */
    private function fitCategoryPadding(float $left, float $right): array
    {
        $axis = $this->theme->axis;
        $count = count($this->labels);
        $first = $this->categoryLabelWidth($this->labels[0]) / 2;
        $last = $this->categoryLabelWidth($this->labels[$count - 1]) / 2;
        $boxLeft = $left;
        $boxRight = $right;
        for ($pass = 0; $pass < 4; $pass++) {
            $width = max(1.0, $this->width - $left - $right);
            $offset = $this->categoryOffset() ? $width / $count / 2 : 0.0;
            $padLeft = max(($first - $offset + $axis->tickPadding) * $width / max(1.0, $width - $offset), 0.0);
            $padRight = max(($last - $offset + $axis->tickPadding) * $width / max(1.0, $width - $offset), 0.0);
            $nextLeft = max($boxLeft, $axis->paddingLeft + $padLeft);
            $nextRight = max($boxRight, $axis->paddingRight + $padRight);
            if (abs($nextLeft - $left) < 1e-9 && abs($nextRight - $right) < 1e-9) {
                break;
            }
            [$left, $right] = [$nextLeft, $nextRight];
        }

        return [$left, $right];
    }

    protected function axisLabelWidth(): float
    {
        if (!$this->axis) {
            return 0.0;
        }

        $widest = 0.0;
        $axis = $this->theme->axis;

        foreach ($this->scale()->ticks() as $tick) {
            $widest = max($widest, $axis->fitEdgeLabels
                ? Text::measure($this->format($tick), $axis->fontSize, 400, $this->theme->fontFamily)
                : Text::width($this->format($tick), $axis->fontSize));
        }

        return $widest + ($axis->fitEdgeLabels ? $axis->tickLength + 2 * $axis->tickPadding : $axis->labelGap);
    }

    /** Width of a category label in the chart font (Chart.js measures it). */
    private function categoryLabelWidth(string $label): float
    {
        $axis = $this->theme->axis;

        return Text::measure($label, $axis->fontSize, match ($axis->xLabelWeight) {
            FontWeight::Bold => 700,
            FontWeight::SemiBold => 600,
            FontWeight::Medium => 500,
            default => 400,
        }, $this->theme->fontFamily);
    }

    protected function hasData(): bool
    {
        foreach ($this->series as $series) {
            if (!$series->isEmpty()) {
                return true;
            }
        }

        return false;
    }

    /** Grid lines, baseline and axis tick labels. */
    protected function paintGrid(Canvas $canvas, LinearScale $scale, float $px, float $py, float $pw, float $ph): Canvas
    {
        $axis = $this->theme->axis;
        foreach ($scale->ticks() as $tick) {
            $y = round($scale->position($tick, $py + $ph, $py), 2);

            // The solid baseline is 0, or the range edge nearest to it.
            $from = $px - $axis->tickLength;
            // Chart.js `_alignPixel`: a 1px line on a whole pixel's centre.
            // Solid (Chart.js-style) grids align too; dashed default grids keep their position.
            $line = $axis->fitEdgeLabels || $axis->gridDash <= 0.0 ? round($y - 0.5) + 0.5 : $y;
            if ($tick === max($scale->min, min($scale->max, 0.0))) {
                $canvas = $canvas->line($from, $line, $px + $pw, $line, $this->theme->grid, 1);
            } elseif ($this->grid > 0) {
                $canvas = $axis->gridDash > 0.0
                    ? $canvas->dashedLine($from, $line, $px + $pw, $line, $this->theme->grid, 1, $axis->gridDash, $axis->gridDash)
                    : $canvas->line($from, $line, $px + $pw, $line, $this->theme->grid, 1);
            }

            if ($this->axis) {
                $canvas = $canvas->label(
                    $this->format($tick),
                    $px - ($axis->fitEdgeLabels ? $axis->tickLength + $axis->tickPadding : $axis->labelGap),
                    // Chart.js value labels: `textBaseline: 'middle'` on the tick.
                    $y + ($axis->fitEdgeLabels ? Text::middle($axis->fontSize, $this->theme->fontFamily) : Text::baselineOffset($axis->fontSize)),
                    $axis->fontSize,
                    $this->theme->muted,
                    TextAlign::Right,
                    FontWeight::Regular,
                );
            }
        }

        if ($axis->borderColor !== null) {
            // Chart.js axis borders: the value axis on the left, the category axis below.
            $x = round($px - 0.5) + 0.5;
            $bottom = round($py + $ph - 0.5) + 0.5;
            $canvas = $canvas
                ->line($x, $py, $x, $py + $ph, $axis->borderColor, 1)
                ->line($px, $bottom, $px + $pw, $bottom, $axis->borderColor, 1);
        }

        return $canvas;
    }

    /**
     * Category labels under the plot.
     *
     * @param list<float> $centers x center of each category
     */
    protected function paintLabels(Canvas $canvas, array $centers, float $px, float $pw, float $baseline): Canvas
    {
        $count = count($centers);
        $last = $count - 1;
        $axis = $this->theme->axis;
        if ($axis->fitEdgeLabels) {
            // Chart.js `_computeLabelItems` (bottom axis): `textBaseline:
            // 'middle'` half a line (font.lineHeight 1.2) under tick + padding.
            [, $py, , $ph] = $this->plotRect();
            $baseline = $py + $ph + $axis->tickLength + $axis->tickPadding + 0.6 * $axis->fontSize + Text::middle($axis->fontSize, $this->theme->fontFamily);
        }

        foreach ($this->labels as $index => $label) {
            if ($index > $last) {
                break;
            }

            $x = $centers[$index];
            $align = TextAlign::Center;

            if ($axis->fitEdgeLabels) {
                // The plot already leaves room for the edge labels.
            } elseif ($count > 1 && $index === 0 && $x - Text::width($label, $this->theme->axis->fontSize) / 2 < $px) {
                $align = TextAlign::Left;
                $x = $px;
            } elseif ($count > 1 && $index === $last && $x + Text::width($label, $this->theme->axis->fontSize) / 2 > $px + $pw) {
                $align = TextAlign::Right;
                $x = $px + $pw;
            }

            $canvas = $canvas->label(
                $label,
                $x,
                $baseline,
                $this->theme->axis->fontSize,
                $this->theme->muted,
                $align,
                $axis->xLabelWeight,
            );
        }

        return $canvas;
    }

    /** Series swatches and names in one row at the top or bottom. */
    protected function paintLegend(Canvas $canvas, float $px, float $pw): Canvas
    {
        if ($this->legend === LegendPosition::None || $this->series === []) {
            return $canvas;
        }

        $y = $this->legend === LegendPosition::Top
            ? self::PADDING + self::LEGEND_HEIGHT / 2
            : $this->height - self::PADDING - self::LEGEND_HEIGHT / 2;
        $x = $px;

        foreach ($this->series as $index => $series) {
            $color = $this->theme->seriesColor($series->color, $index);
            $canvas = $canvas
                ->circle($x + 4.0, $y, 4.0, $color)
                ->label(
                    $series->label,
                    $x + 12.0,
                    $y + Text::baselineOffset(self::AXIS_LABEL_SIZE),
                    self::AXIS_LABEL_SIZE,
                    $this->theme->muted,
                    TextAlign::Left,
                    FontWeight::Medium,
                );
            $x += 12.0 + Text::width($series->label, self::AXIS_LABEL_SIZE) + 14.0;

            if ($x > $px + $pw) {
                break;
            }
        }

        return $canvas;
    }

    /** Baseline plus a muted, localized "No data" label for charts without values. */
    protected function paintEmpty(Canvas $canvas, float $px, float $py, float $pw, float $ph): Canvas
    {
        return $canvas
            ->line($px, $py + $ph, $px + $pw, $py + $ph, $this->theme->grid, 1)
            ->label(
                \Pam\Desktop\I18n\FrameworkText::get('chart.empty'),
                $px + $pw / 2,
                $py + $ph / 2 + Text::baselineOffset(12.0),
                12.0,
                $this->theme->muted,
                TextAlign::Center,
                FontWeight::Medium,
            );
    }

    /** Formatted value centered above a point. */
    protected function paintValueLabel(Canvas $canvas, float $value, float $x, float $y, bool $emphasis = false): Canvas
    {
        return $canvas->label(
            $this->format($value),
            $x,
            $y - self::MARKER_RADIUS - 6.0,
            self::VALUE_LABEL_SIZE,
            $this->theme->ink,
            TextAlign::Center,
            $emphasis ? FontWeight::SemiBold : FontWeight::Medium,
        );
    }
}
