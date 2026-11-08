<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Pam\Desktop\Plugin\MovedToPlugin;

/*
 * Deprecated since 1.46.0: SensorReading moved to the pushinbr/pam-desktop-sensors plugin as
 * \Pam\Desktop\Sensors\SensorReading. This shim aliases the old name while the plugin is
 * installed and is removed in 1.47.0.
 */
MovedToPlugin::alias('Pam\\Desktop\\SensorReading', 'Pam\\Desktop\\Sensors\\SensorReading', 'pushinbr/pam-desktop-sensors');
