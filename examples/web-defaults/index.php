<?php

declare(strict_types=1);

use App\WebDefaults;
use Pam\Desktop\App;
use Pam\Desktop\Charts\ChartsPluginProvider;

require __DIR__.'/../../vendor/autoload.php';
require __DIR__.'/src/Order.php';

// CSS-like defaults: content-sized lists, font stacks, classic thin
// scrollbars, value methods in templates and Chart.js value axes.
(new ChartsPluginProvider())->register();
App::components(__DIR__.'/src', __DIR__.'/.pam-desktop/components');
App::run(App::make(WebDefaults::class));
