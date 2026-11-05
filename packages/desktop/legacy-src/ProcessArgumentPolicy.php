<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum ProcessArgumentPolicy: int
{
    case Fixed = 1;
    case Append = 2;
}
