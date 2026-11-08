<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum TextEllipsizeMode: int
{
    case Tail = 1;
    case Head = 2;
    case Middle = 3;
    case Clip = 4;
    /**
     * A line that does not fit slides to its end and back, resting at each
     * end. For one line (`numberOfLines(1)`); cut like Tail under reduced
     * motion.
     */
    case Marquee = 5;
}
