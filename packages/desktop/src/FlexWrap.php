<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum FlexWrap: int
{
    case NoWrap = 1;
    case Wrap = 2;
    case WrapReverse = 3;
}
