<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts;

use InvalidArgumentException;
use Pam\Desktop\Canvas\Canvas;
use Pam\Desktop\Canvas\FontWeight;
use Pam\Desktop\Canvas\TextAlign;
use Pam\Desktop\Charts\Internal\Text;

/**
 * Donut (ring) chart with gapped slices, a titled center and an optional legend.
 *
 * ```php
 * DonutChart::make(size: 160)
 *     ->slices([Slice::make('Pix', 70)->color('#19c5ff'), Slice::make('Cartão', 30)])
 *     ->thickness(18)
 *     ->center(title: 'R$ 1.234', subtitle: 'este mês');
 * ```
 */
final class DonutChart extends Chart
{
    public const float LEGEND_ROW_HEIGHT = 18.0;
    public const float TITLE_SIZE = 20.0;
    public const float SUBTITLE_SIZE = 12.0;

    /** @var list<Slice> */
    private array $slices = [];

    private float $size;
    private float $thickness = 18.0;
    private float $gap = 2.0;
    private ?string $title = null;
    private ?string $subtitle = null;
    private LegendPosition $legend = LegendPosition::None;
    private bool $legendValues = true;
    private float $legendSpacing = self::LEGEND_ROW_HEIGHT;
    private float $legendWidth = 140.0;
    private bool $legendInline = false;

    protected function __construct(float $size, float $width, float $height)
    {
        parent::__construct($width, $height);
        $this->size = $size;
    }

    public static function make(float|int $size): self
    {
        return new self((float) $size, (float) $size, (float) $size);
    }

    /**
     * @param array<array-key, Slice> $slices
     */
    public function slices(array $slices): self
    {
        if (count($slices) > 64) {
            throw new InvalidArgumentException('Donut charts accept at most 64 slices.');
        }

        $copy = clone $this;
        $copy->slices = array_values(array_map(
            static fn (Slice $slice): Slice => $slice,
            $slices,
        ));

        return $copy;
    }

    public function slice(Slice $slice): self
    {
        return $this->slices([...$this->slices, $slice]);
    }

    /** Ring thickness in dp. */
    public function thickness(float|int $thickness): self
    {
        if ($thickness <= 0 || $thickness > $this->size / 2) {
            throw new InvalidArgumentException('Donut thickness must be positive and fit inside the ring.');
        }

        $copy = clone $this;
        $copy->thickness = (float) $thickness;

        return $copy;
    }

    /** Space between slices in dp (measured on the ring's middle radius). */
    public function gap(float|int $gap): self
    {
        $copy = clone $this;
        $copy->gap = max(0.0, min(16.0, (float) $gap));

        return $copy;
    }

    /** Title and subtitle drawn in the hole. */
    public function center(?string $title, ?string $subtitle = null): self
    {
        $copy = clone $this;
        $copy->title = $title === null ? null : Series::validLabel($title);
        $copy->subtitle = $subtitle === null ? null : Series::validLabel($subtitle);

        return $copy;
    }

    /** Recomputes the legend size: its metrics come from the theme. */
    public function theme(ChartTheme $theme): static
    {
        return parent::theme($theme)->legend($this->legend);
    }

    /** Legend rows below the ring; the chart grows in height to fit them. */
    public function legend(LegendPosition $position): self
    {
        $copy = clone $this;
        $copy->legend = $position;
        $rows = $position === LegendPosition::None ? 0 : ($copy->legendInline && $position !== LegendPosition::Right ? count($copy->inlineRows()) : count($this->slices));
        $copy->height = $this->size + ($rows === 0 || $position === LegendPosition::Right ? 0.0 : $rows * $this->legendSpacing + 8.0);
        $copy->width = $this->size + ($position === LegendPosition::Right ? $this->legendWidth : 0.0);

        return $copy;
    }

    /**
     * Top/Bottom legend laid out horizontally, centered and wrapping at the
     * chart width (Chart.js default), instead of one row per slice. Font,
     * marker and gaps come from the theme's {@see LegendStyle}; the row
     * height is {@see self::legendSpacing()}.
     */
    public function legendInline(bool $inline = true): self
    {
        $copy = clone $this;
        $copy->legendInline = $inline;
        return $copy->legend($this->legend);
    }

    /** @return list<list<array{0:int,1:float}>> slice indexes and item widths per row */
    private function inlineRows(): array
    {
        $style = $this->theme->legend;
        $rows = [[]];
        $used = 0.0;
        foreach ($this->slices as $index => $slice) {
            $item = 2 * $style->markerRadius + $style->markerGap + Text::width($slice->label, $style->fontSize)
                + ($this->legendValues ? $style->valueGap + Text::width('100%', $style->fontSize) : 0.0);
            $needed = $item + ($rows[array_key_last($rows)] === [] ? 0.0 : $style->itemGap);
            if ($rows[array_key_last($rows)] !== [] && $used + $needed > $this->width) {
                $rows[] = [];
                $used = 0.0;
                $needed = $item;
            }
            $rows[array_key_last($rows)][] = [$index, $item];
            $used += $needed;
        }
        return $rows;
    }

    public function legendValues(bool $visible): self
    {
        $copy = clone $this;
        $copy->legendValues = $visible;
        return $copy->legend($this->legend);
    }

    public function legendSpacing(float $spacing): self
    {
        if (!is_finite($spacing) || $spacing < 12 || $spacing > 80) throw new InvalidArgumentException('Legend spacing must be between 12 and 80.');
        $copy = clone $this;
        $copy->legendSpacing = $spacing;
        return $copy->legend($this->legend);
    }

    public function total(): float
    {
        return array_sum(array_map(static fn (Slice $slice): float => $slice->value, $this->slices));
    }

