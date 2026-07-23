<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final readonly class FileSystemRoot
{
    private function __construct(
        public string $name,
        public string $path,
        public FileAccess $access,
    ) {
        Identifier::assert($name, 'The filesystem root identifier');
        if (trim($path) === '' || str_contains($path, "\0")) {
            throw new InvalidArgumentException('The filesystem root path cannot be empty.');
        }
    }

    public static function read(string $name, string $path): self
    {
        return new self($name, $path, FileAccess::Read);
    }

    public static function write(string $name, string $path): self
    {
        return new self($name, $path, FileAccess::Write);
    }

    public static function readWrite(string $name, string $path): self
    {
        return new self($name, $path, FileAccess::ReadWrite);
    }

    /**
     * @return array{name: string, path: string, access: int}
     */
    public function toArray(): array
    {
        return [
            'name' => $this->name,
            'path' => $this->path,
            'access' => $this->access->value,
        ];
    }
}
