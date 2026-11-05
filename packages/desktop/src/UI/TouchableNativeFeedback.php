<?php

declare(strict_types=1);

namespace Pam\Desktop\UI;

use Pam\Desktop\Renderable;

final class TouchableNativeFeedback
{
    private function __construct()
    {
    }

    public static function make(
        int $rippleColor,
        Renderable ...$children,
    ): Pressable {
        return Pressable::make(...$children)->ripple($rippleColor);
    }
}
