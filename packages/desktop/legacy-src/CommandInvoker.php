<?php

declare(strict_types=1);

namespace Pam\Desktop;

use BackedEnum;
use ReflectionMethod;
use ReflectionNamedType;
use ReflectionParameter;
use RuntimeException;

final readonly class CommandInvoker
{
    /**
     * @param list<string> $windows
     * @param array<class-string<DesktopWindow>, string> $windowClasses
     */
    public function __construct(
        private Container $container,
        private array $windows,
        private array $windowClasses = [],
    ) {
    }

    public function invoke(object $target, string $method, CommandContext $context): CommandResult
    {
        return $this->invokeContext($target, $method, $context, $context->payload);
    }

    public function invokeEvent(object $target, string $method, EventContext $context): CommandResult
    {
        return $this->invokeContext($target, $method, $context, $context->payload);
    }

    private function invokeContext(
        object $target,
        string $method,
        CommandContext|EventContext $context,
        mixed $rawPayload,
    ): CommandResult {
        $invocation = new Invocation();
        $windows = new Windows($invocation, $context->windowId, $this->windows);
        $container = $this->container->contextual([
            Invocation::class => $invocation,
            Windows::class => $windows,
            WindowHandle::class => $windows->current(),
            Events::class => new Events($invocation),
            ApplicationControl::class => new ApplicationControl($invocation),
        ]);
        $namedWindows = [];
        foreach ($this->windowClasses as $class => $id) {
            $reflection = new \ReflectionClass($class);
            $namedWindows[$class] = $reflection->newInstance(new WindowHandle($id, $invocation));
        }
        $container = $container->contextual($namedWindows);
        $container = $context instanceof CommandContext
            ? $container->contextual([CommandContext::class => $context])
            : $container->contextual([EventContext::class => $context]);
        $reflection = new ReflectionMethod($target, $method);
        $payload = is_array($rawPayload) ? $rawPayload : [];
        $arguments = array_map(
            fn (ReflectionParameter $parameter): mixed => $this->argument($parameter, $payload, $container),
            $reflection->getParameters(),
        );
        $result = $reflection->invokeArgs($target, $arguments);

        return $invocation->result($result);
    }

    /** @param array<string, mixed> $payload */
    private function argument(
        ReflectionParameter $parameter,
        array $payload,
        Container $container,
    ): mixed {
        $type = $parameter->getType();
        if (!$type instanceof ReflectionNamedType) {
            throw new RuntimeException("Command parameter \${$parameter->getName()} must have one named type.");
        }

        if (!$type->isBuiltin()) {
            /** @var class-string $class */
            $class = $type->getName();
            if (is_subclass_of($class, BackedEnum::class) && array_key_exists($parameter->getName(), $payload)) {
                $value = $payload[$parameter->getName()];
                if (!is_int($value) && !is_string($value)) {
                    throw new RuntimeException("Enum command argument {$parameter->getName()} must be scalar.");
                }

                return $class::from($value);
            }
            if (array_key_exists($parameter->getName(), $payload) && is_array($payload[$parameter->getName()])) {
                return $this->hydrate(
                    $class,
                    $this->objectPayload($parameter->getName(), $payload[$parameter->getName()]),
                    $container,
                );
            }

            return $container->get($class);
        }

        if (!array_key_exists($parameter->getName(), $payload)) {
            if ($parameter->isDefaultValueAvailable()) {
                return $parameter->getDefaultValue();
            }
            if ($type->allowsNull()) {
                return null;
            }
            throw new RuntimeException("Required command argument {$parameter->getName()} is missing.");
        }

        $value = $payload[$parameter->getName()];
        if ($value === null && $type->allowsNull()) {
            return null;
        }
        if (!$this->matchesBuiltin($type->getName(), $value)) {
            throw new RuntimeException(
                "Command argument {$parameter->getName()} must be {$type->getName()}.",
            );
        }

        return $value;
    }

    /**
     * @param class-string $class
     * @param array<string, mixed> $payload
     */
    private function hydrate(string $class, array $payload, Container $container): object
    {
        $reflection = new \ReflectionClass($class);
        $constructor = $reflection->getConstructor();
        if ($constructor === null) {
            return $container->get($class);
        }
        $arguments = array_map(
            fn (ReflectionParameter $parameter): mixed => $this->argument($parameter, $payload, $container),
            $constructor->getParameters(),
        );

        return $reflection->newInstanceArgs($arguments);
    }

    /**
     * @param array<mixed, mixed> $payload
     * @return array<string, mixed>
     */
    private function objectPayload(string $parameter, array $payload): array
    {
        $object = [];
        foreach ($payload as $key => $value) {
            if (!is_string($key)) {
                throw new RuntimeException("DTO command argument {$parameter} must be a JSON object.");
            }
            $object[$key] = $value;
        }

        return $object;
    }

    private function matchesBuiltin(string $type, mixed $value): bool
    {
        return match ($type) {
            'string' => is_string($value),
            'int' => is_int($value),
            'float' => is_float($value) || is_int($value),
            'bool' => is_bool($value),
            'array' => is_array($value),
            'object' => is_object($value),
            'iterable' => is_iterable($value),
            'mixed' => true,
            default => false,
        };
    }
}
