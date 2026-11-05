<?php

declare(strict_types=1);

namespace Pam\Desktop\Attributes;

use Attribute;

#[Attribute(Attribute::TARGET_METHOD)]
final readonly class Listen
{
    public function __construct(public ?string $event = null)
    {
    }
}
