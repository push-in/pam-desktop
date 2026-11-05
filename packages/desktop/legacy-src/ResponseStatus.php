<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum ResponseStatus: int
{
    case Success = 1;
    case Failure = 2;
}

