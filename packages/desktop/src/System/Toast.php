<?php

declare(strict_types=1);

namespace Pam\Desktop\System;

use Pam\Desktop\Plugin\MovedToPlugin;

MovedToPlugin::alias(Toast::class, \Pam\Desktop\Toast\Toast::class, 'pushinbr/pam-desktop-toast');
