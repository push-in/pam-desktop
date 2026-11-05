<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Closure;
use InvalidArgumentException;

/**
 * One entry of a menu: an action, a check item, a separator or a submenu.
 *
 * Items are immutable values; build the menu again to change it.
 */
final readonly class MenuItem
{
    /**
     * @param list<self> $children
     */
    private function __construct(
        public string $label,
        public ?Closure $handler = null,
        public ?string $shortcut = null,
        public bool $enabled = true,
        public ?bool $checked = null,
        public bool $destructive = false,
        public ?MenuRole $role = null,
        public bool $separator = false,
        public array $children = [],
        public ?string $id = null,
    ) {
    }

    /**
     * An item that runs `$handler` when chosen.
     *
     * `$shortcut` is a key combination such as `CmdOrCtrl+Shift+N` (Command
     * on macOS, Control elsewhere), `Alt+F4` or `F11`.
     */
    public static function make(
        string $label,
        ?Closure $handler = null,
        ?string $shortcut = null,
        bool $enabled = true,
        bool $destructive = false,
        ?string $id = null,
    ): self {
        if (trim($label) === '') {
            throw new InvalidArgumentException('A menu item needs a label.');
        }

        return new self($label, $handler, $shortcut, $enabled, null, $destructive, id: $id);
    }

    /** An item with a check mark. The handler receives the new state. */
    public static function checkbox(
        string $label,
        bool $checked,
        ?Closure $handler = null,
        ?string $shortcut = null,
        bool $enabled = true,
        ?string $id = null,
    ): self {
        if (trim($label) === '') {
            throw new InvalidArgumentException('A menu item needs a label.');
        }
        $toggle = $handler === null ? null : static fn (): mixed => $handler(!$checked);

        return new self($label, $toggle, $shortcut, $enabled, $checked, id: $id);
    }

    /** An item the operating system implements (Quit, Copy, Paste, Minimize…). */
    public static function role(MenuRole $role, ?string $label = null, ?string $shortcut = null): self
    {
        return new self($label ?? $role->label(), null, $shortcut ?? $role->shortcut(), role: $role);
    }

    public static function separator(): self
    {
        return new self('', separator: true);
    }

    public static function submenu(string $label, self ...$items): self
    {
        if (trim($label) === '') {
            throw new InvalidArgumentException('A submenu needs a label.');
        }

        return new self($label, children: array_values($items));
    }
}
