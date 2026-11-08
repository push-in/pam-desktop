<?php

declare(strict_types=1);

use App\Navigation\AppRouter;
use Pam\Desktop\App;

require __DIR__.'/vendor/autoload.php';

App::components(__DIR__.'/src', __DIR__.'/.pam-desktop/components');
App::run(AppRouter::make());
