<?php

declare(strict_types=1);

namespace Pam\Desktop\Routing;

use Closure;
use Pam\Desktop\Navigation\ScreenOptions;
use Pam\Desktop\Navigation\ScreenOptionsPatch;

final readonly class RoutePreset
{
    public function __construct(
        public ScreenOptions|ScreenOptionsPatch|Closure $options,
    ) {
    }
}
