<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum TrayCloseBehavior: int
{
    case Exit = 1;
    case Hide = 2;
}
