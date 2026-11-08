<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Pam\Desktop\Plugin\MovedToPlugin;

/*
 * Deprecated since 1.46.0: LocationServicesResult moved to the pushinbr/pam-desktop-location plugin as
 * \Pam\Desktop\Location\LocationServicesResult. This shim aliases the old name while the plugin is
 * installed and is removed in 1.47.0.
 */
MovedToPlugin::alias('Pam\\Desktop\\LocationServicesResult', 'Pam\\Desktop\\Location\\LocationServicesResult', 'pushinbr/pam-desktop-location');
