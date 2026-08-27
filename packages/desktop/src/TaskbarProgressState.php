<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum TaskbarProgressState: int
{
    case Hidden = 1;
    case Indeterminate = 2;
    case Normal = 3;
    case Paused = 4;
    case Error = 5;
}
