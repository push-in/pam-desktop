<?php

declare(strict_types=1);

namespace Pam\Desktop\Canvas;

/**
 * How the ends of stroked lines and arcs are drawn.
 */
enum LineCap: int
{
    case Butt = 1;
    case Round = 2;
}
