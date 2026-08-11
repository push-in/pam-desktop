<?php

declare(strict_types=1);

namespace Pam\Desktop\Attributes;

use Attribute;

#[Attribute(Attribute::TARGET_METHOD)]
final readonly class MenuItem
{
    public function __construct(
        public string $label,
        public ?string $id = null,
        public ?string $shortcut = null,
        public bool $checkbox = false,
        public bool $checked = false,
    ) {
    }
}
