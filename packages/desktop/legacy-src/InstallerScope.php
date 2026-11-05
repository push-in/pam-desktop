<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum InstallerScope: int
{
    case CurrentUser = 1;
    case Machine = 2;
}
