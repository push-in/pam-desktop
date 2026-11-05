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
        public string $windowId = 'main',
        public array $payload = [],
    ) {
        Identifier::assert($windowId, 'The effect window identifier');
    }

    /**
     * @return array{kind: int, windowId: string, payload: array<string, mixed>}
     */
    public function toArray(): array
    {
        return [
            'kind' => $this->kind->value,
            'windowId' => $this->windowId,
            'payload' => $this->payload,
        ];
    }
}
