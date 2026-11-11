<?php

declare(strict_types=1);

namespace App\Components;

use App\Services\Workspace;
use Pam\Desktop\Component;

final class Explorer extends Component
{
    public function __construct(public Workspace $workspace, public int $revision = 0, public int $collapsed = 0)
    {
    }

    public function open(string $path): void
    {
        $this->emit('open', $path);
    }

    public function collapse(): void
    {
        $this->emit('collapse');
    }
}
?>

<template>
    <Column class="explorer">
        <Row class="title">
            <Text class="title-text">EXPLORER</Text>
            <Icon name="ellipsis" font="Lucide" size="16" class="dim" />
        </Row>
        <Row class="section">
            <Icon name="chevron-down" font="Lucide" size="16" class="text" />
            <Text class="section-text">{{ $workspace->name() }}</Text>
            <View class="spacer" />
            <Row class="section-tools">
                <Icon name="file-plus" font="Lucide" size="16" class="dim" tooltip="New File..." />
                <Icon name="folder-plus" font="Lucide" size="16" class="dim" tooltip="New Folder..." />
                <Icon name="refresh-cw" font="Lucide" size="16" class="dim" tooltip="Refresh Explorer" />
                <Pressable class="tool" cursor="pointer" @press="collapse" tooltip="Collapse Folders in Explorer">
                    <Icon name="copy-minus" font="Lucide" size="16" class="dim" />
                </Pressable>
            </Row>
        </Row>
        <TreeView :nodes="$workspace->tree()" :selected="$workspace->active" :expanded="$workspace->expanded()"
                  :collapseRequest="$collapsed" label="Files Explorer" on:activate="open" />
        <Row class="section collapsed">
            <Icon name="chevron-right" font="Lucide" size="16" class="text" />
            <Text class="section-text">OUTLINE</Text>
        </Row>
        <Row class="section collapsed">
            <Icon name="chevron-right" font="Lucide" size="16" class="text" />
            <Text class="section-text">TIMELINE</Text>
        </Row>
    </Column>
</template>

<style scoped>
    .explorer { flex: 1; background-color: var(--sidebar); }
    .title { height: 35px; padding: 0 12px 0 20px; align-items: center; justify-content: space-between; }
    .title-text { font-size: 11px; color: var(--text); }
    .section { height: 22px; padding: 0 8px 0 2px; gap: 2px; align-items: center; }
    .section-text { font-size: 11px; font-weight: 600; color: var(--text); }
    .section-tools { gap: 6px; opacity: 0; transition: opacity 150ms; }
    .explorer:hover .section-tools { opacity: 1; }
    .tool { width: 18px; height: 18px; align-items: center; justify-content: center; }
    .collapsed { border-top-width: 1px; border-color: var(--border); }
    .spacer { flex: 1; }
    .text { color: var(--text); }
    .dim { color: var(--dim); }
</style>
