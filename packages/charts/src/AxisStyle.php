<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts;

use InvalidArgumentException;

/**
 * Metrics of cartesian axes, carried by {@see ChartTheme::axisStyle()}.
 *
 * - `fontSize`: tick and category labels (Chart.js `ticks.font.size`, 12).
 * - `labelGap`: space between the value labels and the plot (Chart.js
 *   `ticks.padding` + `grid.tickLength`).
 * - `xLabelHeight`: band under the plot that holds the category labels.
 * - `gridDash`: dash and gap length of the grid lines; 0 draws them solid,
 *   as Chart.js does by default.
 * - `padding*`: space between the canvas edge and the plot or its labels
 *   (Chart.js `layout.padding`; its plot reaches the right edge).
 *
 * - `xLabelWeight`: weight of the category labels (Chart.js
 *   `scales.x.ticks.font.weight`); value labels stay regular.
 * - `fitEdgeLabels`: Chart.js layout: the plot starts after the value
 *   labels measured with the chart font and ends half the last category
 *   label (plus `tickPadding`) before the right edge, so the edge labels
 *   are centred on their points instead of being pushed inside.
 * - `tickLength`: grid lines reach this far left of the plot (Chart.js
 *   `grid.tickLength` with `drawTicks`).
 * - `tickPadding`: space between a label and its tick (Chart.js
 *   `ticks.padding`), used by `fitEdgeLabels`.
 * - `borderColor`: axis lines along the left and bottom of the plot
 *   (Chart.js `border.color`); null draws none.
 *
 * The defaults keep the previous look.
 */
final readonly class AxisStyle
{
    public function __construct(
        public float $fontSize = 10.0,
        public float $labelGap = 6.0,
        public float $xLabelHeight = 18.0,
        public float $gridDash = 3.0,
        public float $paddingTop = 8.0,
        public float $paddingRight = 8.0,
        public float $paddingBottom = 8.0,
        public float $paddingLeft = 8.0,
        public \Pam\Desktop\Canvas\FontWeight $xLabelWeight = \Pam\Desktop\Canvas\FontWeight::Regular,
        public bool $fitEdgeLabels = false,
        public float $tickLength = 0.0,
        public float $tickPadding = 3.0,
        public ?string $borderColor = null,
    ) {
        foreach ([$fontSize, $labelGap, $xLabelHeight, $gridDash, $paddingTop, $paddingRight, $paddingBottom, $paddingLeft, $tickLength, $tickPadding] as $value) {
            if (!is_finite($value) || $value < 0.0 || $value > 64.0) {
                throw new InvalidArgumentException('Axis metrics must be between 0 and 64.');
            }
        }
        if ($fontSize < 6.0) {
            throw new InvalidArgumentException('Axis font size must be at least 6.');
        }
    }
}
