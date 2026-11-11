<?php

declare(strict_types=1);

namespace App\Components;

use App\Domain\Entry;
use App\Domain\FileKind;
use App\Services\Workspace;
use Pam\Desktop\Component;

final class Explorer extends Component
{
    public function __construct(public Workspace $workspace, public int $revision = 0)
    {
    }

    public function press(string $path, bool $folder): void
    {
        $this->emit($folder ? 'toggle' : 'open', $path);
    }

    public function collapse(): void
    {
        $this->emit('collapse');
    }

    public function rowClass(Entry $entry): string
    {
        return $entry->path === $this->workspace->active ? 'row row-on' : 'row';
    }

    public function icon(Entry $entry): string
    {
        return FileKind::icon($entry->name);
    }

    public function colour(Entry $entry): string
    {
        return FileKind::colour($entry->name);
    }

    /** @return list<int> one indent guide per level */
    public function guides(Entry $entry): array
    {
        return $entry->depth === 0 ? [] : range(1, $entry->depth);
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
        <VirtualizedList class="tree" estimatedRowHeight="22">
            <Pressable p-for="$entry in $workspace->entries()" p-key="$entry->path" :class="rowClass($entry)"
                       on:press="press($entry->path, $entry->folder)">
                <View p-for="$level in guides($entry)" p-key="$level" class="guide" />
                <View class="twistie">
                    <Icon p-if="$entry->folder" :name="$entry->open ? 'chevron-down' : 'chevron-right'" font="Lucide" size="16" class="text" />
                </View>
                <Icon p-if="!$entry->folder" :name="icon($entry)" font="Lucide" size="15" :color="colour($entry)" />
                <Text :class="$entry->folder ? 'name folder' : 'name'" numberOfLines="1">{{ $entry->name }}</Text>
            </Pressable>
        </VirtualizedList>
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
    .explorer { width: 260px; background-color: var(--sidebar); border-right-width: 1px; border-color: var(--border); }
    .title { height: 35px; padding: 0 12px 0 20px; align-items: center; justify-content: space-between; }
    .title-text { font-size: 11px; color: var(--text); }
    .section { height: 22px; padding: 0 8px 0 2px; gap: 2px; align-items: center; }
    .section-text { font-size: 11px; font-weight: 600; color: var(--text); }
    .section-tools { gap: 6px; opacity: 0; }
    .explorer:hover .section-tools { opacity: 1; }
    .tool { width: 18px; height: 18px; align-items: center; justify-content: center; }
    .collapsed { border-top-width: 1px; border-color: var(--border); }
    .spacer { flex: 1; }
    .tree { flex: 1; scrollbar-width: thin; }
    .row { flex-direction: row; height: 22px; padding-left: 8px; gap: 4px; align-items: center; border-width: 1px; border-color: #00000000; }
    .row:hover { background-color: #2A2D2E; }
    .row-on { background-color: #37373D; border-color: var(--focus); }
    .row-on:hover { background-color: #37373D; }
    .guide { width: 8px; height: 22px; margin-right: 4px; border-left-width: 1px; border-color: #585858; opacity: 0; }
    .explorer:hover .guide { opacity: 0.6; }
    .twistie { width: 16px; height: 16px; align-items: center; justify-content: center; }
    .name { flex: 1; font-size: 13px; color: var(--text); }
    .folder { color: var(--text); }
    .text { color: var(--text); }
    .dim { color: var(--dim); }
</style>
