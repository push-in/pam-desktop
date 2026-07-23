<?php

declare(strict_types=1);

namespace Pam\Desktop;

final readonly class CommandContext
{
    public function __construct(
        public int $id,
        public string $name,
        public mixed $payload,
    ) {
    }

    public function string(string $key, ?string $default = null): ?string
    {
        if (!is_array($this->payload)) {
            return $default;
        }

        $value = $this->payload[$key] ?? null;

        return is_string($value) ? $value : $default;
    }

    public function integer(string $key, ?int $default = null): ?int
    {
        if (!is_array($this->payload)) {
            return $default;
        }

        $value = $this->payload[$key] ?? null;

        return is_int($value) ? $value : $default;
    }

    public function boolean(string $key, ?bool $default = null): ?bool
    {
        if (!is_array($this->payload)) {
            return $default;
        }

        $value = $this->payload[$key] ?? null;

        return is_bool($value) ? $value : $default;
    }
}

