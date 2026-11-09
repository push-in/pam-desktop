<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Tests\ProjectCheck\Broken\Greeting;

App::components(__DIR__.'/src', __DIR__.'/.pam-desktop/components');
App::run(new Greeting());
