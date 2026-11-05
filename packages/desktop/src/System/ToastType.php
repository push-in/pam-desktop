<?php

declare(strict_types=1);

namespace Pam\Desktop\System;

use Pam\Desktop\Plugin\MovedToPlugin;

MovedToPlugin::alias(ToastType::class, \Pam\Desktop\Toast\ToastType::class, 'pushinbr/pam-desktop-toast');
