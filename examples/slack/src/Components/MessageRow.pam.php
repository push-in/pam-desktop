<?php

declare(strict_types=1);

namespace App\Components;

use App\Domain\Entry;
use App\Services\Workspace;
use Pam\Desktop\Component;

final class MessageRow extends Component
{
    public function __construct(
        public Entry $entry,
        public Workspace $workspace,
        public bool $inThread = false,
    ) {
    }

    public function openThread(): void
    {
        $this->emit('thread', $this->entry->message->id);
    }

    public function react(string $emoji): void
    {
        $this->emit('react', ['message' => $this->entry->message->id, 'emoji' => $emoji]);
    }

    /** @return list<array{icon: string, label: string}> */
    public function actions(): array
    {
        $actions = [['icon' => 'smile-plus', 'label' => 'Add reaction']];
        if (!$this->inThread) {
            $actions[] = ['icon' => 'message-square-text', 'label' => 'Reply in thread'];
        }

        return [...$actions, ['icon' => 'forward', 'label' => 'Forward message'], ['icon' => 'bookmark', 'label' => 'Save for later'], ['icon' => 'ellipsis-vertical', 'label' => 'More actions']];
    }

    public function act(string $icon): void
    {
        match ($icon) {
            'message-square-text' => $this->openThread(),
            'smile-plus' => $this->react('👍'),
            default => null,
        };
    }

    public function avatar(string $memberId): string
    {
        return $this->workspace->member($memberId)->avatar;
    }
}
?>

<template>
    <Column p-if="$entry->isIntro()" class="intro">
        <Column p-if="$entry->peer !== null" class="intro-body">
            <Image :source="$entry->peer->avatar" class="intro-photo" />
            <Row class="intro-name-row">
                <Text class="intro-name">{{ $entry->peer->name }}</Text>
                <View :class="$entry->peer->active ? 'intro-presence online' : 'intro-presence'" />
            </Row>
            <Text class="intro-title">{{ $entry->peer->title }}</Text>
            <Text class="intro-text">
                <Span class="run-plain">This conversation is just between </Span>
                <Span class="run-mention">@{{ $entry->peer->name }}</Span>
                <Span class="run-plain"> and you. Check out their profile to learn more about them.</Span>
            </Text>
            <Row class="intro-button">
                <Text class="intro-button-text">View Profile</Text>
            </Row>
        </Column>
        <Column p-else class="intro-body">
            <Row class="intro-heading-row">
                <Icon :name="$entry->intro->icon()" font="Lucide" size="24" class="intro-heading-icon" />
                <Text class="intro-heading">{{ $entry->intro->name }}</Text>
            </Row>
            <Text class="intro-text">
                <Span class="run-plain">This is the very beginning of the </Span>
                <Span class="run-bold"># {{ $entry->intro->name }}</Span>
                <Span class="run-plain"> channel. {{ $entry->intro->topic }}.</Span>
            </Text>
            <Row class="intro-button">
                <Icon name="user-plus" font="Lucide" size="16" class="intro-button-icon" />
                <Text class="intro-button-text">Add coworkers</Text>
            </Row>
        </Column>
    </Column>

    <Row p-else-if="$entry->isDay()" class="day">
        <View class="rule" />
        <Row class="day-pill">
            <Text class="day-text">{{ $entry->day }}</Text>
            <Icon name="chevron-down" font="Lucide" size="12" class="day-icon" />
        </Row>
        <View class="rule" />
    </Row>

    <Row p-else :class="$entry->continued ? 'message continued' : 'message'">
        <Column class="gutter">
            <Image p-if="!$entry->continued" :source="$entry->author->avatar" class="avatar" />
        </Column>
        <Column class="content">
            <Row p-if="!$entry->continued" class="meta">
                <Text class="author">{{ $entry->author->name }}</Text>
                <Text class="time">{{ $entry->message->time() }}</Text>
            </Row>
            <Text class="body">
                <Span p-for="($run, $i) in $entry->runs" p-key="$i" :class="'run-' . $run->style">{{ $run->text }}</Span>
            </Text>
            <Row p-if="$entry->message->reactions !== []" class="reactions">
                <Pressable p-for="$chip in $entry->message->reactionChips(\App\Services\Workspace::ME)" p-key="$chip['emoji']"
                           :class="$chip['mine'] ? 'chip mine' : 'chip'" cursor="pointer" @press="react($chip['emoji'])">
                    <Text class="emoji">{{ $chip['emoji'] }}</Text>
                    <Text :class="$chip['mine'] ? 'chip-count mine-count' : 'chip-count'">{{ $chip['count'] }}</Text>
                </Pressable>
                <Column class="chip add-reaction">
                    <Icon name="smile-plus" font="Lucide" size="16" class="add-icon" />
                </Column>
            </Row>
            <Pressable p-if="!$inThread && $entry->message->replies > 0" class="thread" cursor="pointer" @press="openThread">
                <Image p-for="$id in $entry->message->repliedBy" p-key="$id" :source="avatar($id)" class="replier" />
                <Text class="replies">{{ $entry->message->repliesLabel() }}</Text>
                <Text class="last-reply">{{ $entry->message->lastReplyLabel() }}</Text>
            </Pressable>
        </Column>
        <Row class="actions">
            <Pressable p-for="$emoji in ['✅', '👀', '🙌']" p-key="$emoji" class="action" cursor="pointer" @press="react($emoji)">
                <Text class="quick">{{ $emoji }}</Text>
            </Pressable>
            <View class="action-split" />
            <Pressable p-for="$action in actions()" p-key="$action['icon']" class="action" cursor="pointer"
                       :tooltip="$action['label']" on:press="act($action['icon'])">
                <Icon :name="$action['icon']" font="Lucide" size="16" class="action-icon" />
            </Pressable>
        </Row>
    </Row>
