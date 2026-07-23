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
    case UnknownEvent = 9;
    case RequestTimedOut = 10;
    case RequestCancelled = 11;
    case WorkerCrashed = 12;
    case CapabilityDisabled = 13;
    case PermissionDenied = 14;
    case ResourceNotFound = 15;
    case ResourceTooLarge = 16;
    case NativeOperationFailed = 17;
    case InvalidGrant = 18;
    case UpdateDisabled = 19;
    case UpdateUnavailable = 20;
    case UpdateIntegrityFailed = 21;
    case UpdateInstallFailed = 22;
    case PluginUnavailable = 23;
    case PluginFailed = 24;
    case ShortcutUnavailable = 25;
    case BackgroundJobFailed = 26;
}
