<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

/** A named group of file extensions offered by a file dialog. */
final readonly class FileFilter
{
    /** @var list<string> */
    public array $extensions;

    /** @param string ...$extensions without the dot (`png`); `*` accepts every file */
    public function __construct(public string $name, string ...$extensions)
    {
        if (trim($name) === '' || $extensions === []) {
            throw new InvalidArgumentException('A file filter needs a name and at least one extension.');
        }
        $this->extensions = array_values(array_map(
            static fn (string $extension): string => strtolower(ltrim($extension, '.')),
            $extensions,
        ));
    }

    public static function images(string $name = 'Images'): self
    {
        return new self($name, 'png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'svg');
    }

    public static function all(string $name = 'All files'): self
    {
        return new self($name, '*');
    }
}
