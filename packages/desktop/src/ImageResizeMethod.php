<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum ImageResizeMethod: int
{
    case Auto = 1;
    case Resize = 2;
    case Scale = 3;
    case None = 4;
}
