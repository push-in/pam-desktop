<?php

declare(strict_types=1);

namespace Pam\Desktop;

use RuntimeException;

final readonly class Shell
{
    /**
     * @param list<Menu> $menus
     * @param list<GlobalShortcut> $shortcuts
     * @param list<QuickAction> $quickActions
     */
    private function __construct(
        public array $menus,
        public ?Tray $tray,
        public array $shortcuts,
        public array $quickActions,
    ) {
        $menuIds = [];
        $itemIds = [];
        foreach ($menus as $menu) {
            if (isset($menuIds[$menu->id])) {
                throw new RuntimeException("Menu {$menu->id} is already registered.");
            }
            $menuIds[$menu->id] = true;
            self::collectItemIds($menu->items, $itemIds);
        }
        if ($tray !== null && !isset($menuIds[$tray->menuId])) {
            throw new RuntimeException("Tray menu {$tray->menuId} is not registered.");
        }

        $shortcutIds = [];
        $accelerators = [];
        foreach ($shortcuts as $shortcut) {
            if (isset($shortcutIds[$shortcut->id])) {
                throw new RuntimeException("Global shortcut {$shortcut->id} is already registered.");
            }
            $normalized = strtolower($shortcut->accelerator);
            if (isset($accelerators[$normalized])) {
                throw new RuntimeException(
                    "Global shortcut accelerator {$shortcut->accelerator} is already registered.",
                );
            }
            $shortcutIds[$shortcut->id] = true;
            $accelerators[$normalized] = true;
        }
        $quickActionIds = [];
        foreach ($quickActions as $quickAction) {
            if (isset($quickActionIds[$quickAction->id])) {
                throw new RuntimeException("Quick action {$quickAction->id} is already registered.");
            }
            $quickActionIds[$quickAction->id] = true;
        }
        if (count($quickActions) > 10) {
            throw new RuntimeException('A desktop application can register at most ten quick actions.');
        }
    }

    public static function none(): self
    {
        return new self([], null, [], []);
    }

    public function menu(Menu ...$menus): self
    {
        return new self(
            array_values(array_merge($this->menus, $menus)),
            $this->tray,
            $this->shortcuts,
            $this->quickActions,
        );
    }

    public function tray(Tray $tray): self
    {
        return new self($this->menus, $tray, $this->shortcuts, $this->quickActions);
    }

    public function shortcut(GlobalShortcut ...$shortcuts): self
    {
        return new self(
            $this->menus,
            $this->tray,
            array_values(array_merge($this->shortcuts, $shortcuts)),
            $this->quickActions,
        );
    }

    public function quickAction(QuickAction ...$quickActions): self
    {
        return new self(
            $this->menus,
            $this->tray,
            $this->shortcuts,
            array_values(array_merge($this->quickActions, $quickActions)),
        );
    }

    /**
     * @return array{
     *     menus: list<array{id: string, label: string, items: list<array<string, mixed>>}>,
     *     tray: null|array{menuId: string, tooltip: string, closeBehavior: int},
     *     shortcuts: list<array{id: string, accelerator: string}>,
     *     quickActions: list<array{id: string, label: string, description: string}>
     * }
     */
    public function toArray(): array
    {
        if (count($this->menus) > 1) {
            throw new RuntimeException(
                'The stable Linux shell accepts one tray menu per application.',
            );
        }
        if ($this->menus !== [] && $this->tray === null) {
            throw new RuntimeException(
                'Native menus require a tray configuration on Linux.',
            );
        }

        return [
            'menus' => array_map(
                static fn (Menu $menu): array => $menu->toArray(),
                $this->menus,
            ),
            'tray' => $this->tray?->toArray(),
            'shortcuts' => array_map(
                static fn (GlobalShortcut $shortcut): array => $shortcut->toArray(),
                $this->shortcuts,
            ),
            'quickActions' => array_map(
                static fn (QuickAction $quickAction): array => $quickAction->toArray(),
                $this->quickActions,
            ),
        ];
    }

    /**
     * @param list<MenuItem> $items
     * @param array<string, true> $ids
     */
    private static function collectItemIds(array $items, array &$ids): void
    {
        foreach ($items as $item) {
            if ($item->kind !== MenuItemKind::Separator) {
                if (isset($ids[$item->id])) {
                    throw new RuntimeException(
                        "Menu item {$item->id} is already registered in another menu.",
                    );
                }
                $ids[$item->id] = true;
            }
            self::collectItemIds($item->items, $ids);
        }
    }
}
