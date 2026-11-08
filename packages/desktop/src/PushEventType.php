<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Pam\Desktop\Plugin\MovedToPlugin;

// Deprecated in 1.46; removed in 1.47.
MovedToPlugin::alias('Pam\\Desktop\\PushEventType', 'Pam\\Desktop\\Notifications\\PushEventType', 'pushinbr/pam-desktop-notifications');
