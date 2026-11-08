<?php

declare(strict_types=1);

namespace Pam\Desktop\Sync\Durable;

final readonly class OutboxDispatchReport
{
    public function __construct(
        public int $claimed = 0,
        public int $applied = 0,
        public int $retried = 0,
        public int $rejected = 0,
        public int $stale = 0,
        public int $failures = 0,
    ) {}
}
