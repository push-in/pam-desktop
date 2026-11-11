<?php

declare(strict_types=1);

namespace Pam\DesktopSsh;

use Throwable;

/**
 * The server presented a host key other than the expected one. The key may
 * have been rotated, or someone is in the middle: ask the user before
 * trusting `$fingerprint`.
 */
final class HostKeyMismatch extends SshException
{
    /** @param string $fingerprint SHA-256 of the presented key, base64 without padding or prefix. */
    public function __construct(public readonly string $fingerprint, ?Throwable $previous = null)
    {
        parent::__construct('host-key-mismatch:'.$fingerprint, $previous);
    }
}
