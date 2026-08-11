<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final readonly class Database
{
    private function __construct(
        public string $name,
        public string $path,
        public DatabaseAccess $access,
    ) {
        Identifier::assert($name, 'The database identifier');
        if ($path === '' || str_contains($path, "\0")) {
            throw new InvalidArgumentException('The database path must not be empty.');
        }
    }

    public static function read(string $name, string $path): self
    {
        return new self($name, $path, DatabaseAccess::Read);
    }

    public static function readWrite(string $name, string $path): self
    {
        return new self($name, $path, DatabaseAccess::ReadWrite);
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
