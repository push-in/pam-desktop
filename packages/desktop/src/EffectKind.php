<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum EffectKind: int
{
    case SetWindowTitle = 1;
    case SetWindowVisible = 2;
    case CloseWindow = 3;
    case FocusWindow = 4;
}
