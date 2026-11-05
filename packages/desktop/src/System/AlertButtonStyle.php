<?php

declare(strict_types=1);

namespace Pam\Desktop\System;

/** Native alert button roles. */
enum AlertButtonStyle: int
{
    case Default = 1;
    case Cancel = 2;
    case Destructive = 3;
}
