<?php

declare(strict_types=1);

namespace Pam\Desktop;

final readonly class Effect
{
    /**
     * @param array<string, mixed> $payload
     */
    public function __construct(
        public EffectKind $kind,
        public array $payload = [],
    ) {
    }

    /**
     * @return array{kind: int, payload: array<string, mixed>}
     */
    public function toArray(): array
    {
        return [
            'kind' => $this->kind->value,
            'payload' => $this->payload,
        ];
    }
}

