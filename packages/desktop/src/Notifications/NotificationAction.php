<?php
declare(strict_types=1);
namespace Pam\Desktop\Notifications;
use Pam\Desktop\Plugin\MovedToPlugin;
MovedToPlugin::alias(__NAMESPACE__.'\\NotificationAction', \Pam\Desktop\Notifications\Contracts\NotificationAction::class, 'pushinbr/pam-desktop-notifications');
