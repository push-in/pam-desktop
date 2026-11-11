<?php

declare(strict_types=1);

use App\Navigation\AppRouter;
use Pam\Desktop\App;
use Pam\Desktop\Markdown\Markdown;

require __DIR__.'/vendor/autoload.php';

Markdown::register();
App::components(__DIR__.'/src', __DIR__.'/.pam-desktop/components');
App::run(AppRouter::make());
