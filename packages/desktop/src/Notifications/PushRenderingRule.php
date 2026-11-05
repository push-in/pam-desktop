<?php
declare(strict_types=1);
namespace Pam\Desktop\Notifications;
use Pam\Desktop\Plugin\MovedToPlugin;
MovedToPlugin::alias(__NAMESPACE__.'\\PushRenderingRule', \Pam\Desktop\Notifications\Contracts\PushRenderingRule::class, 'pushinbr/pam-desktop-notifications');
