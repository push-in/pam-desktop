<?php

declare(strict_types=1);

namespace Pam\Desktop\Code\Lsp;

/** A place in a file (0-based line and character). */
final readonly class Location
{
    public function __construct(public string $path, public int $line, public int $character)
    {
    }
}
