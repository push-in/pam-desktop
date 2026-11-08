<?php

declare(strict_types=1);

namespace Pam\Desktop\Sync\Durable;

enum OutboxDisposition: int
{
    case Applied = 1;
    case Retry = 2;
    case Rejected = 3;
}
