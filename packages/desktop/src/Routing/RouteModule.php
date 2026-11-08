<?php

declare(strict_types=1);

namespace Pam\Desktop\Routing;

interface RouteModule
{
    public function register(): void;
}
