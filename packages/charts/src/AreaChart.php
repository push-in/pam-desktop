<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts;

/**
 * A `LineChart` whose area fill is enabled by default.
 */
final class AreaChart extends LineChart
{
    public static function make(float|int $width, float|int $height): static
    {
        return parent::make($width, $height)->area(true);
    }
}
