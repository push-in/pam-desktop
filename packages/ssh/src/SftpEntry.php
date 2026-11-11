<?php

declare(strict_types=1);

namespace Pam\DesktopSsh;

/** A remote file, directory or link. Fields the server did not send are null. */
final readonly class SftpEntry
{
    public function __construct(
        public string $name,
        public SftpEntryKind $kind,
        public ?int $size = null,
        public ?int $mtime = null,
        public ?string $permissions = null,
        public ?string $owner = null,
        public ?string $group = null,
        public ?int $uid = null,
        public ?int $gid = null,
    ) {
    }

    /** @param array<array-key, mixed> $values A listing entry or a stat result. */
    public static function fromArray(array $values): self
    {
        $string = static fn (string $key): ?string => isset($values[$key]) && is_string($values[$key]) ? $values[$key] : null;
        $int = static fn (string $key): ?int => isset($values[$key]) && is_int($values[$key]) ? $values[$key] : null;

        return new self(
            $string('name') ?? '',
            SftpEntryKind::tryFrom($string('kind') ?? '') ?? SftpEntryKind::Other,
            $int('size'),
            $int('mtime'),
            $string('permissions'),
            $string('owner'),
            $string('group'),
            $int('uid'),
            $int('gid'),
        );
    }

    public function isDirectory(): bool
    {
        return $this->kind === SftpEntryKind::Directory;
    }
}
