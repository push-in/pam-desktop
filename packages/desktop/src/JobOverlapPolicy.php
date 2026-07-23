<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum JobOverlapPolicy: int
{
    case Skip = 1;
    case Wait = 2;
}
