<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum TextEllipsizeMode: int
{
    case Tail = 1;
    case Head = 2;
    case Middle = 3;
    case Clip = 4;
    /** Single-line marquee ticker. Not implemented on desktop: renders as Tail. */
    case Marquee = 5;
}
