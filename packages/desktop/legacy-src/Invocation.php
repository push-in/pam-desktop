<?php

declare(strict_types=1);

namespace Pam\Desktop;

final class Invocation
{
    /** @var list<Effect> */
    private array $effects = [];

    /** @var list<ClientEvent> */
    private array $events = [];

    public function effect(Effect $effect): void
    {
        $this->effects[] = $effect;
    }

    public function event(ClientEvent $event): void
    {
        $this->events[] = $event;
    }

    public function result(mixed $payload): CommandResult
    {
        $result = $payload instanceof CommandResult
            ? $payload
            : CommandResult::success($payload);
        foreach ($this->effects as $effect) {
            $result = $result->effect($effect);
        }
        foreach ($this->events as $event) {
            $result = $result->event($event);
        }

        return $result;
    }
}
