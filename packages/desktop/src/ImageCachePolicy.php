<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum ImageCachePolicy: int
{
    case Default = 1;
    case Reload = 2;
    case ForceCache = 3;
    case OnlyIfCached = 4;
    case None = 5;
}
