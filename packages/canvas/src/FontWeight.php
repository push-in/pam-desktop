<?php

declare(strict_types=1);

namespace Pam\Desktop\Canvas;

/**
 * Weight of the platform system font used by labels.
 */
enum FontWeight: int
{
    case Regular = 1;
    case Medium = 2;
    case SemiBold = 3;
    case Bold = 4;
}
