<?php

declare(strict_types=1);

namespace App\Navigation;

use App\Screens\Home\HomeScreen;
use Pam\Desktop\Navigation\Navigator;
use Pam\Desktop\Routing\Route;

final class AppRouter
{
    public static function make(): Navigator
    {
        return Route::stack(
            name: 'main',
            initial: AppRoute::Home,
            routes: static function (): void {
                Route::screen(AppRoute::Home, HomeScreen::class);
            },
            restoreState: false,
        );
    }
}
