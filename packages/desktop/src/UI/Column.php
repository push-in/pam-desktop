<?php

declare(strict_types=1);

namespace Pam\Desktop\UI;

use Pam\Desktop\Element;
use Pam\Desktop\NodeKind;
use Pam\Desktop\Renderable;

final class Column extends Element
{
    public static function make(Renderable|false|null ...$children): self
    {
        return (new self(NodeKind::Column))->withChildren($children);
    }
}
