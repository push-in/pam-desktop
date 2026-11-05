<?php

declare(strict_types=1);

namespace Pam\Desktop\Navigation;

use Closure;

interface NavigationObservable
{
    public function addListener(NavigationEventType $type, Closure $listener): NavigationSubscription;
}
