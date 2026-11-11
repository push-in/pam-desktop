<?php

declare(strict_types=1);

namespace App\Components;

use App\Domain\Card;
use App\Services\Board;
use Pam\Desktop\Component;
use Pam\Desktop\DragPayload;

/** A card in a list: dragged by itself, and a drop before it puts a card there. */
final class CardTile extends Component
{
    public const int DRAG = 1;

    public function __construct(
        public Card $card,
        public Board $board,
        public bool $dragging = false,
        public bool $over = false,
    ) {
    }

    public function open(): void
    {
        $this->emit('open', $this->card->id);
    }

    public function start(): void
    {
        $this->emit('dragStart', $this->card->id);
    }

    public function end(): void
    {
        $this->emit('dragEnd');
    }

    public function enter(DragPayload $drag): void
    {
        $this->emit('over', $this->card->id);
    }

    public function drop(DragPayload $drag): void
    {
        $this->emit('drop', ['card' => $drag->data, 'before' => $this->card->id]);
    }

    public function toggleComplete(): void
    {
        $this->emit('complete', $this->card->id);
    }

    public function dueClass(): string
    {
        return 'due due-'.$this->card->dueState($this->board->today());
    }

    public function tileClass(): string
    {
        return $this->dragging ? 'tile tile-ghost' : 'tile';
    }
}
?>

<template>
    <DropTarget dropType="1" on:drop="drop" on:dragEnter="enter">
        <Column class="slot">
            <View p-if="$over" class="gap" />
            <DragSource dragType="1" :dragData="$card->id" on:dragStart="start" on:dragEnd="end">
                <Pressable :class="tileClass()" cursor="pointer" @press="open" :accessibilityLabel="$card->title">
                    <Image p-if="$card->cover !== null" :source="$card->cover" class="cover" />
                    <Column class="content">
                        <Row p-if="$card->labels !== []" class="labels">
                            <View p-for="$label in $card->labels" p-key="$label->name" class="label"
                                  :backgroundColor="$label->color->value" :tooltip="$label->name" />
                        </Row>
                        <Row class="title-row">
                            <Pressable p-if="$card->due !== null || $card->complete" class="check" cursor="pointer"
                                       @press="toggleComplete" :tooltip="$card->complete ? 'Mark incomplete' : 'Mark complete'">
                                <Icon :name="$card->complete ? 'circle-check' : 'circle'" font="Lucide" size="16"
                                      :class="$card->complete ? 'check-on' : 'check-off'" />
                            </Pressable>
                            <Text class="title">{{ $card->title }}</Text>
                        </Row>
                        <Row p-if="$card->hasBadges()" class="badges">
                            <Icon p-if="$card->watching" name="eye" font="Lucide" size="14" class="badge-icon" />
                            <Row p-if="$card->due !== null" :class="dueClass()">
                                <Icon name="clock-3" font="Lucide" size="14" class="due-icon" />
                                <Text class="due-text">{{ $card->dueLabel() }}</Text>
                            </Row>
                            <Icon p-if="$card->description !== ''" name="text" font="Lucide" size="14" class="badge-icon" tooltip="This card has a description." />
                            <Row p-if="$card->comments > 0" class="badge">
                                <Icon name="message-square" font="Lucide" size="14" class="badge-icon" />
                                <Text class="badge-text">{{ $card->comments }}</Text>
                            </Row>
                            <Row p-if="$card->attachments > 0" class="badge">
                                <Icon name="paperclip" font="Lucide" size="14" class="badge-icon" />
                                <Text class="badge-text">{{ $card->attachments }}</Text>
                            </Row>
                            <Row p-if="$card->checklist !== null" :class="$card->checklistDone() ? 'badge badge-done' : 'badge'">
                                <Icon name="square-check-big" font="Lucide" size="14" :class="$card->checklistDone() ? 'done-icon' : 'badge-icon'" />
                                <Text :class="$card->checklistDone() ? 'badge-text done-text' : 'badge-text'">{{ $card->checklistLabel() }}</Text>
                            </Row>
                            <View class="spacer" />
                            <Row class="members">
                                <Text p-for="$id in $card->memberIds" p-key="$id" class="avatar"
                                      :backgroundColor="$board->member($id)->color" :tooltip="$board->member($id)->name">{{ $board->member($id)->initials() }}</Text>
                            </Row>
                        </Row>
                    </Column>
                    <View class="edit">
                        <Icon name="pencil" font="Lucide" size="14" class="edit-icon" />
                    </View>
                </Pressable>
            </DragSource>
        </Column>
    </DropTarget>
</template>

<style scoped>
    .slot { padding-bottom: 8px; }
    .gap { height: 40px; margin-bottom: 8px; border-radius: 8px; background-color: var(--neutral-hover); }
    .tile { border-radius: 8px; border-width: 2px; border-color: #00000000; background-color: var(--card);
            box-shadow: 0 1px 1px #091E4240; }
    .tile:hover { border-color: var(--focus); }
    .tile-ghost { opacity: 0.4; }
    .cover { height: 128px; border-top-left-radius: 8px; border-top-right-radius: 8px; }
    .content { padding: 8px 12px 6px 12px; gap: 4px; }
    .labels { gap: 4px; flex-wrap: wrap; padding-bottom: 2px; }
    .label { width: 40px; height: 8px; border-radius: 4px; }
    .title-row { gap: 4px; align-items: flex-start; }
    .check { width: 16px; height: 20px; justify-content: center; }
    .check-on { color: var(--success); }
    .check-off { color: var(--subtle); }
    .title { flex: 1; font-size: 14px; line-height: 20px; }
    .badges { gap: 8px; align-items: center; flex-wrap: wrap; padding-top: 2px; }
    .badge { gap: 4px; align-items: center; height: 24px; padding: 0 2px; border-radius: 3px; }
    .badge-done { background-color: var(--success); padding: 0 4px; }
    .badge-icon { color: var(--subtle); }
    .done-icon { color: #FFFFFF; }
    .badge-text { font-size: 12px; color: var(--subtle); }
    .done-text { color: #FFFFFF; }
    .due { gap: 4px; align-items: center; height: 24px; padding: 0 4px; border-radius: 3px; }
    .due-icon { color: var(--subtle); }
    .due-text { font-size: 12px; color: var(--subtle); }
    .due-upcoming { background-color: #00000000; }
    .due-soon { background-color: var(--warning); }
    .due-overdue { background-color: var(--danger); }
    .due-complete { background-color: var(--success); }
    .due-overdue .due-icon, .due-complete .due-icon { color: #FFFFFF; }
    .due-overdue .due-text, .due-complete .due-text { color: #FFFFFF; }
    .due-soon .due-icon, .due-soon .due-text { color: var(--text); }
    .spacer { flex: 1; }
    .members { gap: 0; flex-direction: row-reverse; }
    .avatar { width: 24px; height: 24px; border-radius: 12px; margin-left: -4px; font-size: 10px; font-weight: 700;
              color: #FFFFFF; text-align: center; line-height: 24px; border-width: 2px; border-color: var(--card); }
    .edit { position: absolute; top: 4px; right: 4px; width: 32px; height: 32px; border-radius: 16px;
            background-color: var(--card); align-items: center; justify-content: center; opacity: 0; }
    .tile:hover .edit { opacity: 1; }
    .edit-icon { color: var(--subtle); }
</style>
