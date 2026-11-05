<?php

declare(strict_types=1);

namespace Pam\Desktop\LocalFirst;

enum ConflictPolicy: int
{
    case ClientWins = 1;
    case ServerWins = 2;
    case LatestWriteWins = 3;
    case Merge = 4;
}
