<?php

declare(strict_types=1);

namespace Pam\DesktopSsh;

/** The `kind` of an event read with the module's `next`. */
enum SshEventKind: int
{
    case ShellData = 1;
    case ShellExit = 2;
    case TransferProgress = 3;
    case TransferDone = 4;
    case TransferFailed = 5;
    case ForwardOpened = 6;
    case ForwardFailed = 7;
    case ForwardConnections = 8;
    case SessionClosed = 9;
    case SessionFailed = 10;
}
