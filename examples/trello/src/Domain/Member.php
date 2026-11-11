<?php

declare(strict_types=1);

namespace App\Domain;

final readonly class Member
{
    public function __construct(public string $id, public string $name, public string $color)
    {
    }

    public function initials(): string
    {
        $parts = preg_split('/\s+/', trim($this->name)) ?: [];

        return strtoupper(mb_substr($parts[0] ?? '', 0, 1).mb_substr($parts[1] ?? '', 0, 1));
    }
}
