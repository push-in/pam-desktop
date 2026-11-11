<?php

declare(strict_types=1);

namespace App\Components;

use App\Domain\BoardList;
use App\Services\Board;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\DragPayload;
use Pam\Desktop\KeyEvent;

final class ListColumn extends Component
{
    #[State]
    public bool $adding = false;

    #[State(hotReload: false)]
    public string $draft = '';

    public function __construct(
        public BoardList $list,
        public Board $board,
        public ?string $dragging = null,
        public ?string $over = null,
        public int $revision = 0,
    ) {
    }

    public function compose(): void
    {
        $this->adding = true;
    }

    public function cancel(): void
    {
        $this->adding = false;
        $this->draft = '';
    }

    public function add(): void
    {
        if (trim($this->draft) !== '') {
            $this->emit('add', ['list' => $this->list->id, 'title' => $this->draft]);
        }
        $this->draft = '';
    }

    public function key(KeyEvent $event): void
    {
        if ($event->key === 'Escape') {
            $this->cancel();
        }
    }

    public function enter(DragPayload $drag): void
    {
        $this->emit('over', 'end:'.$this->list->id);
    }

    public function drop(DragPayload $drag): void
    {
        $this->emit('drop', ['card' => $drag->data, 'list' => $this->list->id]);
    }

    /** @param array{card: string, before: string} $drop */
    public function dropBefore(array $drop): void
    {
        $this->emit('drop', ['card' => $drop['card'], 'list' => $this->list->id, 'before' => $drop['before']]);
    }

    public function count(): int
    {
        return count($this->list->cardIds);
    }

    public function forward(string $event, mixed $payload = null): void
    {
        $this->emit($event, $payload);
    }
}
?>

<template>
    <DropTarget dropType="1" on:drop="drop" on:dragEnter="enter">
        <Column class="list">
            <Row class="header">
                <Text class="title" numberOfLines="1">{{ $list->title }}</Text>
                <Row class="header-tools">
                    <View class="tool"><Icon name="chevrons-left-right" font="Lucide" size="16" class="tool-icon" /></View>
                    <View class="tool"><Icon name="ellipsis" font="Lucide" size="16" class="tool-icon" /></View>
                </Row>
            </Row>
            <VirtualizedList class="cards" estimatedRowHeight="88">
                    <CardTile p-for="$card in $board->cards($list->id)" p-key="$card->id" :card="$card" :board="$board"
                              :dragging="$dragging === $card->id" :over="$over === $card->id && $dragging !== $card->id"
                              @open="forward('open', $card->id)" @dragStart="forward('dragStart', $card->id)"
                              @dragEnd="forward('dragEnd')" @over="forward('over', $card->id)"
                              @drop="dropBefore" @complete="forward('complete', $card->id)"
                              @watch="forward('watch', $card->id)" @moveTo="forward('moveTo', $event)" />
            </VirtualizedList>
            <View p-if="$over === 'end:' . $list->id && $dragging !== null" class="gap end-gap" />
            <Column p-if="$adding" class="composer" captureKeys="Escape" on:keyDown="key">
                <Input testId="new-card" class="field" multiline="true" submitBehavior="submit" model="draft" sync="immediate"
                       placeholder="Enter a title or paste a link" placeholderColor="#626F86" autoFocus="true" @submit="add" />
                <Row class="composer-actions">
                    <Pressable class="add-button" cursor="pointer" @press="add">
                        <Text class="add-button-text">Add card</Text>
                    </Pressable>
                    <Pressable class="close" cursor="pointer" @press="cancel" tooltip="Cancel">
                        <Icon name="x" font="Lucide" size="18" class="tool-icon" />
                    </Pressable>
                </Row>
            </Column>
            <Row p-else class="footer">
                <Pressable class="add" cursor="pointer" @press="compose">
                    <Icon name="plus" font="Lucide" size="16" class="add-icon" />
                    <Text class="add-text">Add a card</Text>
                </Pressable>
                <View class="tool" tooltip="Create from template…">
                    <Icon name="copy" font="Lucide" size="16" class="tool-icon" />
                </View>
            </Row>
        </Column>
    </DropTarget>
</template>

<style scoped>
    .list { width: 272px; max-height: 100%; padding-bottom: 8px; border-radius: 12px; background-color: var(--list);
            box-shadow: 0 1px 1px #091E4240; }
    .header { height: 44px; padding: 0 4px 0 16px; align-items: center; justify-content: space-between; }
    .title { flex: 1; font-size: 14px; font-weight: 600; }
    .header-tools { gap: 0; }
    .tool { width: 32px; height: 32px; border-radius: 8px; align-items: center; justify-content: center; }
    .tool:hover { background-color: var(--neutral-hover); }
    .tool-icon { color: var(--subtle); }
    .cards { min-height: 0; padding: 2px 8px 0 8px; scrollbar-width: thin; }
    .end-gap { margin: 0 8px 8px 8px; }
    .gap { height: 40px; margin-bottom: 8px; border-radius: 8px; background-color: var(--neutral-hover); }
    .footer { gap: 4px; padding: 0 8px; align-items: center; }
    .add { flex: 1; flex-direction: row; height: 32px; gap: 8px; padding: 0 8px; align-items: center; border-radius: 8px; }
    .add:hover { background-color: var(--neutral-hover); }
    .add-icon { color: var(--subtle); }
    .add-text { font-size: 14px; font-weight: 500; color: var(--subtle); }
    .composer { gap: 8px; padding: 0 8px; }
    .field { min-height: 56px; padding: 8px 12px; border-radius: 8px; border-width: 0; font-size: 14px;
             background-color: var(--card); box-shadow: 0 1px 1px #091E4240; }
    .composer-actions { gap: 4px; align-items: center; }
    .add-button { height: 32px; padding: 0 12px; justify-content: center; border-radius: 3px; background-color: var(--brand); }
    .add-button:hover { background-color: var(--brand-hover); }
    .add-button-text { font-size: 14px; font-weight: 500; color: #FFFFFF; }
    .close { width: 32px; height: 32px; border-radius: 3px; align-items: center; justify-content: center; }
    .close:hover { background-color: var(--neutral-hover); }
</style>
