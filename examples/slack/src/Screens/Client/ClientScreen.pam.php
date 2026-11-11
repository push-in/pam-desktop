<?php

declare(strict_types=1);

namespace App\Screens\Client;

use App\Domain\Conversation;
use App\Services\Workspace;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;

final class ClientScreen extends Component
{
    public Workspace $workspace;

    #[State]
    public string $activeId = 'design';

    #[State]
    public ?string $threadId = null;

    /** Bumped when the workspace changes in place, so the screen renders again. */
    #[State]
    public int $revision = 0;

    /** @var list<array{icon: string, label: string}> */
    public array $rail = [
        ['icon' => 'house', 'label' => 'Home'],
        ['icon' => 'messages-square', 'label' => 'DMs'],
        ['icon' => 'bell', 'label' => 'Activity'],
        ['icon' => 'bookmark', 'label' => 'Later'],
        ['icon' => 'ellipsis', 'label' => 'More'],
    ];

    public function mount(): void
    {
        $this->workspace = new Workspace();
        $this->workspace->markRead($this->activeId);
    }

    public function open(string $id): void
    {
        $this->workspace->markRead($id);
        $this->activeId = $id;
        $this->threadId = null;
    }

    public function openThread(string $messageId): void
    {
        $this->threadId = $messageId;
    }

    public function closeThread(): void
    {
        $this->threadId = null;
    }

    public function reply(string $text): void
    {
        $this->workspace->reply($this->activeId, (string) $this->threadId, $text);
        $this->revision++;
    }

    public function send(string $text): void
    {
        $this->workspace->post($this->activeId, Workspace::ME, $text);
        $this->revision++;
    }

    /** @param array{message: string, emoji: string} $reaction */
    public function react(array $reaction): void
    {
        $this->workspace->toggleReaction($this->activeId, $reaction['message'], $reaction['emoji']);
        $this->revision++;
    }

    public function active(): Conversation
    {
        return $this->workspace->conversation($this->activeId);
    }
}
?>

<template>
    <Column class="window">
        <Row class="topbar">
            <Row class="history">
                <Icon name="arrow-left" font="Lucide" size="18" class="nav-icon" />
                <Icon name="arrow-right" font="Lucide" size="18" class="nav-icon dim" />
                <Icon name="clock" font="Lucide" size="17" class="nav-icon" />
            </Row>
            <SearchBar :workspace="$workspace" @open="open" />
            <Row class="history end">
                <Icon name="circle-help" font="Lucide" size="18" class="nav-icon" />
            </Row>
        </Row>

        <Row class="body">
            <Column class="rail">
                <Text class="workspace-logo">A</Text>
                <Column p-for="$item in $rail" p-key="$item['label']" class="rail-item">
                    <Column :class="$item['label'] === 'Home' ? 'rail-icon rail-on' : 'rail-icon'">
                        <Icon :name="$item['icon']" font="Lucide" size="20" class="rail-glyph" />
                    </Column>
                    <Text class="rail-label">{{ $item['label'] }}</Text>
                </Column>
                <Column class="rail-spacer" />
                <Column class="rail-add">
                    <Icon name="plus" font="Lucide" size="20" class="rail-glyph" />
                </Column>
                <Column class="me">
                    <Image :source="$workspace->me()->avatar" class="me-photo" />
                    <View class="me-presence" />
                </Column>
            </Column>

            <Row class="frame">
                <Sidebar :workspace="$workspace" :activeId="$activeId" @open="open" />

                <Column class="channel">
                    <Row class="channel-header">
                        <Row class="channel-title">
                            <Icon p-if="!active()->isDirect()" :name="active()->icon()" font="Lucide" size="18" class="title-icon" />
                            <Image p-else :source="$workspace->member(active()->memberId)->avatar" class="title-photo" />
                            <Text class="title">{{ active()->name }}</Text>
                            <Icon name="chevron-down" font="Lucide" size="16" class="title-icon" />
                        </Row>
                        <Row class="header-tools">
                            <Row p-if="!active()->isDirect()" class="member-count">
                                <Image :source="$workspace->member('u-ana')->avatar" class="stack-photo" />
                                <Image :source="$workspace->member('u-bruno')->avatar" class="stack-photo" />
                                <Image :source="$workspace->member('u-carla')->avatar" class="stack-photo" />
                                <Text class="count">{{ active()->members }}</Text>
                            </Row>
                            <Row class="huddle">
                                <Icon name="headphones" font="Lucide" size="16" class="tool-icon" />
                                <Icon name="chevron-down" font="Lucide" size="14" class="tool-icon" />
                            </Row>
                            <Icon name="ellipsis-vertical" font="Lucide" size="18" class="tool-icon" />
                        </Row>
                    </Row>
                    <Row class="tabs">
                        <Row class="tab tab-on">
                            <Icon name="message-circle" font="Lucide" size="15" class="tab-icon" />
                            <Text class="tab-text tab-text-on">Messages</Text>
                        </Row>
                        <Row class="tab">
                            <Icon name="file-text" font="Lucide" size="15" class="tab-icon" />
                            <Text class="tab-text">Canvas</Text>
                        </Row>
                        <Row class="tab">
                            <Icon name="plus" font="Lucide" size="15" class="tab-icon" />
                        </Row>
                    </Row>
                    <VirtualizedList class="messages" inverted="true" estimatedRowHeight="64">
                        <MessageRow p-for="$entry in $workspace->timeline($activeId)" :key="$activeId . $entry->key"
                                    :entry="$entry" :workspace="$workspace" @react="react" @thread="openThread" />
                    </VirtualizedList>
                    <Composer :key="'composer-' . $activeId" :placeholder="active()->placeholder()" @send="send" />
                </Column>

                <ThreadPane p-if="$threadId !== null" :key="'pane-' . $threadId" :workspace="$workspace"
                            :conversation="active()" :parentId="$threadId" @close="closeThread" @reply="reply" @react="react" />
            </Row>
        </Row>
    </Column>
