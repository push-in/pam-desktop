<?php

declare(strict_types=1);

namespace Pam\Desktop\Dom;

final readonly class MutationRecord
{
    /** @param list<string> $identities */
    public function __construct(
        public int $version,
        public array $identities,
    ) {
    }
}
