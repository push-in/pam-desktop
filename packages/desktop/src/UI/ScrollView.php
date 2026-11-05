<?php

declare(strict_types=1);

namespace Pam\Desktop\UI;

use Pam\Desktop\Renderable;

final class ScrollView
{
    private function __construct()
    {
    }

    public static function make(Renderable $child): Scroll
    {
        return Scroll::make($child);
    }
}
