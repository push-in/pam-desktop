<?php

declare(strict_types=1);

namespace Pam\Desktop\Sync\Durable;

enum OutboxState: int
{
    case Pending = 1;
    case InFlight = 2;
    case Applied = 3;
    case Failed = 4;
}
