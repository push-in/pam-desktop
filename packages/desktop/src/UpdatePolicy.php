<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum UpdatePolicy: int
{
    case Manual = 1;
    case Notify = 2;
    case Automatic = 3;
}
