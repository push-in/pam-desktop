<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Pam\Desktop\Plugin\MovedToPlugin;

/*
 * Deprecated since 1.46.0: LocationPosition moved to the pushinbr/pam-desktop-location plugin as
 * \Pam\Desktop\Location\LocationPosition. This shim aliases the old name while the plugin is
 * installed and is removed in 1.47.0.
 */
MovedToPlugin::alias('Pam\\Desktop\\LocationPosition', 'Pam\\Desktop\\Location\\LocationPosition', 'pushinbr/pam-desktop-location');
