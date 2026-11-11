<?php

declare(strict_types=1);

namespace App\Screens\Workbench;

use App\Domain\FileKind;
use App\Services\Workspace;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\KeyEvent;
use Pam\Desktop\Shortcut;

final class WorkbenchScreen extends Component
{
    public Workspace $workspace;

    /** @var list<string> */
    public array $menus = ['File', 'Edit', 'Selection', 'View', 'Go', 'Run', 'Terminal', 'Help'];

    /** @var list<array{icon: string, label: string}> */
    public array $activities = [
        ['icon' => 'files', 'label' => 'Explorer'], ['icon' => 'search', 'label' => 'Search'],
        ['icon' => 'git-branch', 'label' => 'Source Control'], ['icon' => 'play', 'label' => 'Run and Debug'],
        ['icon' => 'blocks', 'label' => 'Extensions'],
    ];

    /** @var list<string> */
    public array $panels = ['PROBLEMS', 'OUTPUT', 'DEBUG CONSOLE', 'TERMINAL', 'PORTS'];

    #[State]
    public int $revision = 0;

    #[State]
    public string $activity = 'Explorer';

    #[State]
    public bool $terminal = true;

    #[State]
    public bool $quickOpen = false;

    #[State(hotReload: false)]
    public string $query = '';

    #[State]
    public int $line = 1;

    /** @var list<string> */
    private array $shortcuts = [];

    public function mount(): void
    {
        $this->workspace = new Workspace(getcwd() ?: '.');
        $this->workspace->open('src/Components/Editor.pam.php');
        $this->workspace->open('src/Services/Highlighter.php');
        $this->workspace->open('src/app.css');
        $this->workspace->active = 'src/Components/Editor.pam.php';
        $this->shortcuts = [
            Shortcut::register('CmdOrCtrl+P', $this->showQuickOpen(...)),
            Shortcut::register('CmdOrCtrl+`', $this->toggleTerminal(...)),
            Shortcut::register('CmdOrCtrl+B', fn () => $this->choose($this->activity === '' ? 'Explorer' : '')),
            Shortcut::register('CmdOrCtrl+W', fn () => $this->workspace->active === null ? null : $this->closeTab($this->workspace->active)),
        ];
    }

    public function unmount(): void
    {
        foreach ($this->shortcuts as $id) {
            Shortcut::unregister($id);
        }
    }

    public function choose(string $activity): void
    {
        $this->activity = $this->activity === $activity ? '' : $activity;
    }

    public function toggle(string $path): void
    {
        $this->workspace->toggle($path);
        $this->revision++;
    }

    public function collapse(): void
    {
        $this->workspace->collapseAll();
        $this->revision++;
    }

    public function openFile(string $path): void
    {
        $this->workspace->open($path);
        $this->line = 1;
        $this->quickOpen = false;
        $this->query = '';
        $this->revision++;
    }

    public function select(string $path): void
    {
        $this->workspace->active = $path;
        $this->line = 1;
        $this->revision++;
    }

    public function closeTab(string $path): void
    {
        $this->workspace->close($path);
        $this->revision++;
    }

    public function cursor(int $line): void
    {
        $this->line = $line;
    }

    public function toggleTerminal(): void
    {
        $this->terminal = !$this->terminal;
    }

    public function showQuickOpen(): void
    {
        $this->quickOpen = true;
        $this->query = '';
    }

    public function hideQuickOpen(): void
    {
        $this->quickOpen = false;
    }

    public function quickKey(KeyEvent $event): void
    {
        if ($event->key === 'Escape') {
            $this->quickOpen = false;
        }
    }

    public function pickFirst(): void
    {
        $first = $this->matches()[0] ?? null;
        if ($first !== null) {
            $this->openFile($first);
        }
    }

