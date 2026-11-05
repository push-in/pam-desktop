<?php

declare(strict_types=1);

namespace Pam\Desktop;

final class ShellEffect
{
    public static function quit(): Effect
    {
        return new Effect(EffectKind::QuitApplication);
    }

    public static function menuEnabled(string $id, bool $enabled = true): Effect
    {
        Identifier::assert($id, 'The menu item identifier');

        return new Effect(
            EffectKind::SetMenuItemEnabled,
            payload: ['id' => $id, 'enabled' => $enabled],
        );
    }

    public static function menuChecked(string $id, bool $checked = true): Effect
    {
        Identifier::assert($id, 'The menu item identifier');

        return new Effect(
            EffectKind::SetMenuItemChecked,
            payload: ['id' => $id, 'checked' => $checked],
        );
    }

    public static function trayVisible(bool $visible = true): Effect
    {
        return new Effect(
            EffectKind::SetTrayVisible,
            payload: ['visible' => $visible],
        );
    }

    public static function badge(?int $count): Effect
    {
        if ($count !== null && ($count < 0 || $count > 9_999)) {
            throw new \InvalidArgumentException('The application badge count must be between 0 and 9,999.');
        }

        return new Effect(
            EffectKind::SetApplicationBadge,
            payload: ['visible' => $count !== null, 'count' => $count ?? 0],
        );
    }

    public static function taskbarProgress(
        float $progress,
        TaskbarProgressState $state = TaskbarProgressState::Normal,
    ): Effect {
        if (!is_finite($progress) || $progress < 0.0 || $progress > 1.0) {
            throw new \InvalidArgumentException('Taskbar progress must be between 0.0 and 1.0.');
        }

        return new Effect(
            EffectKind::SetTaskbarProgress,
            payload: ['progress' => $progress, 'state' => $state->value],
        );
    }
}
