<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final readonly class MenuItem
{
    /**
     * @param list<MenuItem> $items
     */
    private function __construct(
        public MenuItemKind $kind,
        public string $id,
        public string $label,
        public bool $enabled,
        public bool $checked,
        public ?string $accelerator,
        public array $items,
    ) {
        if ($kind === MenuItemKind::Separator) {
            if ($id !== '' || $label !== '' || $checked || $accelerator !== null || $items !== []) {
                throw new InvalidArgumentException(
                    'Menu separators cannot define identity, text, state, or children.',
                );
            }

            return;
        }

        Identifier::assert($id, 'The menu item identifier');
        if (trim($label) === '' || strlen($label) > 80) {
            throw new InvalidArgumentException(
                'Menu item labels must contain at most 80 bytes of printable text.',
            );
        }
        if ($accelerator !== null) {
            Accelerator::assert($accelerator);
        }
        if ($kind === MenuItemKind::Submenu && $items === []) {
            throw new InvalidArgumentException('Submenus must contain at least one child item.');
        }
        if ($kind !== MenuItemKind::Submenu && $items !== []) {
            throw new InvalidArgumentException('Only submenus can contain child items.');
        }
        if ($kind !== MenuItemKind::Checkbox && $checked) {
            throw new InvalidArgumentException('Only checkbox menu items can be checked.');
        }
        if ($kind === MenuItemKind::Submenu && $accelerator !== null) {
            throw new InvalidArgumentException('Submenus cannot define accelerators.');
        }
    }

    public static function command(
        string $id,
        string $label,
        ?string $accelerator = null,
    ): self {
        return new self(MenuItemKind::Command, $id, $label, true, false, $accelerator, []);
    }

    public static function checkbox(
        string $id,
        string $label,
        bool $checked = false,
        ?string $accelerator = null,
    ): self {
        return new self(MenuItemKind::Checkbox, $id, $label, true, $checked, $accelerator, []);
    }

    public static function separator(): self
    {
        return new self(MenuItemKind::Separator, '', '', true, false, null, []);
    }

    public static function submenu(string $id, string $label, self ...$items): self
    {
        return new self(
            MenuItemKind::Submenu,
            $id,
            $label,
            true,
            false,
            null,
            array_values($items),
        );
    }

    public function enabled(bool $enabled = true): self
    {
        return new self(
            $this->kind,
            $this->id,
            $this->label,
            $enabled,
            $this->checked,
            $this->accelerator,
            $this->items,
        );
    }

    public function checked(bool $checked = true): self
    {
        return new self(
            $this->kind,
            $this->id,
            $this->label,
            $this->enabled,
            $checked,
            $this->accelerator,
            $this->items,
        );
    }

    /**
     * @return array{
     *     kind: int,
     *     id: string,
     *     label: string,
     *     enabled: bool,
     *     checked: bool,
     *     accelerator: ?string,
     *     items: list<array<string, mixed>>
     * }
     */
    public function toArray(): array
    {
        return [
            'kind' => $this->kind->value,
            'id' => $this->id,
            'label' => $this->label,
            'enabled' => $this->enabled,
            'checked' => $this->checked,
            'accelerator' => $this->accelerator,
            'items' => array_map(
                static fn (self $item): array => $item->toArray(),
                $this->items,
            ),
        ];
    }
}
