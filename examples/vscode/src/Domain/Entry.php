<?php

declare(strict_types=1);

namespace App\Domain;

/** A visible row of the explorer: a folder or a file at its depth. */
final readonly class Entry
{
    public function __construct(
        public string $path,
        public string $name,
        public bool $folder,
        public int $depth,
        public bool $open = false,
    ) {
    }

    public function extension(): string
    {
        return $this->folder ? '' : FileKind::extension($this->name);
    }
}
