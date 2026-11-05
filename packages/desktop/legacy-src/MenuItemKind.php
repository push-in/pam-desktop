<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum MenuItemKind: int
{
    case Command = 1;
    case Checkbox = 2;
    case Separator = 3;
    case Submenu = 4;
}
