<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum NetworkType: int
{
    case None = 1;
    case Wifi = 2;
    case Cellular = 3;
    case Ethernet = 4;
    case Other = 5;
}
