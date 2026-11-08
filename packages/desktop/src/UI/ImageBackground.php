<?php

declare(strict_types=1);

namespace Pam\Desktop\UI;

use Pam\Desktop\Element;
use Pam\Desktop\NodeKind;
use Pam\Desktop\PropKey;
use Pam\Desktop\Renderable;
use Pam\Desktop\UI\Concerns\HasImageBehavior;

final class ImageBackground extends Element
{
    use HasImageBehavior;

    public static function make(string $source, Renderable|false|null ...$children): self
    {
        return (new self(NodeKind::ImageBackground))
            ->withChildren($children)
            ->withProperty(PropKey::Source, $source);
    }
}
