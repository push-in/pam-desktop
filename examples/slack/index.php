<?php

declare(strict_types=1);

use App\Navigation\AppRouter;
use Pam\Desktop\App;
use Pam\Desktop\Markdown\Markdown;
use Pam\DesktopUi\DesktopUI;

require __DIR__.'/vendor/autoload.php';

Markdown::register();
DesktopUI::boot(__DIR__.'/.pam-desktop/desktop-ui');
App::components(__DIR__.'/src', __DIR__.'/.pam-desktop/components');
App::run(AppRouter::make());
