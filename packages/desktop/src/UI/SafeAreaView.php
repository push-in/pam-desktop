<?php

declare(strict_types=1);

namespace Pam\Desktop\UI;

use Pam\Desktop\Element;
use Pam\Desktop\NodeKind;
use Pam\Desktop\PropKey;
use Pam\Desktop\Renderable;
use Pam\Desktop\SafeAreaMode;

final class SafeAreaView extends Element
{
    public static function make(Renderable|false|null ...$children): self
    {
        return (new self(NodeKind::SafeAreaView))->withChildren($children);
    }

    public function edges(
        bool $top = true,
        bool $right = true,
        bool $bottom = true,
        bool $left = true,
    ): self {
        return $this
            ->withProperty(PropKey::SafeAreaTop, $top)
            ->withProperty(PropKey::SafeAreaRight, $right)
            ->withProperty(PropKey::SafeAreaBottomEdge, $bottom)
            ->withProperty(PropKey::SafeAreaLeft, $left);
    }

    public function mode(SafeAreaMode $mode): self
    {
        return $this->withProperty(PropKey::SafeAreaMode, $mode->value);
    }
}
