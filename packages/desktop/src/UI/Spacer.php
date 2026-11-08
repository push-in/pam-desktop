<?php

declare(strict_types=1);

namespace Pam\Desktop\UI;

use Pam\Desktop\Element;
use Pam\Desktop\NodeKind;
use Pam\Desktop\Style;

final class Spacer extends Element
{
    public static function make(float $size = 8.0): self
    {
        return (new self(NodeKind::Spacer))->style(new Style(height: $size));
    }
}

