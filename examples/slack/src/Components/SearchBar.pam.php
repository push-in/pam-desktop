<?php

declare(strict_types=1);

namespace App\Components;

use App\Domain\SearchHit;
use App\Services\Workspace;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\KeyEvent;

final class SearchBar extends Component
{
    #[State(hotReload: false)]
    public string $query = '';

    #[State]
    public bool $open = false;

    public function __construct(public Workspace $workspace)
    {
    }

    /** @return list<SearchHit> */
    public function hits(): array
    {
        return $this->workspace->search($this->query);
    }

    public function show(): void
    {
        $this->open = true;
    }

    public function close(): void
    {
        $this->open = false;
    }

    public function key(KeyEvent $event): void
    {
        if ($event->key === 'Escape') {
            $this->query = '';
            $this->open = false;
        }
    }

    public function pick(string $conversationId): void
    {
        $this->query = '';
        $this->open = false;
        $this->emit('open', $conversationId);
    }

    /** The first hit opens on Enter, as in Slack's quick switcher. */
    public function submit(): void
    {
        $first = $this->hits()[0] ?? null;
        if ($first !== null) {
            $this->pick($first->conversationId);
        }
    }

    public function placeholder(): string
    {
        return 'Search '.Workspace::NAME;
    }
}
?>

<template>
    <Column class="anchor" @clickOutside="close" captureKeys="Escape" on:keyDown="key">
        <Row :class="$open ? 'bar bar-open' : 'bar bar-closed'">
            <Icon name="search" font="Lucide" size="14" :class="$open ? 'icon icon-open' : 'icon'" />
            <Input :class="$open ? 'field field-open' : 'field'" model="query" sync="immediate" :placeholder="placeholder()"
                   :placeholderColor="$open ? '#868686' : '#FFFFFFCC'" @focus="show" @submit="submit" />
        </Row>
        <Column p-if="$open" class="panel">
            <Text p-if="hits() === [] && mb_strlen(trim($query)) < 2" class="hint">Search for messages, channels and people</Text>
            <Text p-else-if="hits() === []" class="hint">No results for “{{ $query }}”</Text>
            <Pressable p-for="$hit in hits()" p-key="$hit->key" class="hit" cursor="pointer"
                       on:press="pick($hit->conversationId)">
                <Column class="hit-media">
                    <Image p-if="$hit->avatar !== null" :source="$hit->avatar" class="hit-photo" />
                    <Icon p-else :name="$hit->icon" font="Lucide" size="16" class="hit-icon" />
                </Column>
                <Column class="hit-body">
                    <Row class="hit-head">
                        <Text class="hit-label">{{ $hit->label }}</Text>
                        <Text class="hit-detail" numberOfLines="1">{{ $hit->detail }}</Text>
                    </Row>
                    <Text p-if="$hit->isMessage()" class="hit-text" numberOfLines="2">
                        <Span p-for="($run, $i) in $hit->runs" p-key="$i" :class="'run-' . $run->style">{{ $run->text }}</Span>
                    </Text>
                </Column>
            </Pressable>
        </Column>
    </Column>
</template>

<style scoped>
    .anchor { flex: 1; max-width: 720px; z-index: 10; }
    .bar { height: 26px; gap: 8px; padding: 0 10px; align-items: center; border-radius: 6px; background-color: #FFFFFF33; }
    .bar-closed:hover { background-color: #FFFFFF40; }
    .bar-open { background-color: var(--surface); border-bottom-left-radius: 0; border-bottom-right-radius: 0; }
    .icon { color: var(--chrome-text); }
    .icon-open { color: var(--muted); }
    .field { flex: 1; height: 26px; padding: 0; border-width: 0; font-size: 13px; color: var(--chrome-text);
             background-color: #00000000; }
    .field-open { color: var(--text); }
    .panel { position: absolute; top: 26px; left: 0; right: 0; padding: 8px 0; background-color: var(--surface);
             border-bottom-left-radius: 8px; border-bottom-right-radius: 8px; box-shadow: 0 4px 12px #0000002E; }
    .hint { padding: 8px 16px; font-size: 13px; color: var(--muted); }
    .hit { flex-direction: row; gap: 10px; padding: 6px 16px; align-items: flex-start; }
    .hit:hover { background-color: var(--hover); }
    .hit-media { width: 24px; height: 24px; align-items: center; justify-content: center; }
    .hit-photo { width: 24px; height: 24px; border-radius: 4px; }
    .hit-icon { color: var(--muted); }
    .hit-body { flex: 1; gap: 2px; }
    .hit-head { gap: 8px; align-items: baseline; }
    .hit-label { font-size: 14px; font-weight: 700; }
    .hit-detail { flex: 1; font-size: 12px; color: var(--muted); }
    .hit-text { font-size: 13px; line-height: 18px; color: var(--text); }
    .run-plain { font-weight: 400; }
    .run-match { font-weight: 700; background-color: #F2C74466; }
</style>
