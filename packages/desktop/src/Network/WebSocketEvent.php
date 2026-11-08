<?php

declare(strict_types=1);

namespace Pam\Desktop\Network;

enum WebSocketEvent: int
{
    case Open = 1;
    case Text = 2;
    case Closed = 3;
    case Failed = 4;
}
