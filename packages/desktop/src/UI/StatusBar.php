<?php

declare(strict_types=1);

namespace Pam\Desktop\UI;

use Pam\Desktop\Element;
use Pam\Desktop\NodeKind;
use Pam\Desktop\PropKey;
use Pam\Desktop\StatusBarAppearance;

final class StatusBar extends Element
{
    public static function make(
        ?int $color = null,
        StatusBarAppearance $appearance = StatusBarAppearance::Dark,
        bool $hidden = false,
    ): self {
        $bar = (new self(NodeKind::StatusBar))
            ->withProperty(PropKey::StatusBarStyle, $appearance->value)
            ->withProperty(PropKey::StatusBarHidden, $hidden);

        return $color === null ? $bar : $bar->withProperty(PropKey::StatusBarColor, $color);
    }

    public function animated(bool $animated = true): self
    {
        return $this->withProperty(PropKey::StatusBarAnimated, $animated);
    }

    public function translucent(bool $translucent = true): self
    {
        return $this->withProperty(PropKey::StatusBarTranslucent, $translucent);
    }

    public function navigationBarHidden(bool $hidden = true): self
    {
        return $this->withProperty(PropKey::NavigationBarHidden, $hidden);
    }
}
