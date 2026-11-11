<?php

declare(strict_types=1);

namespace App\Domain;

final readonly class Label
{
    public function __construct(public LabelColor $color, public string $name = '')
    {
    }
}
