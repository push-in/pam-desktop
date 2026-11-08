<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

/** PrimeVue Toast positions, as sequential integer variants. */
enum ToastPosition: int
{
    case TopRight = 1;
    case TopLeft = 2;
    case TopCenter = 3;
    case BottomRight = 4;
    case BottomLeft = 5;
    case BottomCenter = 6;
    case Center = 7;

    public function top(): bool
    {
        return in_array($this, [self::TopRight, self::TopLeft, self::TopCenter], true);
    }

    public function alignment(): string
    {
        return match ($this) {
            self::TopRight, self::BottomRight => 'flex-end',
            self::TopLeft, self::BottomLeft => 'flex-start',
            default => 'center',
        };
    }

    public function vertical(): string
    {
        return match ($this) {
            self::Center => 'center',
            default => $this->top() ? 'flex-start' : 'flex-end',
        };
    }
}
