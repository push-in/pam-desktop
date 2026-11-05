<?php
declare(strict_types=1);
namespace Pam\Desktop\Notifications;
use Pam\Desktop\Plugin\MovedToPlugin;
MovedToPlugin::alias(__NAMESPACE__.'\\PushRendering', \Pam\Desktop\Notifications\Contracts\PushRendering::class, 'pushinbr/pam-desktop-notifications');
