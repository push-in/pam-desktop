<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Pam\Desktop\Plugin\MovedToPlugin;

/*
 * Deprecated since 1.46.0: AudioRecordingProgress moved to the pushinbr/pam-desktop-audio plugin as
 * \Pam\Desktop\Audio\AudioRecordingProgress. This shim aliases the old name while the plugin is
 * installed and is removed in 1.47.0.
 */
MovedToPlugin::alias('Pam\\Desktop\\AudioRecordingProgress', 'Pam\\Desktop\\Audio\\AudioRecordingProgress', 'pushinbr/pam-desktop-audio');
