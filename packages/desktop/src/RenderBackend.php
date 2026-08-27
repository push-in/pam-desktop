<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum RenderBackend: int
{
    case Automatic = 1;
    case Gpu = 2;
    case Software = 3;
}
