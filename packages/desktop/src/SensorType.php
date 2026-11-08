<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Pam\Desktop\Plugin\MovedToPlugin;

/*
 * Deprecated since 1.46.0: SensorType moved to the pushinbr/pam-desktop-sensors plugin as
 * \Pam\Desktop\Sensors\SensorType. This shim aliases the old name while the plugin is
 * installed and is removed in 1.47.0.
 */
MovedToPlugin::alias('Pam\\Desktop\\SensorType', 'Pam\\Desktop\\Sensors\\SensorType', 'pushinbr/pam-desktop-sensors');
