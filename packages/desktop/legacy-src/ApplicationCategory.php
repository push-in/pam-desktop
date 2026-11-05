<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum ApplicationCategory: int
{
    case Development = 1;
    case Productivity = 2;
    case Graphics = 3;
    case AudioVideo = 4;
    case Network = 5;
    case Utility = 6;
    case Game = 7;
    case Education = 8;
}
