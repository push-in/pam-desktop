<?php

declare(strict_types=1);

namespace App\Components;

use App\Domain\FileKind;
use App\Services\Workspace;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Code\Highlighter;
use Pam\Desktop\Component;

final class Editor extends Component
{
    /** The line with the cursor, 1-based. */
    #[State]
    public int $cursor = 1;

    public function __construct(public Workspace $workspace, public int $revision = 0)
    {
    }

    /** @return list<list<array{0: string, 1: string}>> */
    public function lines(): array
    {
        return $this->workspace->active === null ? [] : $this->workspace->lines($this->workspace->active);
    }

    /** @return list<string> */
    public function crumbs(): array
    {
        return $this->workspace->active === null ? [] : explode('/', $this->workspace->active);
    }

    public function select(string $path): void
    {
        $this->emit('select', $path);
    }

    public function close(string $path): void
    {
        $this->emit('close', $path);
    }

    public function language(): ?string
    {
        return $this->workspace->active === null ? null : Highlighter::language($this->workspace->active);
    }

    /** @param array{line: int, column: int, cursors: int} $at */
    public function moved(array $at): void
    {
        $this->cursor = $at['line'];
        $this->emit('cursor', $at);
    }

    public function edited(string $text): void
    {
        if ($this->workspace->active !== null) {
            $this->workspace->edit($this->workspace->active, $text);
            $this->emit('edit', $this->workspace->active);
        }
    }

    public function tabClass(string $path): string
    {
        return $path === $this->workspace->active ? 'tab tab-on' : 'tab';
    }

    public function fileName(string $path): string
    {
        return basename($path);
    }

    public function icon(string $path): string
    {
        return FileKind::icon(basename($path));
    }

    public function colour(string $path): string
    {
        return FileKind::colour(basename($path));
    }

    /**
     * The minimap: each line as one bar per coloured run, two pixels tall,
     * as wide as its characters.
     *
     * @return list<list<array{0: int, 1: string}>> width and colour per run
     */
    public function minimap(): array
    {
        $rows = [];
        foreach (array_slice($this->lines(), 0, 220) as $line) {
            $row = [];
            foreach ($line as [$text, $colour]) {
                $row[] = [max(1, (int) round(mb_strlen($text) * 1.1)), $colour];
            }
            $rows[] = $row;
        }

        return $rows;
    }
}
?>

<template>
    <Column class="editor">
        <Row class="tabs">
            <Pressable p-for="$path in $workspace->tabs" p-key="$path" :class="tabClass($path)" cursor="pointer"
                       on:press="select($path)" :tooltip="$path">
                <Icon :name="icon($path)" font="Lucide" size="15" :color="colour($path)" />
                <Text :class="$path === $workspace->active ? 'tab-name tab-name-on' : 'tab-name'">{{ fileName($path) }}</Text>
                <Pressable :class="$path === $workspace->active ? 'tab-close shown' : 'tab-close'" cursor="pointer"
                           on:press="close($path)" :tooltip="$workspace->modified($path) ? 'Unsaved changes' : 'Close'">
                    <Icon p-if="$workspace->modified($path)" name="circle" font="Lucide" size="10" class="text dot" />
                    <Icon p-else name="x" font="Lucide" size="15" class="text" />
                </Pressable>
            </Pressable>
            <View class="tabs-rest" />
        </Row>

        <Column p-if="$workspace->active !== null" class="document">
            <Row class="crumbs">
                <Row p-for="$crumb in crumbs()" p-key="$crumbIndex" class="crumb">
                    <Icon p-if="$crumbIndex === count(crumbs()) - 1" :name="icon($crumb)" font="Lucide" size="14" :color="colour($crumb)" />
                    <Text class="crumb-text">{{ $crumb }}</Text>
                    <Icon p-if="$crumbIndex < count(crumbs()) - 1" name="chevron-right" font="Lucide" size="14" class="dim" />
                </Row>
            </Row>
            <Row class="surface">
                <CodeEditor :key="'code-' . $workspace->active" :text="$workspace->source($workspace->active)"
                            :language="language()" fontFamily="JetBrains Mono"
                            :label="$workspace->active" autoFocus="true" on:change="edited" on:cursor="moved" />
                <Column class="minimap">
                    <Row p-for="$row in minimap()" p-key="$rowIndex" class="map-row">
                        <View p-for="$bar in $row" p-key="$barIndex" class="map-bar" :width="$bar[0]" :backgroundColor="$bar[1]" />
                    </Row>
                </Column>
            </Row>
        </Column>

        <Column p-else class="empty">
            <Icon name="code-xml" font="Lucide" size="120" class="watermark" />
            <Row class="hint"><Text class="hint-text">Show All Commands</Text><Text class="key">Ctrl + Shift + P</Text></Row>
            <Row class="hint"><Text class="hint-text">Go to File</Text><Text class="key">Ctrl + P</Text></Row>
            <Row class="hint"><Text class="hint-text">Toggle Terminal</Text><Text class="key">Ctrl + `</Text></Row>
        </Column>
    </Column>
</template>

<style scoped>
    .editor { flex: 1; background-color: var(--editor); }
    .tabs { height: 35px; background-color: var(--sidebar); }
    .tab { flex-direction: row; height: 35px; gap: 6px; padding: 0 6px 0 12px; align-items: center;
           border-right-width: 1px; border-bottom-width: 1px; border-top-width: 1px; border-color: var(--border);
           border-top-color: #00000000; background-color: var(--sidebar); }
    .tab:hover { background-color: var(--editor); }
    .tab-on { background-color: var(--editor); border-bottom-color: var(--editor); border-top-color: var(--focus); }
    .tab-name { font-size: 13px; color: var(--dim); }
    .tab-name-on { color: #FFFFFF; }
    .tab-close { width: 20px; height: 20px; border-radius: 4px; align-items: center; justify-content: center; opacity: 0; }
    .tab:hover .tab-close { opacity: 1; }
    .tab-close:hover { background-color: #FFFFFF1F; }
    .shown { opacity: 1; }
    .tabs-rest { flex: 1; border-bottom-width: 1px; border-color: var(--border); }
    .document { flex: 1; }
    .crumbs { height: 22px; padding: 0 16px; gap: 2px; align-items: center; }
    .crumb { gap: 4px; align-items: center; }
    .crumb-text { font-size: 13px; color: var(--dim); }
    .surface { flex: 1; }
    .dot { opacity: 1; }
    .tab:hover .dot { opacity: 0; }
    .minimap { width: 90px; padding: 0 10px; gap: 0; overflow: hidden; border-left-width: 1px; border-color: #FFFFFF08; }
    .map-row { height: 2px; gap: 0; }
    .map-bar { height: 2px; opacity: 0.55; }
    .empty { flex: 1; gap: 12px; align-items: center; justify-content: center; }
    .watermark { color: #FFFFFF0D; margin-bottom: 24px; }
    .hint { gap: 16px; align-items: center; }
    .hint-text { width: 160px; font-size: 13px; color: var(--dim); text-align: right; }
    .key { width: 160px; font-size: 12px; color: var(--dim); }
    .text { color: var(--text); }
    .dim { color: var(--dim); }
</style>
