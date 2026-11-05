<?php

declare(strict_types=1);

namespace Pam\Desktop\Navigation;

use Pam\Desktop\Plugin\MovedToPlugin;

MovedToPlugin::alias(NavigationDrawerItem::class, \Pam\Desktop\Drawer\NavigationDrawerItem::class, 'pushinbr/pam-desktop-drawer');
