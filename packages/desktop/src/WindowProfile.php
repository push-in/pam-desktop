<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final readonly class WindowProfile
{
    private function __construct(
        public WindowRole $role,
        public ?string $parent,
        public bool $restore,
        public bool $rememberMonitor,
        public bool $detachableTabs,
    ) {
        if ($parent !== null) {
            Identifier::assert($parent, 'The parent window identifier');
        }
        if ($role === WindowRole::Primary && $parent !== null) {
            throw new InvalidArgumentException('A primary window cannot declare a parent.');
        }
        if (in_array($role, [WindowRole::Child, WindowRole::Modal, WindowRole::Popover], true)
            && $parent === null) {
            throw new InvalidArgumentException('Child, modal, and popover windows require a parent.');
        }
    }

    public static function primary(): self
    {
        return new self(WindowRole::Primary, null, true, true, true);
    }

    public static function child(string $parent = 'main'): self
    {
        return new self(WindowRole::Child, $parent, true, true, true);
    }

    public static function modal(string $parent = 'main'): self
    {
        return new self(WindowRole::Modal, $parent, false, false, false);
    }

    public static function popover(string $parent = 'main'): self
    {
        return new self(WindowRole::Popover, $parent, false, false, false);
    }

    public static function panel(): self
    {
        return new self(WindowRole::Panel, null, true, true, true);
    }

    public static function palette(): self
    {
        return new self(WindowRole::Palette, null, true, true, false);
    }

    /** @return array{role: int, parent: ?string, restore: bool, rememberMonitor: bool, detachableTabs: bool} */
    public function toArray(): array
    {
        return [
            'role' => $this->role->value,
            'parent' => $this->parent,
            'restore' => $this->restore,
            'rememberMonitor' => $this->rememberMonitor,
            'detachableTabs' => $this->detachableTabs,
        ];
    }
}
