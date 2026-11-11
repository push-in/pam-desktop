<?php

declare(strict_types=1);

namespace App\Domain;

final readonly class Member
{
    public function __construct(
        public string $id,
        public string $name,
        public string $title,
        public string $avatar,
        public bool $active = true,
    ) {
    }
}
