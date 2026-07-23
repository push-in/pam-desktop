<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum ErrorCode: int
{
    case InvalidMessage = 1;
    case UnsupportedProtocol = 2;
    case UnknownCommand = 3;
    case InvalidPayload = 4;
    case HandlerFailed = 5;
    case WorkerUnavailable = 6;
    case Unauthorized = 7;
    case Internal = 8;
}

