<?php

declare(strict_types=1);

namespace Pam\Desktop\Routing;

use Pam\Desktop\Plugin\MovedToPlugin;

MovedToPlugin::alias(DrawerRegistrar::class, \Pam\Desktop\Drawer\DrawerRegistrar::class, 'pushinbr/pam-desktop-drawer');