    /** @return list<string> files whose path holds the query's letters in order (VS Code's fuzzy match) */
    public function matches(): array
    {
        $letters = preg_split('//u', strtolower(str_replace(' ', '', $this->query)), flags: PREG_SPLIT_NO_EMPTY) ?: [];
        $pattern = '/'.implode('.*?', array_map(static fn (string $letter): string => preg_quote($letter, '/'), $letters)).'/i';

        return array_slice(array_values(array_filter(
            $this->workspace->files(),
            static fn (string $path): bool => $letters === [] || preg_match($pattern, basename($path)) === 1,
        )), 0, 10);
    }

    public function fileName(string $path): string
    {
        return basename($path);
    }

    public function folderOf(string $path): string
    {
        $folder = dirname($path);

        return $folder === '.' ? '' : $folder;
    }

    public function icon(string $path): string
    {
        return FileKind::icon(basename($path));
    }

    public function colour(string $path): string
    {
        return FileKind::colour(basename($path));
    }

    public function language(): string
    {
        return $this->workspace->active === null ? '' : FileKind::language(basename($this->workspace->active));
    }
}
?>

<template>
    <Column class="window">
        <Row class="titlebar">
            <Row class="menus">
                <Icon name="code-xml" font="Lucide" size="16" class="logo" />
                <Text p-for="$menu in $menus" p-key="$menu" class="menu">{{ $menu }}</Text>
            </Row>
            <Row class="center">
                <Icon name="arrow-left" font="Lucide" size="16" class="dim" />
                <Icon name="arrow-right" font="Lucide" size="16" class="faint" />
                <Pressable class="command-center" cursor="pointer" @press="showQuickOpen" tooltip="Search files by name (Ctrl+P)">
                    <Icon name="search" font="Lucide" size="14" class="dim" />
                    <Text class="command-text">{{ strtolower($workspace->name()) }}</Text>
                </Pressable>
            </Row>
            <Row class="layout">
                <Icon name="panel-left" font="Lucide" size="16" class="dim" />
                <Pressable cursor="pointer" @press="toggleTerminal" tooltip="Toggle Panel (Ctrl+`)">
                    <Icon name="panel-bottom" font="Lucide" size="16" :class="$terminal ? 'text' : 'dim'" />
                </Pressable>
                <Icon name="panel-right" font="Lucide" size="16" class="dim" />
                <Icon name="layout-panel-left" font="Lucide" size="16" class="dim" />
            </Row>
        </Row>

        <Row class="body">
            <Column class="activitybar">
                <Pressable p-for="$item in $activities" p-key="$item['label']"
                           :class="$activity === $item['label'] ? 'activity activity-on' : 'activity'" cursor="pointer"
                           on:press="choose($item['label'])" :tooltip="$item['label']">
                    <Icon :name="$item['icon']" font="Lucide" size="24" :class="$activity === $item['label'] ? 'activity-icon on' : 'activity-icon'" />
                </Pressable>
                <View class="spacer" />
                <View class="activity"><Icon name="circle-user-round" font="Lucide" size="24" class="activity-icon" /></View>
                <View class="activity"><Icon name="settings" font="Lucide" size="24" class="activity-icon" /></View>
            </Column>

            <Explorer p-if="$activity === 'Explorer'" :workspace="$workspace" :revision="$revision"
                      @toggle="toggle" @open="openFile" @collapse="collapse" />
            <Column p-else-if="$activity !== ''" class="other">
                <Text class="other-title">{{ strtoupper($activity) }}</Text>
                <Text class="other-text">This view is not part of the clone.</Text>
            </Column>

            <Column class="main">
                <Editor :workspace="$workspace" :revision="$revision" @select="select" @close="closeTab" @cursor="cursor" />
                <Column p-if="$terminal" class="panel">
                    <Row class="panel-tabs">
                        <Column p-for="$panel in $panels" p-key="$panel" :class="$panel === 'TERMINAL' ? 'panel-tab panel-tab-on' : 'panel-tab'">
                            <Text :class="$panel === 'TERMINAL' ? 'panel-text panel-text-on' : 'panel-text'">{{ $panel }}</Text>
                        </Column>
                        <View class="spacer" />
                        <Icon name="plus" font="Lucide" size="16" class="dim" />
                        <Icon name="trash-2" font="Lucide" size="16" class="dim" />
                        <Pressable cursor="pointer" @press="toggleTerminal" tooltip="Hide Panel">
                            <Icon name="x" font="Lucide" size="16" class="dim" />
                        </Pressable>
                    </Row>
                    <Column class="terminal">
                        <Text class="term"><Span class="prompt">david@notebook</Span><Span class="term-dim">:</Span><Span class="path">~/pam-desktop/examples/vscode</Span><Span class="term-dim">$ </Span>pam desktop check</Text>
                        <Text class="term">pam-desktop check: 3 components in 1 root, no diagnostics.</Text>
                        <Text class="term"><Span class="prompt">david@notebook</Span><Span class="term-dim">:</Span><Span class="path">~/pam-desktop/examples/vscode</Span><Span class="term-dim">$ </Span>pam composer test</Text>
                        <Text class="term">VS Code example tests passed.</Text>
                        <Text class="term"><Span class="prompt">david@notebook</Span><Span class="term-dim">:</Span><Span class="path">~/pam-desktop/examples/vscode</Span><Span class="term-dim">$ </Span><Span class="caret"> </Span></Text>
                    </Column>
                </Column>
            </Column>
        </Row>

        <Row class="statusbar">
            <Row class="remote"><Icon name="chevrons-left-right" font="Lucide" size="14" class="white" /></Row>
            <Row class="status-item">
                <Icon name="git-branch" font="Lucide" size="14" class="status-icon" />
                <Text class="status-text">main*</Text>
            </Row>
            <Row class="status-item">
                <Icon name="circle-x" font="Lucide" size="14" class="status-icon" />
                <Text class="status-text">0</Text>
                <Icon name="triangle-alert" font="Lucide" size="14" class="status-icon" />
                <Text class="status-text">0</Text>
            </Row>
            <View class="spacer" />
            <Text p-if="$workspace->active !== null" class="status-text item">Ln {{ $line }}, Col 1</Text>
            <Text class="status-text item">Spaces: 4</Text>
            <Text class="status-text item">UTF-8</Text>
            <Text class="status-text item">LF</Text>
            <Row p-if="$workspace->active !== null" class="status-item">
                <Icon name="braces" font="Lucide" size="14" class="status-icon" />
                <Text class="status-text">{{ language() }}</Text>
            </Row>
            <Icon name="bell" font="Lucide" size="14" class="status-icon item" />
        </Row>

        <Column p-if="$quickOpen" class="quick-open" @clickOutside="hideQuickOpen" captureKeys="Escape" on:keyDown="quickKey">
            <Input class="quick-field" model="query" sync="immediate" autoFocus="true" placeholder="Search files by name"
                   placeholderColor="#989898" @submit="pickFirst" />
            <Pressable p-for="$path in matches()" p-key="$path" :class="$pathIndex === 0 ? 'quick-row quick-row-on' : 'quick-row'"
                       cursor="pointer" on:press="openFile($path)">
                <Icon :name="icon($path)" font="Lucide" size="15" :color="colour($path)" />
                <Text class="quick-name">{{ fileName($path) }}</Text>
                <Text class="quick-folder" numberOfLines="1">{{ folderOf($path) }}</Text>
            </Pressable>
        </Column>
    </Column>
