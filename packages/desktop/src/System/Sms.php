<?php

declare(strict_types=1);

namespace Pam\Desktop\System;

use Pam\Desktop\Plugin\MovedToPlugin;

/*
 * Deprecated since 1.46.0: Sms moved to the pushinbr/pam-desktop-contacts plugin as
 * \Pam\Desktop\Contacts\Sms. This shim aliases the old name while the plugin is
 * installed and is removed in 1.47.0.
 */
MovedToPlugin::alias('Pam\\Desktop\\System\\Sms', 'Pam\\Desktop\\Contacts\\Sms', 'pushinbr/pam-desktop-contacts');
