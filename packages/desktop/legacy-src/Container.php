<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Closure;
use ReflectionClass;
use ReflectionNamedType;
use RuntimeException;

final class Container
{
    /** @var array<string, object|Closure(self): object> */
    private array $bindings = [];

    /**
     * @param class-string $id
     * @param object|Closure(self): object $value
     */
    public function bind(string $id, object $value): self
    {
        $this->bindings[$id] = $value;

        return $this;
    }

    /** @param array<class-string, object> $bindings */
    public function contextual(array $bindings): self
    {
        $container = clone $this;
        foreach ($bindings as $id => $value) {
            $container->bindings[$id] = $value;
        }

        return $container;
    }

    /**
     * @template T of object
     * @param class-string<T> $id
     * @return T
     */
    public function get(string $id): object
    {
        $binding = $this->bindings[$id] ?? null;
        if ($binding !== null) {
            $resolved = $binding instanceof Closure ? $binding($this) : $binding;
            if (!$resolved instanceof $id) {
                throw new RuntimeException("The container binding for {$id} returned an incompatible object.");
            }

            return $resolved;
        }

        if (!class_exists($id) && !interface_exists($id)) {
            throw new RuntimeException("Container entry {$id} does not name a class or interface.");
        }
        $reflection = new ReflectionClass($id);
        if (!$reflection->isInstantiable()) {
            throw new RuntimeException("{$id} is not instantiable and has no container binding.");
        }

        $constructor = $reflection->getConstructor();
        if ($constructor === null) {
            return $reflection->newInstance();
        }

        $arguments = [];
        foreach ($constructor->getParameters() as $parameter) {
            $type = $parameter->getType();
            if ($type instanceof ReflectionNamedType && !$type->isBuiltin()) {
                /** @var class-string $dependency */
                $dependency = $type->getName();
                $arguments[] = $this->get($dependency);
            } elseif ($parameter->isDefaultValueAvailable()) {
                $arguments[] = $parameter->getDefaultValue();
            } else {
                throw new RuntimeException(
                    "Cannot resolve constructor parameter {$id}::\${$parameter->getName()}.",
                );
            }
        }

        return $reflection->newInstanceArgs($arguments);
    }
}
