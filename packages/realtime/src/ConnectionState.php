<?php

declare(strict_types=1);

namespace Pam\Desktop\Realtime;

enum ConnectionState: int
{
    case Connecting = 1;
    case Authorizing = 2;
    case Subscribed = 3;
    case Reconnecting = 4;
    case Suspended = 5;
    case Rejected = 6;
    case Closed = 7;
}
