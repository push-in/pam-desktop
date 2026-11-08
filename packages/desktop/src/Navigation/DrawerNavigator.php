<?php

declare(strict_types=1);

namespace Pam\Desktop\Navigation;

use Pam\Desktop\Plugin\MovedToPlugin;

MovedToPlugin::alias(DrawerNavigator::class, \Pam\Desktop\Drawer\DrawerNavigator::class, 'pushinbr/pam-desktop-drawer');
