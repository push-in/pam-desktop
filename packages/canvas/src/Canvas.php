<?php

declare(strict_types=1);

namespace Pam\Desktop\Canvas;

use InvalidArgumentException;
use Pam\Desktop\Canvas\Internal\Numbers;

/**
 * Fluent, immutable builder of a display list. Every call returns a new
 * canvas; call {@see scene()} to obtain the validated {@see CanvasScene}.
 *
 * Angles are degrees, 0° at 12 o'clock, clockwise positive. Colors are CSS
 * hex (`#rgb`, `#rrggbb`, `#rrggbbaa`). Text `y` coordinates are baselines.
 */
final class Canvas
{
    public const int MAX_POINTS = 512;

    /** @var list<CanvasCommand> */
    private array $commands = [];

    // Filled shapes.

    public function fillRect(float $x, float $y, float $width, float $height, string $color): self
    {
        return $this->add(DisplayCommandKind::FillRect, $x, $y, $width, $height, Color::assert($color));
    }

    public function roundRect(float $x, float $y, float $width, float $height, float $radius, string $color): self
    {
        return $this->add(
            DisplayCommandKind::RoundRect,
            $x,
            $y,
            $width,
            $height,
            max(0, $radius),
            Color::assert($color),
        );
    }

    public function circle(float $x, float $y, float $radius, string $color): self
    {
        return $this->add(DisplayCommandKind::Circle, $x, $y, $radius, Color::assert($color));
    }

    /**
     * Filled ring segment (donut slice). `$innerRadius` 0 draws a pie slice.
     */
    public function sector(
        float $cx,
        float $cy,
        float $outerRadius,
        float $innerRadius,
        float $startDegrees,
        float $sweepDegrees,
        string $color,
    ): self {
        return $this->add(
            DisplayCommandKind::Sector,
            $cx,
            $cy,
            $outerRadius,
            max(0, $innerRadius),
            $startDegrees,
            $sweepDegrees,
            Color::assert($color),
        );
    }

    /**
     * Closed, filled polygon.
     *
     * @param list<array{0: float, 1: float}> $points at least three
     */
    public function polygon(array $points, string $color): self
    {
        return $this->add(DisplayCommandKind::Polygon, self::encodePoints($points, 3), Color::assert($color));
    }

    /**
     * Fills or strokes a {@see Path}.
     */
    public function path(Path $path, string $color, float $lineWidth = 1, PathMode $mode = PathMode::Fill): self
    {
        return $this->add(DisplayCommandKind::Path, $path->spec(), Color::assert($color), $lineWidth, $mode->value);
    }

    /**
     * Path filled with a linear gradient (area charts).
     */
    public function gradientPath(
        Path $path,
        float $x0,
        float $y0,
        float $x1,
        float $y1,
        string $colorStart,
        string $colorEnd,
    ): self {
        return $this->add(
            DisplayCommandKind::GradientPath,
            $path->spec(),
            $x0,
            $y0,
            $x1,
            $y1,
            Color::assert($colorStart),
            Color::assert($colorEnd),
        );
    }

    // -- strokes --------------------------------------------------------------

    public function line(float $x1, float $y1, float $x2, float $y2, string $color, float $lineWidth = 1): self
    {
        return $this->add(DisplayCommandKind::Line, $x1, $y1, $x2, $y2, Color::assert($color), $lineWidth);
    }

    /**
     * Dashed straight line, for grids and baselines.
     */
    public function dashedLine(
        float $x1,
        float $y1,
        float $x2,
        float $y2,
        string $color,
        float $lineWidth = 1,
        float $dash = 4,
        float $gap = 4,
    ): self {
        if ($dash <= 0 || $gap <= 0) {
            throw new InvalidArgumentException('Canvas dash and gap must be positive.');
        }

        return $this->add(
            DisplayCommandKind::DashedLine,
            $x1,
            $y1,
            $x2,
            $y2,
            Color::assert($color),
            $lineWidth,
            $dash,
            $gap,
        );
    }

    /**
     * Stroked arc of a circle.
     */
    public function arc(
        float $cx,
        float $cy,
        float $radius,
        float $startDegrees,
        float $sweepDegrees,
        string $color,
        float $lineWidth = 1,
        LineCap $cap = LineCap::Butt,
    ): self {
        return $this->add(
            DisplayCommandKind::Arc,
            $cx,
            $cy,
            $radius,
            $startDegrees,
            $sweepDegrees,
            Color::assert($color),
            $lineWidth,
            $cap->value,
        );
    }

    /**
     * Open stroked polyline.
     *
     * @param list<array{0: float, 1: float}> $points at least two
     */
    public function polyline(
        array $points,
        string $color,
        float $lineWidth = 1,
        LineCap $cap = LineCap::Round,
    ): self {
        return $this->add(
            DisplayCommandKind::Polyline,
            self::encodePoints($points, 2),
            Color::assert($color),
            $lineWidth,
            $cap->value,
        );
    }

    // -- text -----------------------------------------------------------------

    /**
     * Left-aligned regular text; `$y` is the baseline.
     */
    public function text(string $text, float $x, float $y, float $size, string $color): self
    {
        return $this->add(DisplayCommandKind::Text, $text, $x, $y, $size, Color::assert($color));
    }

    /**
     * Aligned, weighted text; `$y` is the baseline.
     */
    public function label(
        string $text,
        float $x,
        float $y,
        float $size,
        string $color,
        TextAlign $align = TextAlign::Left,
        FontWeight $weight = FontWeight::Regular,
    ): self {
        return $this->add(
            DisplayCommandKind::Label,
            $text,
            $x,
            $y,
            $size,
            Color::assert($color),
            $align->value,
            $weight->value,
        );
    }

    // -- output ---------------------------------------------------------------

    /**
     * Validates and freezes the display list.
     */
    public function scene(): CanvasScene
    {
        return new CanvasScene($this->commands);
    }

    /**
     * Encodes points as `"x,y x,y …"`.
     *
     * @param list<array{0: float, 1: float}> $points
     */
    public static function encodePoints(array $points, int $minimum = 2): string
    {
        $count = count($points);

        if ($count < $minimum || $count > self::MAX_POINTS) {
            throw new InvalidArgumentException("Canvas point lists need {$minimum} to 512 points.");
        }

        $encoded = [];

        foreach ($points as $point) {
            $encoded[] = Numbers::format((float) $point[0]) . ',' . Numbers::format((float) $point[1]);
        }

        return implode(' ', $encoded);
    }

    private function add(DisplayCommandKind $kind, float|int|string ...$arguments): self
    {
        if (count($this->commands) >= CanvasScene::MAX_COMMANDS) {
            throw new InvalidArgumentException('Canvas scenes are limited to 10,000 commands.');
        }
        $copy = clone $this;
        $copy->commands[] = new CanvasCommand($kind, array_values($arguments));

        return $copy;
    }
}
