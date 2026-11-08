<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum KeyboardAvoidingBehavior: int
{
    case Resize = 1;
    case Pan = 2;
    case Padding = 3;
    case Interactive = 4;
}
