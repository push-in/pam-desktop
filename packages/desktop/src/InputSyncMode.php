<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum InputSyncMode: int
{
    case Native = 1;
    case Debounced = 2;
    case Immediate = 3;
    case OnBlur = 4;
    case OnSubmit = 5;
}
