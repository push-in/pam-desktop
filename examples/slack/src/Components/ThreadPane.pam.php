<?php

declare(strict_types=1);

namespace App\Components;

use App\Domain\Conversation;
use App\Domain\Entry;
use App\Services\Workspace;
use Pam\Desktop\Component;

final class ThreadPane extends Component
{
    public function __construct(
        public Workspace $workspace,
        public Conversation $conversation,
        public string $parentId,
    ) {
    }

    /** @return list<Entry> */
    public function entries(): array
    {
        return $this->workspace->thread($this->conversation->id, $this->parentId);
    }

    public function close(): void
    {
        $this->emit('close');
    }

    public function reply(string $text): void
    {
        $this->emit('reply', $text);
    }

    /** @param array{message: string, emoji: string} $reaction */
    public function react(array $reaction): void
    {
        $this->emit('react', $reaction);
    }

    public function repliesLabel(): string
    {
        $count = count($this->entries()) - 1;

        return $count === 1 ? '1 reply' : "{$count} replies";
    }
}
?>

<template>
    <Column class="pane">
        <Row class="header">
            <Row class="title">
                <Text class="heading">Thread</Text>
                <Text class="channel"># {{ $conversation->name }}</Text>
            </Row>
            <Pressable class="close" cursor="pointer" @press="close" accessibilityLabel="Close thread">
                <Icon name="x" font="Lucide" size="18" class="close-icon" />
            </Pressable>
        </Row>
        <ScrollView class="scroll" scrollbarWidth="thin">
            <Column class="list">
                <MessageRow :key="'thread-' . $parentId" :entry="entries()[0]" :workspace="$workspace" :inThread="true" @react="react" />
                <Row class="count">
                    <Text class="count-text">{{ repliesLabel() }}</Text>
                    <View class="rule" />
                </Row>
                <MessageRow p-for="$entry in array_slice(entries(), 1)" :key="'reply-' . $entry->key"
                            :entry="$entry" :workspace="$workspace" :inThread="true" @react="react" />
            </Column>
        </ScrollView>
        <Composer :key="'reply-' . $parentId" placeholder="Reply…" @send="reply" />
    </Column>
</template>

<style scoped>
    .pane { width: 400px; border-left-width: 1px; border-color: var(--faint); background-color: var(--surface); }
    .header { height: 49px; padding: 0 12px 0 16px; align-items: center; justify-content: space-between;
              border-bottom-width: 1px; border-color: var(--faint); }
    .title { gap: 8px; align-items: baseline; }
    .heading { font-size: 18px; font-weight: 900; }
    .channel { font-size: 13px; color: var(--muted); }
    .close { width: 32px; height: 32px; border-radius: 6px; align-items: center; justify-content: center; }
    .close:hover { background-color: var(--hover); }
    .close-icon { color: var(--muted); }
    .scroll { flex: 1; }
    .list { padding: 8px 0 16px 0; }
    .count { height: 32px; gap: 12px; padding: 0 20px; align-items: center; }
    .count-text { font-size: 13px; color: var(--muted); }
    .rule { flex: 1; height: 1px; background-color: var(--faint); }
</style>
