<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum TextBreakStrategy: int
{
    case HighQuality = 1;
    case Simple = 2;
    case Balanced = 3;
}
