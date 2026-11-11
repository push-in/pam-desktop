<?php

declare(strict_types=1);

namespace App\Components;

use App\Domain\Conversation;
use App\Services\Workspace;
use Pam\Desktop\Component;

final class Sidebar extends Component
{
    /** @var list<array{icon: string, label: string}> */
    public array $shortcuts = [
        ['icon' => 'messages-square', 'label' => 'Threads'],
        ['icon' => 'headphones', 'label' => 'Huddles'],
        ['icon' => 'send-horizontal', 'label' => 'Drafts & sent'],
        ['icon' => 'book-user', 'label' => 'Directories'],
    ];

    public function __construct(
        public Workspace $workspace,
        public string $activeId,
    ) {
    }

    public function open(string $id): void
    {
        $this->emit('open', $id);
    }

    public function rowClass(Conversation $conversation): string
    {
        return match (true) {
            $conversation->id === $this->activeId => 'row row-active',
            $conversation->unread > 0 => 'row row-unread',
            default => 'row',
        };
    }
}
?>

<template>
    <Column class="sidebar">
        <Row class="header">
            <Row class="workspace">
                <Text class="workspace-name">{{ \App\Services\Workspace::NAME }}</Text>
                <Icon name="chevron-down" font="Lucide" size="16" class="header-icon" />
            </Row>
            <Row class="header-tools">
                <Icon name="sliders-horizontal" font="Lucide" size="16" class="header-icon" />
                <Column class="compose">
                    <Icon name="square-pen" font="Lucide" size="16" class="compose-icon" />
                </Column>
            </Row>
        </Row>

        <ScrollView class="scroll" scrollbarWidth="thin">
            <Column class="list">
                <Row p-for="$item in $shortcuts" p-key="$item['label']" class="row">
                    <Icon :name="$item['icon']" font="Lucide" size="16" class="row-icon" />
                    <Text class="row-text">{{ $item['label'] }}</Text>
                </Row>

                <Row class="section">
                    <Icon name="chevron-down" font="Lucide" size="14" class="section-icon" />
                    <Text class="section-text">Channels</Text>
                </Row>
                <Pressable p-for="$channel in $workspace->channels()" p-key="$channel->id"
                           :class="rowClass($channel)" cursor="pointer" @press="open($channel->id)">
                    <Icon :name="$channel->icon()" font="Lucide" size="15" class="row-icon" />
                    <Text class="row-text">{{ $channel->name }}</Text>
                    <Text p-if="$channel->mentions > 0" class="badge">{{ $channel->mentions }}</Text>
                </Pressable>
                <Row class="row">
                    <Column class="add"><Icon name="plus" font="Lucide" size="12" class="row-icon" /></Column>
                    <Text class="row-text">Add channels</Text>
                </Row>

                <Row class="section">
                    <Icon name="chevron-down" font="Lucide" size="14" class="section-icon" />
                    <Text class="section-text">Direct messages</Text>
                </Row>
                <Pressable p-for="$dm in $workspace->directMessages()" p-key="$dm->id"
                           :class="rowClass($dm)" cursor="pointer" @press="open($dm->id)">
                    <Column class="dm-photo">
                        <Image :source="$workspace->member($dm->memberId)->avatar" class="photo" />
                        <View :class="$workspace->member($dm->memberId)->active ? 'presence on' : 'presence'" />
                    </Column>
                    <Text class="row-text">{{ $dm->name }}</Text>
                    <Text p-if="$dm->mentions > 0" class="badge">{{ $dm->mentions }}</Text>
                </Pressable>
                <Row class="row">
                    <Column class="add"><Icon name="plus" font="Lucide" size="12" class="row-icon" /></Column>
                    <Text class="row-text">Invite people</Text>
                </Row>
            </Column>
        </ScrollView>
    </Column>
</template>

<style scoped>
    .sidebar { width: 260px; }
    .header { height: 49px; padding: 0 12px 0 16px; align-items: center; justify-content: space-between; }
    .workspace { gap: 4px; align-items: center; padding: 3px 6px; margin-left: -6px; border-radius: 6px; }
    .workspace:hover { background-color: var(--chrome-hover); }
    .workspace-name { color: var(--chrome-text); font-size: 18px; font-weight: 900; }
    .header-icon { color: var(--chrome-text); }
    .header-tools { gap: 12px; align-items: center; }
    .compose { width: 32px; height: 32px; border-radius: 16px; background-color: #FFFFFF; align-items: center; justify-content: center; }
    .compose-icon { color: var(--chrome); }

    .scroll { flex: 1; }
    .list { padding: 6px 8px 16px 8px; gap: 1px; }
    .row { flex-direction: row; height: 28px; gap: 8px; padding: 0 10px 0 16px; align-items: center; border-radius: 6px; }
    .row:hover { background-color: var(--chrome-hover); }
    .row-icon { color: var(--chrome-muted); }
    .row-text { flex: 1; color: var(--chrome-muted); font-size: 15px; white-space: nowrap; text-overflow: ellipsis; }
    .row-unread .row-text { color: var(--chrome-text); font-weight: 900; }
    .row-unread .row-icon { color: var(--chrome-text); }
    .row-active { background-color: var(--chrome-active); }
    .row-active:hover { background-color: var(--chrome-active); }
    .row-active .row-text { color: var(--chrome-active-text); font-weight: 700; }
    .row-active .row-icon { color: var(--chrome-active-text); }
    .badge { min-width: 20px; height: 18px; padding: 0 6px; border-radius: 9px; background-color: var(--badge);
             color: #FFFFFF; font-size: 12px; font-weight: 700; text-align: center; line-height: 18px; }
    .section { height: 28px; gap: 6px; padding: 0 10px 0 8px; margin-top: 10px; align-items: center; }
    .section-icon { color: var(--chrome-muted); }
    .section-text { color: var(--chrome-muted); font-size: 15px; }
    .add { width: 20px; height: 20px; border-radius: 4px; background-color: #FFFFFF1A; align-items: center; justify-content: center; }
    .dm-photo { width: 20px; height: 20px; }
    .photo { width: 20px; height: 20px; border-radius: 4px; }
    .presence { position: absolute; right: -3px; bottom: -3px; width: 9px; height: 9px; border-radius: 5px;
                border-width: 2px; border-color: var(--chrome); background-color: var(--chrome); }
    .presence.on { background-color: var(--presence); }
</style>
