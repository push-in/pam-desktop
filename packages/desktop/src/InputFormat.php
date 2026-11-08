<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum InputFormat: int
{
    case None = 1;
    case Pattern = 2;
    case Currency = 3;
    case Decimal = 4;
}
