<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum PermissionKind: int
{
    case Camera = 1;
    case Microphone = 2;
    case Photos = 3;
    case Notifications = 4;
    case LocationWhenInUse = 5;
    case Contacts = 6;
    /** Bluetooth access (headsets during calls). */
    case BluetoothConnect = 7;
    /** Full-screen call/alarm notifications. */
    case FullScreenIntent = 8;
    /** Cellular call state. */
    case PhoneState = 9;
}
