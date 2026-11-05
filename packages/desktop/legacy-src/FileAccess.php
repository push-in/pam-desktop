<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum FileAccess: int
{
    case Read = 1;
    case Write = 2;
    case ReadWrite = 3;
}
