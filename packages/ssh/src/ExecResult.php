<?php

declare(strict_types=1);

namespace Pam\DesktopSsh;

/** The outcome of SshSession::exec(). Output is raw bytes, not always UTF-8. */
final readonly class ExecResult
{
    public function __construct(
        public int $exitCode,
        public string $stdout,
        public string $stderr,
        public bool $truncated,
    ) {
    }

    public function succeeded(): bool
    {
        return $this->exitCode === 0;
    }
}
