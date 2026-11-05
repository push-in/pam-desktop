<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final readonly class Menu
{
    /**
     * @param list<MenuItem> $items
     */
    private function __construct(
        public string $id,
        public string $label,
        public array $items,
    ) {
        Identifier::assert($id, 'The menu identifier');
        if (trim($label) === '' || strlen($label) > 80) {
            throw new InvalidArgumentException(
                'Menu labels must contain at most 80 bytes of printable text.',
            );
        }
        self::assertUniqueItems($items);
    }

    public static function create(string $id, string $label, MenuItem ...$items): self
    {
        return new self($id, $label, array_values($items));
    }

    public function item(MenuItem ...$items): self
    {
        return new self(
            $this->id,
            $this->label,
            array_values(array_merge($this->items, $items)),
        );
    }

    /**
     * @return array{id: string, label: string, items: list<array<string, mixed>>}
     */
    public function toArray(): array
    {
        if ($this->items === []) {
            throw new InvalidArgumentException("The menu {$this->id} must contain at least one item.");
        }

        return [
            'id' => $this->id,
            'label' => $this->label,
            'items' => array_map(
                static fn (MenuItem $item): array => $item->toArray(),
                $this->items,
            ),
        ];
    }

    /**
     * @param list<MenuItem> $items
     */
    private static function assertUniqueItems(array $items): void
    {
        $ids = [];
        $count = 0;
        $walk = static function (array $children, int $depth) use (&$walk, &$ids, &$count): void {
            if ($depth > 8) {
                throw new InvalidArgumentException('Native menu nesting cannot exceed eight levels.');
            }
            foreach ($children as $item) {
                ++$count;
                if ($item->kind !== MenuItemKind::Separator) {
                    if (isset($ids[$item->id])) {
                        throw new InvalidArgumentException(
                            "The menu item {$item->id} is duplicated.",
                        );
                    }
                    $ids[$item->id] = true;
                }
                $walk($item->items, $depth + 1);
            }
        };
        $walk($items, 1);
        if ($count > 256) {
            throw new InvalidArgumentException('Native menus cannot contain more than 256 items.');
        }
    }
}
