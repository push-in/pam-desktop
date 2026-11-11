<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

use Closure;
use InvalidArgumentException;

/**
 * One item of a `TreeView`. `children` null is a leaf; a list (possibly
 * empty) or a closure that returns one is a branch, the closure called the
 * first time the branch opens (a folder read from disk).
 */
final class TreeNode
{
    /** @var list<self>|null */
    private ?array $loaded = null;

    /**
     * @param list<self>|(Closure(): list<self>)|null $children
     * @param string|null $icon icon name in the tree's icon font (Lucide)
     */
    public function __construct(
        public readonly string $id,
        public readonly string $label,
        private readonly array|Closure|null $children = null,
        public readonly ?string $icon = null,
        public readonly int|string|null $iconColor = null,
        public readonly int|string|null $color = null,
        /** Dim text after the label (a path, a count). */
        public readonly ?string $detail = null,
        /** A short mark at the end of the row (`M`, `2`). */
        public readonly ?string $badge = null,
        public readonly int|string|null $badgeColor = null,
        public readonly ?string $tooltip = null,
        /** @var list<\Pam\Desktop\MenuItem> the menu of a secondary click on the row */
        public readonly array $contextMenu = [],
    ) {
        if ($id === '') {
            throw new InvalidArgumentException('A tree node needs an id.');
        }
    }

    public function branch(): bool
    {
        return $this->children !== null;
    }

    /** @return list<self> */
    public function children(): array
    {
        if ($this->loaded !== null) {
            return $this->loaded;
        }
        $children = $this->children instanceof Closure ? ($this->children)() : ($this->children ?? []);
        foreach ($children as $child) {
            if (!$child instanceof self) {
                throw new InvalidArgumentException("The children of tree node {$this->id} must be TreeNode instances.");
            }
        }

        return $this->loaded = array_values($children);
    }
}
