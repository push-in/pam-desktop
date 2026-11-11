<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

/** A visible row of a `TreeView`: a node at its depth. */
final readonly class TreeRow
{
    public function __construct(
        public TreeNode $node,
        public int $depth,
        public bool $open,
        public ?string $parent,
    ) {
    }
}
