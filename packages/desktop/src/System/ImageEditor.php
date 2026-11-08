<?php

declare(strict_types=1);

namespace Pam\Desktop\System;

use Pam\Desktop\Plugin\MovedToPlugin;

/*
 * Deprecated since 1.46.0: ImageEditor moved to the pushinbr/pam-desktop-media-editor plugin as
 * \Pam\Desktop\MediaEditor\ImageEditor. This shim aliases the old name while the plugin is
 * installed and is removed in 1.47.0.
 */
MovedToPlugin::alias('Pam\\Desktop\\System\\ImageEditor', 'Pam\\Desktop\\MediaEditor\\ImageEditor', 'pushinbr/pam-desktop-media-editor');
