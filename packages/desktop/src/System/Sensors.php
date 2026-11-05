<?php

declare(strict_types=1);

namespace Pam\Desktop\System;

use Pam\Desktop\Plugin\MovedToPlugin;

/*
 * Deprecated since 1.46.0: Sensors moved to the pushinbr/pam-desktop-sensors plugin as
 * \Pam\Desktop\Sensors\Sensors. This shim aliases the old name while the plugin is
 * installed and is removed in 1.47.0.
 */
MovedToPlugin::alias('Pam\\Desktop\\System\\Sensors', 'Pam\\Desktop\\Sensors\\Sensors', 'pushinbr/pam-desktop-sensors');
