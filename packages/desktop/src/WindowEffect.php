<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final class WindowEffect
{
    public static function title(string $title): Effect
    {
        if (trim($title) === '') {
            throw new InvalidArgumentException('The window title cannot be empty.');
        }

        return new Effect(EffectKind::SetWindowTitle, ['title' => $title]);
    }

    public static function visible(bool $visible): Effect
    {
        return new Effect(EffectKind::SetWindowVisible, ['visible' => $visible]);
    }

    public static function close(): Effect
    {
        return new Effect(EffectKind::CloseWindow);
    }
}

