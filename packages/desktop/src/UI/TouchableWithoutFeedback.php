<?php

declare(strict_types=1);

namespace Pam\Desktop\UI;

use Pam\Desktop\Renderable;

final class TouchableWithoutFeedback
{
    private function __construct()
    {
    }

    public static function make(Renderable ...$children): Pressable
    {
        return Pressable::make(...$children)->pressedOpacity(1.0);
    }
}
