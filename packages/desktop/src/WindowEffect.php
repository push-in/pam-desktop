<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final class WindowEffect
{
    public static function title(string $title, string $windowId = 'main'): Effect
    {
        if (trim($title) === '') {
            throw new InvalidArgumentException('The window title cannot be empty.');
        }

        return new Effect(
            kind: EffectKind::SetWindowTitle,
            windowId: $windowId,
            payload: ['title' => $title],
        );
    }

    public static function visible(bool $visible, string $windowId = 'main'): Effect
    {
        return new Effect(
            kind: EffectKind::SetWindowVisible,
            windowId: $windowId,
            payload: ['visible' => $visible],
        );
    }

    public static function close(string $windowId = 'main'): Effect
    {
        return new Effect(EffectKind::CloseWindow, $windowId);
    }

    public static function focus(string $windowId = 'main'): Effect
    {
        return new Effect(EffectKind::FocusWindow, $windowId);
    }

    public static function fullscreen(bool $fullscreen = true, string $windowId = 'main'): Effect
    {
        return new Effect(
            EffectKind::SetWindowFullscreen,
            $windowId,
            ['fullscreen' => $fullscreen],
        );
    }

    public static function maximized(bool $maximized = true, string $windowId = 'main'): Effect
    {
        return new Effect(
            EffectKind::SetWindowMaximized,
            $windowId,
            ['maximized' => $maximized],
        );
    }

    public static function alwaysOnTop(bool $alwaysOnTop = true, string $windowId = 'main'): Effect
    {
        return new Effect(
            EffectKind::SetWindowAlwaysOnTop,
            $windowId,
            ['alwaysOnTop' => $alwaysOnTop],
        );
    }

    public static function attention(
        bool $active = true,
        bool $critical = false,
        string $windowId = 'main',
    ): Effect {
        return new Effect(
            EffectKind::SetWindowAttention,
            $windowId,
            ['active' => $active, 'critical' => $critical],
        );
    }
}
