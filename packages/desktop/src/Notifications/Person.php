<?php
declare(strict_types=1);
namespace Pam\Desktop\Notifications;
use Pam\Desktop\Plugin\MovedToPlugin;
MovedToPlugin::alias(__NAMESPACE__.'\\Person', \Pam\Desktop\Notifications\Contracts\Person::class, 'pushinbr/pam-desktop-notifications');
