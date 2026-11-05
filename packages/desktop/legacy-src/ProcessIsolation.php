<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum ProcessIsolation: int
{
    case Shared = 1;
    case PerWindow = 2;
    case PerWorkspace = 3;
}
