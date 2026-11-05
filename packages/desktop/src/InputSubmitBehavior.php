<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum InputSubmitBehavior: int
{
    case Submit = 1;
    case BlurAndSubmit = 2;
    case Newline = 3;
}
