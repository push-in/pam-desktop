<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

use Pam\Desktop\App;

final class DesktopUI
{
    public static function boot(?string $cachePath = null): void
    {
        App::components(__DIR__, $cachePath);
    }
}
