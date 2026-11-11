<?php

declare(strict_types=1);

namespace App\Navigation;

use App\Screens\Client\ClientScreen;
use Pam\Desktop\Navigation\Navigator;
use Pam\Desktop\Routing\Route;

final class AppRouter
{
    public static function make(): Navigator
    {
        return Route::stack(
            name: 'main',
            initial: AppRoute::Client,
            routes: static function (): void {
                Route::screen(AppRoute::Client, ClientScreen::class);
            },
            restoreState: false,
        );
    }
}
