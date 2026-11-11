<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

use InvalidArgumentException;

/**
 * Where a floating surface goes next to its anchor: a side (`bottom`, `top`,
 * `right`, `left`) and an alignment along it (`start`, `center`, `end`),
 * flipped to the other side when it does not fit and kept inside the window.
 */
final readonly class PopoverPlacement
{
    private const float MARGIN = 4.0;

    public string $side;
    public string $align;

    public function __construct(string $placement = 'bottom-start')
    {
        [$side, $align] = array_pad(explode('-', $placement, 2), 2, 'center');
        if (!in_array($side, ['bottom', 'top', 'right', 'left'], true) || !in_array($align, ['start', 'center', 'end'], true)) {
            throw new InvalidArgumentException("Unknown placement {$placement}: use bottom, top, right or left, optionally with -start or -end.");
        }
        $this->side = $side;
        $this->align = $align;
    }

    /**
     * The window position of a `$width` × `$height` surface for an anchor at
     * (`$x`, `$y`, `$anchorWidth`, `$anchorHeight`) in a window of
     * `$windowWidth` × `$windowHeight`, `$offset` points away from it.
     *
     * @return array{x: float, y: float, side: string}
     */
    public function place(
        float $x,
        float $y,
        float $anchorWidth,
        float $anchorHeight,
        float $width,
        float $height,
        float $windowWidth,
        float $windowHeight,
        float $offset = 6.0,
    ): array {
        // A window that has not reported its size yet does not limit anything.
        $windowWidth = $windowWidth > 0.0 ? $windowWidth : INF;
        $windowHeight = $windowHeight > 0.0 ? $windowHeight : INF;
        $vertical = $this->side === 'bottom' || $this->side === 'top';
        $side = $this->side;
        if ($vertical) {
            $below = $y + $anchorHeight + $offset;
            $above = $y - $offset - $height;
            $fitsBelow = $below + $height <= $windowHeight - self::MARGIN;
            $fitsAbove = $above >= self::MARGIN;
            if ($side === 'bottom' && !$fitsBelow && $fitsAbove) {
                $side = 'top';
            } elseif ($side === 'top' && !$fitsAbove && $fitsBelow) {
                $side = 'bottom';
            }
            $top = $side === 'bottom' ? $below : $above;
            $left = $this->aligned($x, $anchorWidth, $width);

            return ['x' => self::clamp($left, $width, $windowWidth), 'y' => self::clamp($top, $height, $windowHeight), 'side' => $side];
        }
        $after = $x + $anchorWidth + $offset;
        $before = $x - $offset - $width;
        $fitsAfter = $after + $width <= $windowWidth - self::MARGIN;
        $fitsBefore = $before >= self::MARGIN;
        if ($side === 'right' && !$fitsAfter && $fitsBefore) {
            $side = 'left';
        } elseif ($side === 'left' && !$fitsBefore && $fitsAfter) {
            $side = 'right';
        }
        $left = $side === 'right' ? $after : $before;
        $top = $this->aligned($y, $anchorHeight, $height);

        return ['x' => self::clamp($left, $width, $windowWidth), 'y' => self::clamp($top, $height, $windowHeight), 'side' => $side];
    }

    private function aligned(float $start, float $anchor, float $size): float
    {
        return match ($this->align) {
            'start' => $start,
            'center' => $start + ($anchor - $size) / 2,
            'end' => $start + $anchor - $size,
        };
    }

    private static function clamp(float $position, float $size, float $window): float
    {
        return max(self::MARGIN, min($position, $window - $size - self::MARGIN));
    }
}
