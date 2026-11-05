<?php

declare(strict_types=1);

namespace Pam\Desktop;

final readonly class ClientEvent
{
    public function __construct(
        public string $name,
        public mixed $payload = null,
        public ?string $windowId = null,
    ) {
        Identifier::assert($name, 'The event name');
        if ($windowId !== null) {
            Identifier::assert($windowId, 'The target window identifier');
        }
    }

    public function to(string $windowId): self
    {
        return new self($this->name, $this->payload, $windowId);
    }

    /**
     * @return array{name: string, payload: mixed, windowId?: string}
     */
    public function toArray(): array
    {
        $event = [
            'name' => $this->name,
            'payload' => $this->payload,
        ];
        if ($this->windowId !== null) {
            $event['windowId'] = $this->windowId;
        }

        return $event;
    }
}
