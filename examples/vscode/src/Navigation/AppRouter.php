<?php

declare(strict_types=1);

namespace App\Navigation;

use App\Screens\Workbench\WorkbenchScreen;
use Pam\Desktop\Navigation\Navigator;
use Pam\Desktop\Routing\Route;

final class AppRouter
{
    public static function make(): Navigator
    {
        return Route::stack(
            name: 'main',
            initial: AppRoute::Workbench,
            routes: static function (): void {
                Route::screen(AppRoute::Workbench, WorkbenchScreen::class);
            },
            restoreState: false,
        );
    }
}
