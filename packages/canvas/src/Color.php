<?php

declare(strict_types=1);

namespace Pam\Desktop\Canvas;

use InvalidArgumentException;
use Stringable;

/**
 * A CSS hex color accepted by both renderers: `#rgb`, `#rrggbb` or
 * `#rrggbbaa` (RGBA order, as in CSS). Anything else is rejected.
 */
final readonly class Color implements Stringable
{
    private const string PATTERN = '/^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/';

    private function __construct(public string $value)
    {
    }

    /**
     * Wraps a validated CSS hex color.
     *
     * @throws InvalidArgumentException when the string is not a hex color
     */
    public static function from(string $value): self
    {
        return new self(self::assert($value));
    }

    /**
     * Validates a CSS hex color and returns it unchanged.
     *
     * @throws InvalidArgumentException when the string is not a hex color
     */
    public static function assert(string $value): string
    {
        if (preg_match(self::PATTERN, $value) !== 1) {
            throw new InvalidArgumentException(
                'Canvas colors must be #rgb, #rrggbb or #rrggbbaa.',
            );
        }

        return $value;
    }

    /**
     * Whether the string is a color this package accepts.
     */
    public static function isValid(string $value): bool
    {
        return preg_match(self::PATTERN, $value) === 1;
    }

    public function __toString(): string
    {
        return $this->value;
    }
}
