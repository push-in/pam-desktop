<?php

declare(strict_types=1);

namespace Pam\Desktop\System;

/** The edge of the window a card toast appears at. */
enum ToastPosition: int
{
    case Top = 1;
    case Bottom = 2;
}
