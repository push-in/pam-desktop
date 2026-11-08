<?php

declare(strict_types=1);

namespace Pam\Desktop\Canvas;

use InvalidArgumentException;

/**
 * One entry of the display list: a kind plus its bounded argument list.
 */
final readonly class CanvasCommand
{
    public const int MAX_ARGUMENTS = 16;
    public const int MAX_TEXT_LENGTH = 4096;

    /**
     * @param list<float|int|string> $arguments
     *
     * @throws InvalidArgumentException when an argument is out of bounds
     */
    public function __construct(
        public DisplayCommandKind $kind,
        public array $arguments = [],
    ) {
        if (count($arguments) > self::MAX_ARGUMENTS) {
            throw new InvalidArgumentException('Canvas commands accept at most 16 arguments.');
        }

        foreach ($arguments as $value) {
            if (is_string($value)) {
                if (strlen($value) > self::MAX_TEXT_LENGTH || str_contains($value, "\0")) {
                    throw new InvalidArgumentException('Canvas text is invalid.');
                }
            } elseif (!is_finite((float) $value)) {
                throw new InvalidArgumentException('Canvas numbers must be finite.');
            }
        }
    }

    /**
     * @return array{k: int, a: list<float|int|string>}
     */
    public function toArray(): array
    {
        return ['k' => $this->kind->value, 'a' => $this->arguments];
    }
}
