<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum ImageFit: int
{
    case Cover = 1;
    case Contain = 2;
    case Fill = 3;
    case Center = 4;
    case Repeat = 5;
}
