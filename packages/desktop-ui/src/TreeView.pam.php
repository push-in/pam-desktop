<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\KeyEvent;
use Pam\Desktop\LayoutEvent;
use Pam\Desktop\System\ViewRef;

/**
 * A virtualized tree: a file explorer, a channel list with sections, an
 * outline. Only the rows on screen are built, so a tree of thousands of
 * nodes costs what fits in its height.
 *
 * Pressing a branch opens or closes it; pressing a leaf selects it and
 * emits `activate`. With a row focused: Up/Down move, Right opens a branch
 * (or goes to its first child), Left closes it (or goes to the parent),
 * Enter and Space act like a press, Home/End jump to the ends. Every
 * selection change emits `select` with the node id; opening and closing
 * emit `toggle` with `['id' => …, 'open' => …]`.
 *
 * ```php
 * $this->tree = [
 *     new TreeNode('src', 'src', [new TreeNode('src/App.php', 'App.php', icon: 'file-code')], icon: 'folder'),
 *     new TreeNode('README.md', 'README.md', icon: 'file-text'),
 * ];
 * ```
 *
 * ```html
 * <TreeView :nodes="$tree" :selected="$active" :expanded="['src']" on:activate="open" />
 * ```
 */
final class TreeView extends Component
{
    /** @var array<string, true> */
    #[State] public array $open = [];
    #[State] public int $scrollRevision = 0;
    #[State] public float $scrollTarget = 0.0;
    private float $scrollOffset = 0.0;
    private float $viewport = 0.0;
    public readonly string $key;

    /**
     * @param list<TreeNode> $nodes
     * @param list<string> $expanded branches open at first
     */
    public function __construct(
        #[State] public array $nodes,
        #[State] public ?string $selected = null,
        array $expanded = [],
        public int $rowHeight = 22,
        public int $indent = 8,
        public string $iconFont = 'Lucide',
        public int $iconSize = 16,
        public int $fontSize = 13,
        public bool $guides = true,
        public int|string $color = '#CCCCCC',
        public int|string $muted = '#8C8C8C',
        public int|string $guideColor = '#585858',
        public ?string $label = null,
    ) {
        $this->open = array_fill_keys($expanded, true);
        $this->key = 'tree-'.spl_object_id($this);
    }

    /** @return list<TreeRow> the visible rows, depth first */
    public function rows(): array
    {
        $rows = [];
        $walk = function (array $nodes, int $depth, ?string $parent) use (&$walk, &$rows): void {
            foreach ($nodes as $node) {
                $open = $node->branch() && isset($this->open[$node->id]);
                $rows[] = new TreeRow($node, $depth, $open, $parent);
                if ($open) {
                    $walk($node->children(), $depth + 1, $node->id);
                }
            }
        };
        $walk($this->nodes, 0, null);

        return $rows;
    }

    /** @return list<int> one indent guide per level above the row */
    public function levels(TreeRow $row): array
    {
        return $row->depth === 0 ? [] : range(1, $row->depth);
    }

    public function rowRef(int $index): string
    {
        return $this->key.'-'.$index;
    }

    public function press(string $id): void
    {
        $row = $this->find($id);
        if ($row === null) {
            return;
        }
        $this->select($id);
        if ($row->node->branch()) {
            $this->toggle($id);
        } else {
            $this->emit('activate', $id);
        }
    }

    public function toggle(string $id, ?bool $open = null): void
    {
        $open ??= !isset($this->open[$id]);
        if ($open === isset($this->open[$id])) {
            return;
        }
        if ($open) {
            $this->open[$id] = true;
        } else {
            unset($this->open[$id]);
        }
        $this->emit('toggle', ['id' => $id, 'open' => $open]);
    }

    public function expand(string $id): void
    {
        $this->toggle($id, true);
    }

    public function collapseAll(): void
    {
        $this->open = [];
    }

    public function key(KeyEvent $event): void
    {
        if ($event->control || $event->alt || $event->meta) {
            return;
        }
        $rows = $this->rows();
        if ($rows === []) {
            return;
        }
        $index = $this->indexOf($rows, $this->selected) ?? -1;
        $row = $rows[$index] ?? null;
        $target = match ($event->key) {
            'ArrowDown' => min(count($rows) - 1, $index + 1),
            'ArrowUp' => max(0, $index - 1),
            'Home' => 0,
            'End' => count($rows) - 1,
            default => null,
        };
        if ($row !== null && $event->key === 'ArrowRight' && $row->node->branch()) {
            if (!$row->open) {
                $this->toggle($row->node->id, true);
                return;
            }
            $target = $index + 1 < count($rows) && $rows[$index + 1]->parent === $row->node->id ? $index + 1 : null;
        } elseif ($row !== null && $event->key === 'ArrowLeft') {
            if ($row->open) {
                $this->toggle($row->node->id, false);
                return;
            }
            $target = $row->parent === null ? null : $this->indexOf($rows, $row->parent);
        } elseif ($row !== null && ($event->key === 'Enter' || $event->key === ' ')) {
            $this->press($row->node->id);
            return;
        }
        if ($target === null || $target < 0) {
            return;
        }
        $this->select($rows[$target]->node->id);
        $this->reveal($target);
        ViewRef::of($this->rowRef($target))->focus();
    }

