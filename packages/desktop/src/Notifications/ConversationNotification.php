<?php
declare(strict_types=1);
namespace Pam\Desktop\Notifications;
use Pam\Desktop\Plugin\MovedToPlugin;
MovedToPlugin::alias(__NAMESPACE__.'\\ConversationNotification', \Pam\Desktop\Notifications\Contracts\ConversationNotification::class, 'pushinbr/pam-desktop-notifications');