</template>

<style scoped>
    .window { flex: 1; background-color: var(--chrome); }

    .topbar { height: 40px; align-items: center; padding: 0 8px; z-index: 10; }
    .history { width: 220px; gap: 14px; padding-left: 64px; align-items: center; }
    .end { justify-content: flex-end; padding-left: 0; padding-right: 8px; }
    .nav-icon { color: var(--chrome-text); }
    .dim { opacity: 0.4; }

    .body { flex: 1; }
    .rail { width: 70px; align-items: center; padding-top: 8px; gap: 12px; padding-bottom: 16px; }
    .workspace-logo { width: 36px; height: 36px; border-radius: 8px; background-color: #FFFFFF; color: var(--chrome);
                      font-size: 20px; font-weight: 900; text-align: center; line-height: 36px; margin-bottom: 4px; }
    .rail-item { align-items: center; gap: 4px; cursor: pointer; }
    .rail-icon { width: 36px; height: 36px; border-radius: 8px; align-items: center; justify-content: center; }
    .rail-icon:hover { background-color: var(--chrome-hover); }
    .rail-on { background-color: #FFFFFF33; }
    .rail-glyph { color: var(--chrome-text); }
    .rail-label { color: var(--chrome-text); font-size: 11px; font-weight: 700; }
    .rail-spacer { flex: 1; }
    .rail-add { width: 36px; height: 36px; border-radius: 18px; background-color: #FFFFFF26; align-items: center; justify-content: center; }
    .me { width: 36px; height: 36px; }
    .me-photo { width: 36px; height: 36px; border-radius: 8px; }
    .me-presence { position: absolute; right: -3px; bottom: -3px; width: 12px; height: 12px; border-radius: 6px;
                   background-color: var(--presence); border-width: 2px; border-color: var(--chrome); }

    .frame { flex: 1; margin: 0 4px 4px 0; border-radius: 8px; overflow: hidden; background-color: var(--sidebar); }
    .channel { flex: 1; background-color: var(--surface); border-top-left-radius: 8px; border-bottom-left-radius: 8px; }
    .channel-header { height: 49px; padding: 0 12px 0 20px; align-items: center; justify-content: space-between; }
    .channel-title { gap: 4px; align-items: center; padding: 3px 8px 3px 4px; border-radius: 6px; cursor: pointer; }
    .channel-title:hover { background-color: var(--hover); }
    .title { font-size: 18px; font-weight: 900; }
    .title-icon { color: var(--text); }
    .title-photo { width: 24px; height: 24px; border-radius: 6px; margin-right: 4px; }
    .header-tools { gap: 10px; align-items: center; }
    .member-count { align-items: center; padding: 3px 8px 3px 4px; border-radius: 6px; border-width: 1px; border-color: var(--faint); }
    .stack-photo { width: 20px; height: 20px; border-radius: 4px; border-width: 2px; border-color: var(--surface); margin-right: -4px; }
    .count { margin-left: 10px; font-size: 13px; font-weight: 700; color: var(--muted); }
    .huddle { gap: 2px; align-items: center; height: 28px; padding: 0 8px; border-radius: 6px; border-width: 1px; border-color: var(--faint); }
    .tool-icon { color: var(--muted); }
    .tabs { height: 38px; padding: 0 20px; gap: 4px; border-bottom-width: 1px; border-color: var(--faint); }
    .tab { gap: 6px; align-items: center; padding: 0 8px; border-bottom-width: 2px; border-color: #00000000; }
    .tab-on { border-color: var(--chrome); }
    .tab-icon { color: var(--muted); }
    .tab-text { font-size: 13px; font-weight: 700; color: var(--muted); }
    .tab-text-on { color: var(--text); }
    .messages { flex: 1; }
</style>
