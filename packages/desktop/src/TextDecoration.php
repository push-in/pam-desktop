<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum TextDecoration: int
{
    case None = 1;
    case Underline = 2;
    case LineThrough = 3;
    case UnderlineLineThrough = 4;
}
