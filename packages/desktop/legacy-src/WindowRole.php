<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum WindowRole: int
{
    case Primary = 1;
    case Child = 2;
    case Modal = 3;
    case Popover = 4;
    case Panel = 5;
    case Palette = 6;
}
