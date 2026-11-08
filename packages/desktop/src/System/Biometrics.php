<?php
declare(strict_types=1);
namespace Pam\Desktop\System;
use Pam\Desktop\Plugin\MovedToPlugin;
MovedToPlugin::alias(__NAMESPACE__.'\\Biometrics', \Pam\Desktop\Auth\LegacyBiometrics::class, 'pushinbr/pam-desktop-auth');
