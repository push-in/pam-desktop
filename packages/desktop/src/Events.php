<?php

declare(strict_types=1);

namespace Pam\Desktop;

use JsonSerializable;
use ReflectionClass;

final readonly class Events
{
    public function __construct(private Invocation $invocation)
    {
    }

    /** @param Event|object|string $event */
    public function emit(object|string $event, mixed $payload = null, ?string $windowId = null): void
    {
        if (is_string($event)) {
            $this->invocation->event(new ClientEvent($event, $payload, $windowId));

            return;
        }

        if ($event instanceof Event) {
            $name = $event->name();
            $payload = $event->payload();
        } else {
            $name = self::eventName($event);
            $payload = $event instanceof JsonSerializable
                ? $event->jsonSerialize()
                : get_object_vars($event);
        }

        $this->invocation->event(new ClientEvent($name, $payload, $windowId));
    }

    private static function eventName(object $event): string
    {
        $shortName = (new ReflectionClass($event))->getShortName();
        $words = preg_replace('/(?<!^)[A-Z]/', '.$0', $shortName) ?? $shortName;

        return strtolower($words);
    }
}
