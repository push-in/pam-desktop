<?php

declare(strict_types=1);

namespace Pam\Desktop\System;

use Pam\Desktop\Plugin\MovedToPlugin;

/*
 * Deprecated since 1.46.0: Location moved to the pushinbr/pam-desktop-location plugin as
 * \Pam\Desktop\Location\Location. This shim aliases the old name while the plugin is
 * installed and is removed in 1.47.0.
 */
MovedToPlugin::alias('Pam\\Desktop\\System\\Location', 'Pam\\Desktop\\Location\\Location', 'pushinbr/pam-desktop-location');
