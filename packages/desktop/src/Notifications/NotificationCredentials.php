<?php
declare(strict_types=1);
namespace Pam\Desktop\Notifications;
use Pam\Desktop\Plugin\MovedToPlugin;
MovedToPlugin::alias(__NAMESPACE__.'\\NotificationCredentials', \Pam\Desktop\Notifications\Contracts\NotificationCredentials::class, 'pushinbr/pam-desktop-notifications');
