<?php

declare(strict_types=1);

namespace Pam\Desktop\Attributes;

use Attribute;
use Pam\Desktop\TrayCloseBehavior;

#[Attribute(Attribute::TARGET_CLASS)]
final readonly class Menu
{
    public function __construct(
        public string $id,
        public string $label,
        public bool $tray = true,
        public ?string $tooltip = null,
        public TrayCloseBehavior $close = TrayCloseBehavior::Exit,
    ) {
    }
}
