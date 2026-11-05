<?php

declare(strict_types=1);

namespace Pam\Desktop\Attributes;

use Attribute;
use Pam\Desktop\ApplicationCategory;
use Pam\Desktop\WindowTheme;

#[Attribute(Attribute::TARGET_CLASS)]
final readonly class Desktop
{
    public function __construct(
        public string $id,
        public string $name,
        public string $version = '1.0.0',
        public string $page = 'resources/index.html',
        public string $description = '',
        public ?string $publisher = null,
        public ApplicationCategory $category = ApplicationCategory::Utility,
        public WindowTheme $theme = WindowTheme::System,
        public int $width = 1120,
        public int $height = 720,
        public int $minimumWidth = 720,
        public int $minimumHeight = 520,
    ) {
    }
}
