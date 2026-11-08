<?php

declare(strict_types=1);

namespace Pam\Desktop\UI;

use Closure;
use Pam\Desktop\Element;
use Pam\Desktop\EventKind;
use Pam\Desktop\NodeKind;
use Pam\Desktop\PropKey;

final class Button extends Element
{
    public static function make(string $label): self
    {
        return (new self(NodeKind::Button))->withProperty(PropKey::Text, $label);
    }

    public function onPress(Closure $handler): self
    {
        return $this->withEvent(EventKind::Press, $handler);
    }

    public function loading(bool $loading = true): self
    {
        return $this->withProperty(PropKey::Loading, $loading);
    }
}
