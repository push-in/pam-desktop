<?php
declare(strict_types=1);
namespace Pam\Desktop\Notifications;
use Pam\Desktop\Plugin\MovedToPlugin;
MovedToPlugin::alias(__NAMESPACE__.'\\NotificationActionType', \Pam\Desktop\Notifications\Contracts\NotificationActionType::class, 'pushinbr/pam-desktop-notifications');
