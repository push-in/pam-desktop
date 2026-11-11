<?php

declare(strict_types=1);

namespace App\Domain;

/** One styled piece of a message body. */
final readonly class Run
{
    public function __construct(
        public string $text,
        public string $style = 'plain',
    ) {
    }
}