</template>

<style scoped>
    .intro { padding: 24px 20px 16px 20px; }
    .intro-body { gap: 4px; align-items: flex-start; }
    .intro-photo { width: 80px; height: 80px; border-radius: 10px; margin-bottom: 12px; }
    .intro-name-row { gap: 8px; align-items: center; }
    .intro-name { font-size: 22px; font-weight: 900; }
    .intro-presence { width: 10px; height: 10px; border-radius: 5px; border-width: 1.5px; border-color: var(--muted); }
    .online { background-color: var(--presence); border-color: var(--presence); }
    .intro-title { font-size: 15px; color: var(--muted); }
    .intro-heading-row { gap: 6px; align-items: center; }
    .intro-heading-icon { color: var(--text); }
    .intro-heading { font-size: 28px; font-weight: 900; }
    .intro-text { margin-top: 12px; font-size: 15px; line-height: 22px; color: var(--muted); }
    .intro-button { flex-direction: row; height: 36px; gap: 6px; margin-top: 16px; padding: 0 12px; align-items: center;
                    border-radius: 8px; border-width: 1px; border-color: #1D1C1D4D; }
    .intro-button:hover { background-color: var(--hover); }
    .intro-button-icon { color: var(--text); }
    .intro-button-text { font-size: 15px; font-weight: 700; }

    .day { height: 40px; align-items: center; padding: 0 20px; }
    .rule { flex: 1; height: 1px; background-color: var(--faint); }
    .day-pill { height: 28px; gap: 4px; padding: 0 12px 0 16px; align-items: center; border-radius: 14px;
                border-width: 1px; border-color: var(--faint); background-color: var(--surface); }
    .day-text { font-size: 13px; font-weight: 700; }
    .day-icon { color: var(--text); }

    .message { padding: 8px 20px; gap: 8px; }
    .message:hover { background-color: var(--hover); }
    .actions { position: absolute; top: -16px; right: 20px; height: 34px; padding: 0 2px; gap: 1px; align-items: center;
               border-radius: 8px; border-width: 1px; border-color: var(--faint); background-color: var(--surface);
               box-shadow: 0 1px 3px #0000001A; opacity: 0; }
    .message:hover .actions { opacity: 1; }
    .action { width: 32px; height: 30px; border-radius: 6px; align-items: center; justify-content: center; }
    .action:hover { background-color: #1D1C1D0F; }
    .quick { font-size: 15px; }
    .action-split { width: 1px; height: 20px; margin: 0 3px; background-color: var(--faint); }
    .action-icon { color: var(--muted); }
    .continued { padding-top: 2px; padding-bottom: 2px; }
    .gutter { width: 36px; }
    .avatar { width: 36px; height: 36px; border-radius: 6px; }
    .content { flex: 1; gap: 2px; }
    .meta { gap: 8px; align-items: baseline; }
    .author { font-size: 15px; font-weight: 900; }
    .time { font-size: 12px; color: var(--muted); }
    .body { font-size: 15px; line-height: 22px; }
    .run-plain { font-weight: 400; }
    .run-bold { font-weight: 700; }
    .run-italic { font-style: italic; }
    .run-code { font-family: monospace; font-size: 12px; color: #E01E5A; background-color: #1D1C1D0A;
                padding: 2px 3px; border-width: 1px; border-color: var(--faint); border-radius: 3px; }
    .run-link { color: var(--link); }
    .run-mention-me { color: var(--link); background-color: #F2C74466; border-radius: 3px; padding: 0 2px; }
    .run-mention { color: var(--link); background-color: #1D9BD11A; border-radius: 3px; padding: 0 2px; }

    .reactions { gap: 4px; margin-top: 4px; flex-wrap: wrap; }
    .chip { flex-direction: row; height: 24px; gap: 4px; padding: 0 8px; align-items: center; border-radius: 12px;
            background-color: #1D1C1D0F; border-width: 1px; border-color: #00000000; }
    .chip:hover { border-color: #1D1C1D40; background-color: var(--surface); }
    .mine { background-color: var(--reaction); border-color: var(--reaction-border); }
    .emoji { font-size: 15px; }
    .chip-count { font-size: 11px; font-weight: 700; color: var(--text); }
    .mine-count { color: var(--link); }
    .add-reaction { width: 34px; justify-content: center; }
    .add-icon { color: var(--muted); }

    .thread { flex-direction: row; align-self: flex-start; gap: 4px; margin-top: 4px; padding: 5px 8px 5px 5px; margin-left: -5px; align-items: center; border-radius: 6px; }
    .thread:hover { background-color: var(--surface); border-width: 1px; border-color: var(--faint); }
    .replier { width: 24px; height: 24px; border-radius: 4px; }
    .replies { margin-left: 4px; font-size: 13px; font-weight: 700; color: var(--link); }
    .last-reply { margin-left: 4px; font-size: 13px; color: var(--muted); }
</style>