    public function scrolled(string $offset): void
    {
        $this->scrollOffset = (float) $offset;
    }

    public function measured(LayoutEvent $layout): void
    {
        $this->viewport = $layout->height;
    }

    public function rowClass(TreeRow $row): string
    {
        return $row->node->id === $this->selected ? 'row on' : 'row';
    }

    private function select(string $id): void
    {
        if ($this->selected === $id) {
            return;
        }
        $this->selected = $id;
        $this->emit('select', $id);
    }

    /** Scrolls just enough to show row `$index`. */
    private function reveal(int $index): void
    {
        $top = $index * $this->rowHeight;
        $bottom = $top + $this->rowHeight;
        if ($top < $this->scrollOffset) {
            $this->scrollTarget = $top;
        } elseif ($this->viewport > 0 && $bottom > $this->scrollOffset + $this->viewport) {
            $this->scrollTarget = $bottom - $this->viewport;
        } else {
            return;
        }
        $this->scrollOffset = $this->scrollTarget;
        ++$this->scrollRevision;
    }

    private function find(string $id): ?TreeRow
    {
        foreach ($this->rows() as $row) {
            if ($row->node->id === $id) {
                return $row;
            }
        }

        return null;
    }

    /** @param list<TreeRow> $rows */
    private function indexOf(array $rows, ?string $id): ?int
    {
        foreach ($rows as $index => $row) {
            if ($row->node->id === $id) {
                return $index;
            }
        }

        return null;
    }
}
?>

<template>
    <VirtualizedList class="tree" :estimatedRowHeight="$rowHeight" :scrollRequest="$scrollRevision" :scrollTargetOffset="$scrollTarget"
                     on:scroll="scrolled" on:layout="measured" accessibilityRole="list" :accessibilityLabel="$label">
        <Pressable p-for="($row, $index) in rows()" p-key="$row->node->id" :nativeRef="rowRef($index)" :class="rowClass($row)"
                   :height="$rowHeight" :tooltip="$row->node->tooltip" on:press="press($row->node->id)" on:keyDown="key"
                   captureKeys="ArrowUp,ArrowDown,ArrowLeft,ArrowRight,Home,End,Enter,Space"
                   accessibilityRole="listitem" :accessibilityLabel="$row->node->label"
                   :accessibilityExpanded="$row->node->branch() ? $row->open : null" :accessibilitySelected="$row->node->id === $selected">
            <View p-if="$guides" p-for="$level in levels($row)" p-key="$level" class="guide" :width="$indent" :height="$rowHeight" :borderColor="$guideColor" />
            <View p-if="!$guides && $row->depth > 0" :width="$row->depth * $indent" />
            <View class="twistie" :width="$iconSize" :height="$iconSize">
                <Icon p-if="$row->node->branch()" :name="$row->open ? 'chevron-down' : 'chevron-right'" :font="$iconFont" :size="$iconSize" :color="$color" />
            </View>
            <Icon p-if="$row->node->icon !== null" :name="$row->node->icon" :font="$iconFont" :size="$iconSize - 1" :color="$row->node->iconColor ?? $muted" />
            <Text class="label" :fontSize="$fontSize" :textColor="$row->node->color ?? $color" numberOfLines="1">{{ $row->node->label }}</Text>
            <Text p-if="$row->node->detail !== null" class="detail" :fontSize="$fontSize - 1" :textColor="$muted" numberOfLines="1">{{ $row->node->detail }}</Text>
            <Text p-if="$row->node->badge !== null" class="badge" :fontSize="$fontSize - 1" :textColor="$row->node->badgeColor ?? $muted">{{ $row->node->badge }}</Text>
        </Pressable>
    </VirtualizedList>
</template>

<style scoped>
    .tree { flex: 1; scrollbar-width: thin; }
    .row { flex-direction: row; padding-left: 8px; padding-right: 12px; gap: 4px; align-items: center; border-width: 1px; border-color: #00000000; }
    .row:hover { background-color: #2A2D2E; }
    .on { background-color: #37373D; border-color: #0078D4; }
    .on:hover { background-color: #37373D; }
    .guide { margin-right: 4px; border-left-width: 1px; opacity: 0; transition: opacity 120ms; }
    .tree:hover .guide { opacity: 0.6; }
    .twistie { align-items: center; justify-content: center; }
    .label { flex-shrink: 1; }
    .detail { flex-shrink: 1; margin-left: 4px; }
    .badge { margin-left: auto; padding-left: 8px; }
</style>
