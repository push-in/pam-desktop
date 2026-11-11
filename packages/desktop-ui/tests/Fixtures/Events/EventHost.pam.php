<?php

declare(strict_types=1);

namespace Fixtures\Events;

use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\DesktopUi\TreeNode;

/** Listens to DesktopUI component events with `on:` (some share a native event's name). */
final class EventHost extends Component
{
    #[State] public float $size = 200.0;
    /** @var list<mixed> */
    public array $events = [];

    /** @return list<TreeNode> */
    public function nodes(): array
    {
        return [new TreeNode('src', 'src', [])];
    }

    public function resized(float $size): void
    {
        $this->events[] = ['resize', $size];
        $this->size = $size;
    }

    public function closed(): void
    {
        $this->events[] = ['close'];
    }

    /** @param array{id: string, open: bool} $change */
    public function toggled(array $change): void
    {
        $this->events[] = ['toggle', $change['id'], $change['open']];
    }
}
?>
<template>
    <Column>
        <SplitView :size="$size" on:resize="resized"><Text>A</Text><Text>B</Text></SplitView>
        <Popover :open="true" anchor="nowhere" on:close="closed"><Text>P</Text></Popover>
        <TreeView :nodes="nodes()" on:toggle="toggled" />
    </Column>
</template>
