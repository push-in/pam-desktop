<?php

declare(strict_types=1);

namespace Pam\Desktop\Style;

enum StyleScope: int
{
    case Scoped = 1;
    case Module = 2;
    case Global = 3;
}
