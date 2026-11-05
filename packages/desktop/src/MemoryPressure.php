<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum MemoryPressure: int
{
    case Moderate = 1;
    case Critical = 2;
}
