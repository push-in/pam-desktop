<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts;

use InvalidArgumentException;
use Pam\Desktop\Canvas\FontWeight;

/**
 * Metrics of chart legends, carried by {@see ChartTheme::legendStyle()}.
 *
 * - `fontSize` / `fontWeight`: label and value text.
 * - `markerRadius`: radius of the colour dot (Chart.js `pointStyle: 'circle'`
 *   draws a dot of about half the font size).
 * - `markerGap`: space between the dot and its label.
 * - `itemGap`: space between items of an inline legend (Chart.js `padding`).
 * - `valueGap`: space between a label and its percentage.
 * - `markerBox`: width the marker takes in an inline item (Chart.js
 *   `boxWidth`); the dot is centred in it and may overflow it. Null keeps
 *   the dot's diameter.
 * - `color`: label colour (Chart.js `legend.labels.color`); null uses the
 *   theme's muted colour.
 */
final readonly class LegendStyle
{
    public function __construct(
        public float $fontSize = 11.0,
        public FontWeight $fontWeight = FontWeight::Medium,
        public float $markerRadius = 4.0,
        public float $markerGap = 6.0,
        public float $itemGap = 12.0,
        public float $valueGap = 6.0,
        public ?float $markerBox = null,
        public ?string $color = null,
    ) {
        foreach ([$fontSize, $markerRadius, $markerGap, $itemGap, $valueGap, $markerBox ?? 0.0] as $value) {
            if (!is_finite($value) || $value < 0.0 || $value > 64.0) {
                throw new InvalidArgumentException('Legend metrics must be between 0 and 64.');
            }
        }
        if ($fontSize < 6.0) {
            throw new InvalidArgumentException('Legend font size must be at least 6.');
        }
    }

    /** Width of the marker in an inline item: {@see $markerBox} or the dot's diameter. */
    public function markerWidth(): float
    {
        return $this->markerBox ?? 2 * $this->markerRadius;
    }
}
