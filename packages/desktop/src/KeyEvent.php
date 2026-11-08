<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Pam\Desktop\Internal\Wire;

/**
 * A key pressed or released on a desktop keyboard.
 */
final readonly class KeyEvent
{
    public function __construct(
        /** The key as the W3C UI Events `key` value: "Enter", "ArrowLeft", "a", "F5"… */
        public string $key,
        public bool $shift = false,
        public bool $control = false,
        public bool $alt = false,
        /** Command on macOS, the Windows/Super key elsewhere. */
        public bool $meta = false,
        public bool $repeat = false,
    ) {
    }

    public static function fromPayload(string $payload): self
    {
        $values = $payload === '' ? [] : Wire::decodeMap($payload);

        return new self(
            is_string($values['key'] ?? null) ? $values['key'] : '',
            ($values['shift'] ?? false) === true,
            ($values['control'] ?? false) === true,
            ($values['alt'] ?? false) === true,
            ($values['meta'] ?? false) === true,
            ($values['repeat'] ?? false) === true,
        );
    }

    /** True for Command on macOS and Control elsewhere: the primary shortcut modifier. */
    public function primary(): bool
    {
        return Platform::isMacOS() ? $this->meta : $this->control;
    }
}
