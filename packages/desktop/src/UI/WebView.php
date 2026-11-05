<?php
declare(strict_types=1);
namespace Pam\Desktop\UI;
use Pam\Desktop\Plugin\MovedToPlugin;
MovedToPlugin::alias(__NAMESPACE__.'\\WebView', \Pam\Desktop\WebView\WebView::class, 'pushinbr/pam-desktop-webview');
