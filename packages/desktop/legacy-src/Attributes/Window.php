<?php

declare(strict_types=1);

namespace Pam\Desktop\Attributes;

use Attribute;
use Pam\Desktop\WindowTheme;

#[Attribute(Attribute::TARGET_CLASS)]
final readonly class Window
{
    public function __construct(
        public string $name,
        public string $title,
        public string $page,
        public int $width = 800,
        public int $height = 600,
        public int $minimumWidth = 480,
        public int $minimumHeight = 360,
        public bool $visible = false,
        public WindowTheme $theme = WindowTheme::System,
    ) {
    }
}
