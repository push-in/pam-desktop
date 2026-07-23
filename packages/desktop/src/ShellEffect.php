<?php

declare(strict_types=1);

namespace Pam\Desktop;

final class ShellEffect
{
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
}
