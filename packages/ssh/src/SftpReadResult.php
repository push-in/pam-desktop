<?php

declare(strict_types=1);

namespace Pam\DesktopSsh;

/** The start of a remote file: `$bytes`, the file's `$size` and whether bytes were left out. */
final readonly class SftpReadResult
{
    public function __construct(public string $bytes, public int $size, public bool $truncated)
    {
    }
}
