<?php
declare(strict_types=1);
namespace Pam\Desktop\Notifications;
use Pam\Desktop\Plugin\MovedToPlugin;
MovedToPlugin::alias(__NAMESPACE__.'\\ActionEndpoint', \Pam\Desktop\Notifications\Contracts\ActionEndpoint::class, 'pushinbr/pam-desktop-notifications');
