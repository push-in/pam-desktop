<?php

declare(strict_types=1);

namespace Pam\Desktop;

final readonly class CommandResult
{
    /**
     * @param list<Effect> $effects
     */
    private function __construct(
        public mixed $payload,
        public array $effects,
    ) {
    }

    public static function success(mixed $payload = null): self
    {
        return new self($payload, []);
    }

    public function effect(Effect $effect): self
    {
        return new self($this->payload, [...$this->effects, $effect]);
    }
}

