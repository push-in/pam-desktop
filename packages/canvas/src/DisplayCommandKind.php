<?php

declare(strict_types=1);

namespace Pam\Desktop\Canvas;

/** Stable sequential scene vocabulary; mapped to the core desktop drawing protocol. */
enum DisplayCommandKind: int
{
    case FillRect = 1;
    case RoundRect = 2;
    case Circle = 3;
    case Line = 4;
    case Text = 5;
    case Arc = 6;
    case Sector = 7;
    case Polyline = 8;
    case Polygon = 9;
    case Path = 10;
    case GradientPath = 11;
    case Label = 12;
    case DashedLine = 13;
}
