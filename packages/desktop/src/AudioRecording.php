<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Pam\Desktop\Plugin\MovedToPlugin;

/*
 * Deprecated since 1.46.0: AudioRecording moved to the pushinbr/pam-desktop-audio plugin as
 * \Pam\Desktop\Audio\AudioRecording. This shim aliases the old name while the plugin is
 * installed and is removed in 1.47.0.
 */
MovedToPlugin::alias('Pam\\Desktop\\AudioRecording', 'Pam\\Desktop\\Audio\\AudioRecording', 'pushinbr/pam-desktop-audio');