    protected function draw(Canvas $canvas): Canvas
    {
        $canvas = $this->paintSurface($canvas);
        $ringTop = $this->legend === LegendPosition::Top ? $this->height - $this->size : 0.0;
        $cx = $this->size / 2;
        $cy = $ringTop + $this->size / 2;
        $outer = $this->size / 2;
        $inner = $outer - $this->thickness;
        $total = $this->total();

        if ($total <= 0.0) {
            $canvas = $canvas
                ->sector($cx, $cy, $outer, $inner, 0, 360, $this->theme->grid)
                ->label(\Pam\Desktop\I18n\FrameworkText::get('chart.empty'), $cx, $cy + Text::baselineOffset(self::SUBTITLE_SIZE), self::SUBTITLE_SIZE, $this->theme->muted, TextAlign::Center, FontWeight::Medium);

            return $this->paintLegend($canvas, $ringTop);
        }

        $middle = ($outer + $inner) / 2;
        $gapDegrees = $this->gap <= 0.0 ? 0.0 : $this->gap / (2 * M_PI * $middle) * 360;
        $visible = count(array_filter($this->slices, static fn (Slice $slice): bool => $slice->value > 0.0));
        $start = 0.0;

        foreach ($this->slices as $index => $slice) {
            if ($slice->value <= 0.0) {
                continue;
            }

            $sweep = $slice->value / $total * 360;
            $trim = $visible > 1 ? min($gapDegrees, $sweep / 2) : 0.0;
            $canvas = $canvas->sector(
                $cx,
                $cy,
                $outer,
                $inner,
                round($start + $trim / 2, 3),
                round($sweep - $trim, 3),
                $this->theme->seriesColor($slice->color, $index),
            );
            $start += $sweep;
        }

        $canvas = $this->paintCenter($canvas, $cx, $cy);

        return $this->paintLegend($canvas, $ringTop);
    }

    private function paintCenter(Canvas $canvas, float $cx, float $cy): Canvas
    {
        if ($this->title === null && $this->subtitle === null) {
            return $canvas;
        }

        $titleSize = min(self::TITLE_SIZE, $this->size / 6);
        $subtitleSize = min(self::SUBTITLE_SIZE, $this->size / 10);

        if ($this->title !== null && $this->subtitle !== null) {
            return $canvas
                ->label($this->title, $cx, $cy - 2.0, $titleSize, $this->theme->ink, TextAlign::Center, FontWeight::Bold)
                ->label($this->subtitle, $cx, $cy + $subtitleSize + 2.0, $subtitleSize, $this->theme->muted, TextAlign::Center, FontWeight::Regular);
        }

        return $this->title !== null
            ? $canvas->label($this->title, $cx, $cy + Text::baselineOffset($titleSize), $titleSize, $this->theme->ink, TextAlign::Center, FontWeight::Bold)
            : $canvas->label((string) $this->subtitle, $cx, $cy + Text::baselineOffset($subtitleSize), $subtitleSize, $this->theme->muted, TextAlign::Center, FontWeight::Regular);
    }

    private function paintLegend(Canvas $canvas, float $ringTop): Canvas
    {
        if ($this->legend === LegendPosition::None || $this->slices === []) {
            return $canvas;
        }

        $total = $this->total();
        $right = $this->legend === LegendPosition::Right;
        $style = $this->theme->legend;
        $font = $style->fontSize;
        $baseline = Text::baselineOffset($font);
        if ($this->legendInline && !$right) {
            $y = ($this->legend === LegendPosition::Top ? 4.0 : $ringTop + $this->size + 8.0) + $this->legendSpacing / 2;
            foreach ($this->inlineRows() as $row) {
                $width = array_sum(array_column($row, 1)) + $style->itemGap * (count($row) - 1);
                $x = ($this->width - $width) / 2;
                foreach ($row as [$index, $item]) {
                    $slice = $this->slices[$index];
                    $canvas = $canvas
                        ->circle($x + $style->markerRadius, $y, $style->markerRadius, $this->theme->seriesColor($slice->color, $index))
                        ->label($slice->label, $x + 2 * $style->markerRadius + $style->markerGap, $y + $baseline, $font, $this->theme->muted, TextAlign::Left, $style->fontWeight);
                    if ($this->legendValues) {
                        $share = $total <= 0.0 ? 0.0 : $slice->value / $total * 100;
                        $canvas = $canvas->label(Text::number($share) . '%', $x + $item, $y + $baseline, $font, $this->theme->ink, TextAlign::Right, $style->fontWeight);
                    }
                    $x += $item + $style->itemGap;
                }
                $y += $this->legendSpacing;
            }
            return $canvas;
        }
        $x = $right ? $this->size + 8.0 : 0.0;
        $y = $right ? max($this->legendSpacing / 2, ($this->height - count($this->slices) * $this->legendSpacing) / 2 + $this->legendSpacing / 2)
            : ($this->legend === LegendPosition::Top ? 4.0 : $ringTop + $this->size + 8.0) + $this->legendSpacing / 2;

        foreach ($this->slices as $index => $slice) {
            $share = $total <= 0.0 ? 0.0 : $slice->value / $total * 100;
            $canvas = $canvas
                ->circle($x + 8.0, $y, $right ? 6.0 : 4.0, $this->theme->seriesColor($slice->color, $index))
                ->label($slice->label, $x + ($right ? 24.0 : 18.0), $y + $baseline, $font, $this->theme->muted, TextAlign::Left, $style->fontWeight);
            if ($this->legendValues) $canvas = $canvas->label(Text::number($share) . '%', $this->width - 4.0, $y + $baseline, $font, $this->theme->ink, TextAlign::Right, $style->fontWeight);
            $y += $this->legendSpacing;
        }

        return $canvas;
    }
}
