<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum PositionType: int
{
    case Relative = 1;
    case Absolute = 2;
    case Fixed = 3;
    /** CSS `position: sticky`: in flow, held at its insets inside the nearest scroll view. */
    case Sticky = 4;
}
