<?php

declare(strict_types=1);

namespace Pam\DesktopSsh;

/**
 * One event of a session, as the host sent it. Shells, transfers and forwards
 * receive their own events through their callbacks; SshSession::onEvent()
 * sees every event.
 */
final readonly class SshEvent
{
    /** @param array<string, string|int|float|bool> $values */
    public function __construct(public SshEventKind $kind, public array $values)
    {
    }

    public function string(string $key): string
    {
        return Native::string($this->values, $key);
    }

    public function int(string $key, int $default = 0): int
    {
        return Native::int($this->values, $key, $default);
    }
}
