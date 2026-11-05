<?php

declare(strict_types=1);

namespace Pam\Desktop\System;

use Pam\Desktop\Plugin\MovedToPlugin;

/*
 * Deprecated since 1.46.0: Contacts moved to the pushinbr/pam-desktop-contacts plugin as
 * \Pam\Desktop\Contacts\Contacts. This shim aliases the old name while the plugin is
 * installed and is removed in 1.47.0.
 */
MovedToPlugin::alias('Pam\\Desktop\\System\\Contacts', 'Pam\\Desktop\\Contacts\\Contacts', 'pushinbr/pam-desktop-contacts');
