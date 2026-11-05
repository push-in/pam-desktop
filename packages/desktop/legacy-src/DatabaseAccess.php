<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum DatabaseAccess: int
{
    case Read = 1;
    case ReadWrite = 2;
}
