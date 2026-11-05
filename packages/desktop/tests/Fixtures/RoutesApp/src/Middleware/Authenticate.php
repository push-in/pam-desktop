<?php

declare(strict_types=1);

namespace RoutesApp\Middleware;

use Closure;
use Pam\Desktop\Middleware;
use Pam\Desktop\Route;

use function Pam\Desktop\redirect;

final class Authenticate implements Middleware
{
    public static bool $signedIn = false;

    public function handle(Route $to, Closure $next): mixed
    {
        return self::$signedIn ? $next($to) : redirect('login');
    }
}
