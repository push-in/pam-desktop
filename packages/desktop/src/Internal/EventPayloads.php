<?php

declare(strict_types=1);

namespace Pam\Desktop\Internal;

use Pam\Desktop\DragPayload;
use Pam\Desktop\GestureEvent;
use Pam\Desktop\GestureSettleEvent;
use Pam\Desktop\ImageErrorEvent;
use Pam\Desktop\ImageLoadEvent;
use Pam\Desktop\ImageProgressEvent;
use Pam\Desktop\KeyEvent;
use Pam\Desktop\LayoutEvent;
use Pam\Desktop\PageChangeEvent;
use Pam\Desktop\PressEvent;
use Pam\Desktop\ScrollPhaseEvent;
use Pam\Desktop\TextLayoutEvent;
use ReflectionMethod;
use ReflectionNamedType;

/**
 * Typed native event payloads. Template handlers whose parameter is typed
 * with one of these classes receive the decoded object instead of the wire
 * string.
 *
 * @internal
 */
final class EventPayloads
{
    public const array CLASSES = [
        PressEvent::class => true,
        GestureEvent::class => true,
        GestureSettleEvent::class => true,
        ImageErrorEvent::class => true,
        ImageLoadEvent::class => true,
        ImageProgressEvent::class => true,
        ScrollPhaseEvent::class => true,
        PageChangeEvent::class => true,
        TextLayoutEvent::class => true,
        KeyEvent::class => true,
        LayoutEvent::class => true,
        DragPayload::class => true,
    ];

    private function __construct()
    {
    }

    /** @return class-string|null */
    public static function parameterClass(ReflectionMethod $method, int $index): ?string
    {
        $parameter = $method->getParameters()[$index] ?? null;
        $type = $parameter?->getType();
        if ($type instanceof ReflectionNamedType && !$type->isBuiltin() && isset(self::CLASSES[$type->getName()])) {
            return $type->getName();
        }

        return null;
    }

    public static function decode(string $class, string $payload): object
    {
        return $class::fromPayload($payload);
    }
}
