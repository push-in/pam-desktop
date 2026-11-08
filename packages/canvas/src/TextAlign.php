<?php

declare(strict_types=1);

namespace Pam\Desktop\Canvas;

/**
 * Horizontal anchor of a label relative to its x coordinate.
 */
enum TextAlign: int
{
    case Left = 1;
    case Center = 2;
    case Right = 3;
}
