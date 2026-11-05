<?php

declare(strict_types=1);

namespace Pam\Desktop\System;

use Pam\Desktop\Plugin\MovedToPlugin;

/*
 * Deprecated since 1.46.0: AudioRecorder moved to the pushinbr/pam-desktop-audio plugin as
 * \Pam\Desktop\Audio\AudioRecorder. This shim aliases the old name while the plugin is
 * installed and is removed in 1.47.0.
 */
MovedToPlugin::alias('Pam\\Desktop\\System\\AudioRecorder', 'Pam\\Desktop\\Audio\\AudioRecorder', 'pushinbr/pam-desktop-audio');
