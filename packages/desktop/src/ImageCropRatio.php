<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Pam\Desktop\Plugin\MovedToPlugin;

/*
 * Deprecated since 1.46.0: ImageCropRatio moved to the pushinbr/pam-desktop-media-editor plugin as
 * \Pam\Desktop\MediaEditor\ImageCropRatio. This shim aliases the old name while the plugin is
 * installed and is removed in 1.47.0.
 */
MovedToPlugin::alias('Pam\\Desktop\\ImageCropRatio', 'Pam\\Desktop\\MediaEditor\\ImageCropRatio', 'pushinbr/pam-desktop-media-editor');
