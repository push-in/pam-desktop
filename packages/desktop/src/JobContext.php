<?php

declare(strict_types=1);

namespace Pam\Desktop;

final readonly class JobContext
{
    public function __construct(
        public int $requestId,
        public string $id,
        public int $runId,
        public int $startedAtMilliseconds,
    ) {
    }
}
