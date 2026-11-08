<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts;

/**
 * How several bar series share one category slot.
 */
enum BarLayout: int
{
    /** One bar per series, side by side. */
    case Grouped = 1;

    /** One bar per category, series stacked from the baseline. */
    case Stacked = 2;
}
