<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts;

use InvalidArgumentException;
use Pam\Desktop\Canvas\Canvas;
use Pam\Desktop\Canvas\FontWeight;
use Pam\Desktop\Canvas\Path;
use Pam\Desktop\Canvas\PathMode;
use Pam\Desktop\Canvas\TextAlign;
use Pam\Desktop\Charts\Internal\Text;
use Pam\Desktop\PropKey;

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
    private float $cornerRadius = 0.0;
    private float $spacing = 0.0;
    private float $borderWidth = 0.0;
    private ?string $borderColor = null;
    private ?float $cutout = null;
    private ?float $frameWidth = null;
    private ?float $frameHeight = null;
    /** Set on the copy {@see toElement()} paints: the inline legend is native text there. */
    private bool $nativeLegend = false;

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

    /**
     * Chart.js doughnut layout in a `$width` x `$height` canvas: a right
     * legend takes its measured width (items + theme `itemGap` + 10), the
     * ring fills and centres in the rest (radius from the smaller side, less
     * {@see spacing()} and the border), legend rows are centred vertically
     * at {@see legendSpacing()} pitch. Without a frame the ring is `size`.
     */
    public function frame(float|int $width, float|int $height): self
    {
        if ($width <= 0 || $height <= 0 || $width > 4096 || $height > 4096) {
            throw new InvalidArgumentException('Donut frame must be between 1 and 4096.');
        }
        $copy = clone $this;
        $copy->frameWidth = (float) $width;
        $copy->frameHeight = (float) $height;

        return $copy->legend($this->legend);
    }

    /** Chart.js `cutout`: inner radius as a fraction of the outer one (framed rings). */
    public function cutout(float $ratio): self
    {
        if (!is_finite($ratio) || $ratio < 0.0 || $ratio >= 1.0) {
            throw new InvalidArgumentException('Donut cutout must be in [0, 1).');
        }
        $copy = clone $this;
        $copy->cutout = $ratio;

        return $copy;
    }

    /** Chart.js `borderRadius`: rounded slice corners, clamped to half the ring and the slice. */
    public function cornerRadius(float|int $radius): self
    {
        $copy = clone $this;
        $copy->cornerRadius = max(0.0, min(64.0, (float) $radius));

        return $copy;
    }

    /**
     * Chart.js `spacing`: the gap between slices, kept even along the ring
     * (each slice's angle shrinks around its middle radius) instead of {@see gap()}.
     */
    public function spacing(float|int $spacing): self
    {
        $copy = clone $this;
        $copy->spacing = max(0.0, min(32.0, (float) $spacing));

        return $copy;
    }

    /** Chart.js `borderWidth`/`borderColor`: an outline stroked around each slice. */
    public function border(float|int $width, string $color): self
    {
        $copy = clone $this;
        $copy->borderWidth = max(0.0, min(16.0, (float) $width));
        $copy->borderColor = Internal\Color::normalize($color);

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
        if ($this->frameWidth !== null && $this->frameHeight !== null) {
            $copy->width = $this->frameWidth;
            $copy->height = $this->frameHeight;
        }

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
            $weight = match ($style->fontWeight) { FontWeight::Medium => 500, FontWeight::SemiBold => 600, FontWeight::Bold => 700, default => 400 };
            $item = $style->markerWidth() + $style->markerGap + Text::measure($slice->label, $style->fontSize, $weight, $this->theme->fontFamily)
                + ($this->legendValues ? $style->valueGap + Text::measure('100%', $style->fontSize, $weight, $this->theme->fontFamily) : 0.0);
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
        if ($this->frameWidth !== null) {
            return $this->drawFramed($this->paintSurface($canvas));
        }
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
        if ($this->legendInline && !$right && $this->nativeLegend) {
            return $canvas;
        }
        if ($this->legendInline && !$right) {
            $y = ($this->legend === LegendPosition::Top ? 4.0 : $ringTop + $this->size + 8.0) + $this->legendSpacing / 2;
            foreach ($this->inlineRows() as $row) {
                $width = array_sum(array_column($row, 1)) + $style->itemGap * (count($row) - 1);
                $x = ($this->width - $width) / 2;
                foreach ($row as [$index, $item]) {
                    $slice = $this->slices[$index];
                    $canvas = $canvas
                        ->circle($x + $style->markerWidth() / 2, $y, $style->markerRadius, $this->theme->seriesColor($slice->color, $index))
                        ->label($slice->label, $x + $style->markerWidth() + $style->markerGap, $y + $baseline, $font, $style->color ?? $this->theme->muted, TextAlign::Left, $style->fontWeight);
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
                ->label($slice->label, $x + ($right ? 24.0 : 18.0), $y + $baseline, $font, $style->color ?? $this->theme->muted, TextAlign::Left, $style->fontWeight);
            if ($this->legendValues) $canvas = $canvas->label(Text::number($share) . '%', $this->width - 4.0, $y + $baseline, $font, $this->theme->ink, TextAlign::Right, $style->fontWeight);
            $y += $this->legendSpacing;
        }

        return $canvas;
    }

    /**
     * An inline Top/Bottom legend is laid out as native text: the host measures
     * the labels (font fallback, kerning, its text cache), so items are placed
     * like Chart.js instead of from estimated glyph widths. The ring stays a canvas.
     */
    public function toElement(): \Pam\Desktop\Element
    {
        if (!$this->legendInline || $this->slices === [] || !in_array($this->legend, [LegendPosition::Top, LegendPosition::Bottom], true)) {
            return parent::toElement();
        }
        $ring = clone $this;
        $ring->nativeLegend = true;
        $top = $this->legend === LegendPosition::Top;
        $canvas = $ring->view()->toElement()
            ->property(PropKey::Width, $this->width)
            ->property(PropKey::Height, $this->size)
            ->property($top ? PropKey::Top : PropKey::Left, $top ? $this->height - $this->size : 0.0)
            ->property(PropKey::PositionType, \Pam\Desktop\PositionType::Absolute->value);
        $legend = $this->legendElement()
            ->property(PropKey::PositionType, \Pam\Desktop\PositionType::Absolute->value)
            ->property(PropKey::Left, 0.0)
            ->property(PropKey::Top, $top ? 4.0 : $this->size + 8.0);

        return \Pam\Desktop\UI\View::make($canvas, $legend)
            ->property(PropKey::Width, $this->width)
            ->property(PropKey::Height, $this->height);
    }

    private function legendElement(): \Pam\Desktop\Element
    {
        $style = $this->theme->legend;
        $total = $this->total();
        $marker = $style->markerWidth();
        $radius = $style->markerRadius;
        $muted = Internal\Color::toArgb($this->theme->muted);
        $items = [];
        foreach ($this->slices as $index => $slice) {
            $dot = \Pam\Desktop\UI\View::make()
                ->property(PropKey::PositionType, \Pam\Desktop\PositionType::Absolute->value)
                ->property(PropKey::Left, $marker / 2 - $radius)
                ->property(PropKey::Top, $this->legendSpacing / 2 - $radius)
                ->property(PropKey::Width, 2 * $radius)
                ->property(PropKey::Height, 2 * $radius)
                ->property(PropKey::BorderRadius, $radius)
                ->property(PropKey::BackgroundColor, Internal\Color::toArgb($this->theme->seriesColor($slice->color, $index)));
            $parts = [
                \Pam\Desktop\UI\View::make($dot)->property(PropKey::Width, $marker)->property(PropKey::Height, $this->legendSpacing),
                $this->legendText($slice->label, $muted)->property(PropKey::MarginLeft, $style->markerGap),
            ];
            if ($this->legendValues) {
                $share = $total <= 0.0 ? 0.0 : $slice->value / $total * 100;
                $parts[] = $this->legendText(Text::number($share) . '%', Internal\Color::toArgb($this->theme->ink))->property(PropKey::MarginLeft, $style->valueGap);
            }
            $items[] = \Pam\Desktop\UI\Row::make(...$parts)
                ->property(PropKey::AlignItems, \Pam\Desktop\Align::Center->value)
                ->property(PropKey::Height, $this->legendSpacing);
        }

        return \Pam\Desktop\UI\Row::make(...$items)
            ->property(PropKey::Width, $this->width)
            ->property(PropKey::FlexWrap, \Pam\Desktop\FlexWrap::Wrap->value)
            ->property(PropKey::JustifyContent, \Pam\Desktop\Justify::Center->value)
            ->property(PropKey::Gap, $style->itemGap);
    }

    private function legendText(string $text, int $color): \Pam\Desktop\Element
    {
        $style = $this->theme->legend;
        $element = \Pam\Desktop\UI\Text::make($text)
            ->property(PropKey::FontSize, $style->fontSize)
            ->property(PropKey::FontWeight, match ($style->fontWeight) { FontWeight::Medium => 500, FontWeight::SemiBold => 600, FontWeight::Bold => 700, default => 400 })
            ->property(PropKey::TextColor, $color)
            ->property(PropKey::NumberOfLines, 1);

        return $this->theme->fontFamily === '' ? $element : $element->property(PropKey::FontFamily, $this->theme->fontFamily);
    }

    /** Chart.js doughnut geometry (see {@see frame()}). */
    private function drawFramed(Canvas $canvas): Canvas
    {
        $style = $this->theme->legend;
        $weight = match ($style->fontWeight) {
            FontWeight::Bold => 700,
            FontWeight::SemiBold => 600,
            FontWeight::Medium => 500,
            default => 400,
        };
        $right = $this->legend === LegendPosition::Right && $this->slices !== [];
        $widest = 0.0;
        foreach ($this->slices as $slice) {
            $widest = max($widest, $style->markerWidth() + $style->markerGap
                + Text::measure($slice->label, $style->fontSize, $weight, $this->theme->fontFamily));
        }
        $areaWidth = max(1.0, $this->width - ($right ? $widest + $style->itemGap + 10.0 : 0.0));
        $half = $this->spacing / 2;
        $outer = max(0.0, (min($areaWidth, $this->height) - $this->spacing) / 2 - $this->borderWidth / 2);
        $inner = $this->cutout !== null ? $outer * $this->cutout : max(0.0, $outer - $this->thickness);
        $cx = $areaWidth / 2;
        $cy = $this->height / 2;
        $total = $this->total();
        $angle = -M_PI / 2;
        foreach ($this->slices as $index => $slice) {
            if ($slice->value <= 0.0 || $total <= 0.0) {
                continue;
            }
            $sweep = $slice->value / $total * 2 * M_PI;
            $path = self::arcPath($cx, $cy, $outer + $half, $inner > 0.0 ? $inner + $half : 0.0, $inner, $angle, $angle + $sweep, $half, $this->cornerRadius);
            $color = $this->theme->seriesColor($slice->color, $index);
            $canvas = $canvas->path($path, $color);
            if ($this->borderWidth > 0.0 && $this->borderColor !== null) {
                $canvas = $canvas->path($path, $this->borderColor, $this->borderWidth, PathMode::Stroke);
            }
            $angle += $sweep;
        }
        $canvas = $this->paintCenter($canvas, $cx, $cy);
        if (!$right) {
            return $canvas;
        }
        $font = $style->fontSize;
        $count = count($this->slices);
        $gap = max(0.0, $this->legendSpacing - $font);
        $top = ($this->height - ($count * $font + ($count - 1) * $gap)) / 2;
        $left = $areaWidth + $style->itemGap;
        // Chart.js legend labels: `textBaseline: 'middle'` on the item's middle.
        $baseline = Text::middle($font, $this->theme->fontFamily);
        foreach ($this->slices as $index => $slice) {
            $middle = $top + $font / 2;
            $canvas = $canvas
                ->circle($left + $style->markerWidth() / 2, $middle, $style->markerRadius, $this->theme->seriesColor($slice->color, $index))
                ->label($slice->label, $left + $style->markerWidth() + $style->markerGap, $middle + $baseline, $font, $style->color ?? $this->theme->muted, TextAlign::Left, $style->fontWeight);
            $top += $this->legendSpacing;
        }

        return $canvas;
    }

    /**
     * Chart.js `pathArc`: a ring slice from `$start` to `$end` (radians,
     * clockwise from 3 o'clock) between the drawn radii, its angle narrowed by
     * `$spacing` (half of Chart.js `spacing`) around the middle radius and its
     * four corners rounded by `$radius`.
     */
    private static function arcPath(float $x, float $y, float $outer, float $inner, float $elementInner, float $start, float $end, float $spacing, float $radius): Path
    {
        $alpha = $end - $start;
        $offset = 0.0;
        if ($spacing > 0.0) {
            $noSpacingInner = $elementInner > 0.0 ? $elementInner - $spacing : 0.0;
            $noSpacingOuter = $outer > 0.0 ? $outer - $spacing : 0.0;
            $average = ($noSpacingInner + $noSpacingOuter) / 2;
            $adjusted = $average !== 0.0 ? ($alpha * $average) / ($average + $spacing) : $alpha;
            $offset = ($alpha - $adjusted) / 2;
        }
        $start += $offset;
        $end -= $offset;
        $delta = $end - $start;
        $halfThickness = ($outer - $inner) / 2;
        $outerLimit = min($halfThickness, $delta * $outer / 2);
        $innerLimit = min($halfThickness, $inner > 0.0 ? $delta * $inner / 2 : 0.0);
        $outerR = max(0.0, min($radius, $outerLimit));
        $innerR = $inner > 0.0 ? max(0.0, min($radius, $innerLimit)) : 0.0;
        $point = static fn (float $r, float $t): array => [$x + $r * cos($t), $y + $r * sin($t)];
        $outerAdjusted = $outer - $outerR;
        $innerAdjusted = $inner + $innerR;
        $outerStartAngle = $start + ($outerAdjusted > 0.0 ? $outerR / $outerAdjusted : 0.0);
        $outerEndAngle = $end - ($outerAdjusted > 0.0 ? $outerR / $outerAdjusted : 0.0);
        $innerStartAngle = $start + ($innerAdjusted > 0.0 ? $innerR / $innerAdjusted : 0.0);
        $innerEndAngle = $end - ($innerAdjusted > 0.0 ? $innerR / $innerAdjusted : 0.0);

        [$sx, $sy] = $point($outer, $outerStartAngle);
        $path = Path::start($sx, $sy);
        $path = self::arcTo($path, $x, $y, $outer, $outerStartAngle, $outerEndAngle);
        if ($outerR > 0.0) {
            [$px, $py] = $point($outerAdjusted, $outerEndAngle);
            $path = self::arcTo($path, $px, $py, $outerR, $outerEndAngle, $end + M_PI / 2);
        }
        [$lx, $ly] = $point($innerAdjusted, $end);
        $path = $path->lineTo($lx, $ly);
        if ($inner > 0.0) {
            if ($innerR > 0.0) {
                [$px, $py] = $point($innerAdjusted, $innerEndAngle);
                $path = self::arcTo($path, $px, $py, $innerR, $end + M_PI / 2, $innerEndAngle + M_PI);
            }
            $path = self::arcTo($path, $x, $y, $inner, $end - $innerR / $inner, $start + $innerR / $inner, true);
            if ($innerR > 0.0) {
                [$px, $py] = $point($innerAdjusted, $innerStartAngle);
                $path = self::arcTo($path, $px, $py, $innerR, $start + M_PI, $innerStartAngle - M_PI / 2);
            }
        } else {
            $path = $path->lineTo($x, $y);
        }
        [$lx, $ly] = $point($outerAdjusted, $start);
        $path = $path->lineTo($lx, $ly);
        if ($outerR > 0.0) {
            [$px, $py] = $point($outerAdjusted, $outerStartAngle);
            $path = self::arcTo($path, $px, $py, $outerR, $start - M_PI / 2, $outerStartAngle);
        }

        return $path->close();
    }

    /**
     * Appends a circular arc (from the current point at `$from`) as cubic
     * Béziers of at most 90°.
     */
    private static function arcTo(Path $path, float $x, float $y, float $r, float $from, float $to, bool $counterClockwise = false): Path
    {
        // Canvas `arc()` semantics: the sweep runs in the given direction, less than a turn.
        $sweep = $to - $from;
        if ($counterClockwise) {
            while ($sweep > 0.0) $sweep -= 2 * M_PI;
        } else {
            while ($sweep < 0.0) $sweep += 2 * M_PI;
        }
        $steps = max(1, (int) ceil(abs($sweep) / (M_PI / 2) - 1e-9));
        $step = $sweep / $steps;
        $k = 4 / 3 * tan($step / 4);
        for ($i = 0; $i < $steps; $i++) {
            $a = $from + $i * $step;
            $b = $a + $step;
            $path = $path->curveTo(
                $x + $r * (cos($a) - $k * sin($a)), $y + $r * (sin($a) + $k * cos($a)),
                $x + $r * (cos($b) + $k * sin($b)), $y + $r * (sin($b) - $k * cos($b)),
                $x + $r * cos($b), $y + $r * sin($b),
            );
        }

        return $path;
    }
}
