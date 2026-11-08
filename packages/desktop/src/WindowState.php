<?php

declare(strict_types=1);

namespace Pam\Desktop;

/** How the main window currently sits on the desktop. */
final readonly class WindowState
{
    public function __construct(
        public bool $maximized = false,
        public bool $minimized = false,
        public bool $fullScreen = false,
        public bool $focused = true,
        public float $width = 0.0,
        public float $height = 0.0,
    ) {
    }

    /** @internal */
    public static function fromJson(string $json): self
    {
        $values = json_decode($json, true);

        return is_array($values) ? self::fromArray($values) : new self();
    }

    /**
     * @internal
     * @param array<array-key, mixed> $values
     */
    public static function fromArray(array $values): self
    {
        $number = static fn (mixed $value): float => is_int($value) || is_float($value) ? (float) $value : 0.0;

        return new self(
            ($values['maximized'] ?? false) === true,
            ($values['minimized'] ?? false) === true,
            ($values['fullScreen'] ?? false) === true,
            ($values['focused'] ?? true) === true,
            $number($values['width'] ?? null),
            $number($values['height'] ?? null),
        );
    }
}
