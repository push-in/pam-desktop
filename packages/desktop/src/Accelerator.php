<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final class Accelerator
{
    public static function assert(string $accelerator): void
    {
        if (
            $accelerator === ''
            || strlen($accelerator) > 64
            || preg_match('/\s/u', $accelerator) === 1
        ) {
            throw new InvalidArgumentException(
                'Keyboard accelerators must contain 1-64 non-whitespace characters.',
            );
        }

        $tokens = explode('+', $accelerator);
        if (in_array('', $tokens, true)) {
            throw new InvalidArgumentException("The keyboard accelerator {$accelerator} is malformed.");
        }

        $modifiers = [];
        foreach (array_slice($tokens, 0, -1) as $modifier) {
            $normalized = strtolower($modifier);
            if (
                !in_array(
                    $normalized,
                    ['alt', 'ctrl', 'control', 'shift', 'super', 'cmd', 'command', 'cmdorctrl'],
                    true,
                )
                || isset($modifiers[$normalized])
            ) {
                throw new InvalidArgumentException(
                    "The keyboard accelerator {$accelerator} has an invalid or duplicate modifier.",
                );
            }
            $modifiers[$normalized] = true;
        }

        $key = $tokens[array_key_last($tokens)];
        if (strlen($key) > 24 || preg_match('/\A[A-Za-z0-9]+\z/D', $key) !== 1) {
            throw new InvalidArgumentException(
                "The keyboard accelerator {$accelerator} has an invalid key.",
            );
        }
    }
}
