<?php

declare(strict_types=1);

namespace Pam\DesktopSsh;

/** A directory listing, without `.` and `..`, in the server's order. */
final readonly class SftpListing
{
    /** @param list<SftpEntry> $entries */
    public function __construct(public array $entries, public bool $truncated)
    {
    }
}