</template>

<style scoped>
    .window { flex: 1; background-color: var(--editor); }
    .titlebar { height: 35px; padding: 0 8px; align-items: center; justify-content: space-between; background-color: var(--sidebar);
                border-bottom-width: 1px; border-color: var(--border); }
    .menus { width: 440px; gap: 2px; align-items: center; }
    .logo { color: var(--focus); margin: 0 8px 0 4px; }
    .menu { padding: 0 6px; font-size: 13px; color: var(--text); }
    .center { flex: 1; max-width: 520px; gap: 8px; align-items: center; justify-content: center; }
    .command-center { flex: 1; flex-direction: row; height: 24px; gap: 6px; align-items: center; justify-content: center;
                      border-radius: 6px; border-width: 1px; border-color: #FFFFFF1F; background-color: #FFFFFF0D; }
    .command-center:hover { background-color: #FFFFFF1A; }
    .command-text { font-size: 12px; color: var(--text); }
    .layout { width: 440px; gap: 12px; align-items: center; justify-content: flex-end; padding-right: 8px; }
    .body { flex: 1; min-height: 0; }
    .activitybar { width: 48px; background-color: var(--sidebar); border-right-width: 1px; border-color: var(--border); }
    .activity { width: 48px; height: 48px; align-items: center; justify-content: center; border-left-width: 2px; border-color: #00000000; }
    .activity-on { border-color: var(--focus); }
    .activity-icon { color: #868686; }
    .activity:hover .activity-icon { color: #D7D7D7; }
    .on { color: #D7D7D7; }
    .other { width: 260px; padding: 10px 20px; gap: 12px; background-color: var(--sidebar); border-right-width: 1px; border-color: var(--border); }
    .other-title { font-size: 11px; color: var(--text); }
    .other-text { font-size: 13px; color: var(--dim); }
    .main { flex: 1; min-width: 0; }
    .panel { height: 190px; border-top-width: 1px; border-color: var(--border); background-color: var(--sidebar); }
    .panel-tabs { height: 35px; padding: 0 8px; gap: 12px; align-items: center; }
    .panel-tab { height: 35px; padding: 0 4px; justify-content: center; border-bottom-width: 1px; border-color: #00000000; }
    .panel-tab-on { border-color: var(--focus); }
    .panel-text { font-size: 11px; color: var(--dim); }
    .panel-text-on { color: #E7E7E7; }
    .terminal { padding: 4px 20px; gap: 2px; }
    .term { font-family: "JetBrains Mono", monospace; font-size: 13px; line-height: 19px; color: #CCCCCC; }
    .prompt { color: #23D18B; }
    .path { color: #3B8EEA; }
    .term-dim { color: #CCCCCC; }
    .caret { background-color: #CCCCCC; }
    .statusbar { height: 22px; align-items: center; gap: 0; background-color: var(--sidebar); border-top-width: 1px; border-color: var(--border); }
    .remote { width: 34px; height: 22px; align-items: center; justify-content: center; background-color: var(--focus); }
    .status-item { height: 22px; gap: 4px; padding: 0 8px; align-items: center; }
    .status-item:hover { background-color: #FFFFFF1F; }
    .item { padding: 0 8px; }
    .status-icon { color: var(--text); }
    .status-text { font-size: 12px; color: var(--text); }
    .quick-open { position: absolute; top: 6px; left: 340px; right: 340px; padding: 6px 6px 8px 6px; gap: 2px; border-radius: 8px;
                  background-color: #222222; border-width: 1px; border-color: #454545; box-shadow: 0 8px 16px #00000099; z-index: 10; }
    .quick-field { height: 26px; padding: 0 8px; border-radius: 3px; border-width: 1px; border-color: var(--focus);
                   background-color: #313131; font-size: 13px; color: var(--text); margin-bottom: 4px; }
    .quick-row { flex-direction: row; height: 22px; gap: 6px; padding: 0 8px; align-items: center; border-radius: 3px; }
    .quick-row:hover { background-color: #2A2D2E; }
    .quick-row-on { background-color: #04395E; }
    .quick-name { font-size: 13px; color: var(--text); }
    .quick-folder { flex: 1; font-size: 12px; color: var(--dim); }
    .spacer { flex: 1; }
    .text { color: var(--text); }
    .white { color: #FFFFFF; }
    .dim { color: var(--dim); }
    .faint { color: #5A5A5A; }
</style>
