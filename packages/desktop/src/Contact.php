<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Pam\Desktop\Plugin\MovedToPlugin;

/*
 * Deprecated since 1.46.0: Contact moved to the pushinbr/pam-desktop-contacts plugin as
 * \Pam\Desktop\Contacts\Contact. This shim aliases the old name while the plugin is
 * installed and is removed in 1.47.0.
 */
MovedToPlugin::alias('Pam\\Desktop\\Contact', 'Pam\\Desktop\\Contacts\\Contact', 'pushinbr/pam-desktop-contacts');
