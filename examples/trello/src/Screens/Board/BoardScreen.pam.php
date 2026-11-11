<?php

declare(strict_types=1);

namespace App\Screens\Board;

use App\Services\Board;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;

final class BoardScreen extends Component
{
    public Board $board;

    /** @var list<array{icon: string, label: string}> */
    public array $menus = [
        ['icon' => '', 'label' => 'Workspaces'], ['icon' => '', 'label' => 'Recent'],
        ['icon' => '', 'label' => 'Starred'], ['icon' => '', 'label' => 'Templates'],
    ];

    #[State]
    public int $revision = 0;

    #[State]
    public ?string $dragging = null;

    /** The card a drag hovers (it lands before it), or "end:<list>". */
    #[State]
    public ?string $over = null;

    #[State]
    public ?string $openId = null;

    #[State]
    public bool $starred = true;

    public function mount(): void
    {
        $this->board = new Board();
    }

    public function dragStart(string $cardId): void
    {
        $this->dragging = $cardId;
    }

    public function dragEnd(): void
    {
        $this->dragging = null;
        $this->over = null;
    }

    public function hover(string $target): void
    {
        $this->over = $target;
    }

    /** @param array{card: string, list: string, before?: string} $drop */
    public function drop(array $drop): void
    {
        $this->board->move($drop['card'], $drop['list'], $drop['before'] ?? null);
        $this->dragging = null;
        $this->over = null;
        $this->revision++;
    }

    /** @param array{list: string, title: string} $request */
    public function add(array $request): void
    {
        $this->board->add($request['list'], $request['title']);
        $this->revision++;
    }

    public function complete(string $cardId): void
    {
        $this->board->toggleComplete($cardId);
        $this->revision++;
    }

    public function open(string $cardId): void
    {
        $this->openId = $cardId;
    }

    public function close(): void
    {
        $this->openId = null;
        $this->revision++;
    }

    public function changed(): void
    {
        $this->revision++;
    }

    public function star(): void
    {
        $this->starred = !$this->starred;
    }
}
?>

<template>
    <ImageBackground source="asset://assets/backgrounds/mountains.jpg" resizeMode="cover" class="window">
        <Row class="appbar">
            <Row class="brand">
                <View class="apps"><Icon name="grip" font="Lucide" size="18" class="chrome-icon" /></View>
                <Row class="logo">
                    <View class="logo-mark"><Icon name="trello" font="Lucide" size="16" class="logo-glyph" /></View>
                    <Text class="logo-text">Trello</Text>
                </Row>
                <Row p-for="$menu in $menus" p-key="$menu['label']" class="menu">
                    <Text class="menu-text">{{ $menu['label'] }}</Text>
                    <Icon name="chevron-down" font="Lucide" size="14" class="chrome-icon" />
                </Row>
                <Row class="create"><Text class="create-text">Create</Text></Row>
            </Row>
            <Row class="tools">
                <Row class="search">
                    <Icon name="search" font="Lucide" size="16" class="chrome-icon" />
                    <Text class="search-text">Search</Text>
                </Row>
                <View class="round"><Icon name="bell" font="Lucide" size="18" class="chrome-icon" /></View>
                <View class="round"><Icon name="circle-help" font="Lucide" size="18" class="chrome-icon" /></View>
                <Text class="me">DB</Text>
            </Row>
        </Row>

        <Row class="boardbar">
            <Row class="boardbar-left">
                <Text class="board-name">{{ \App\Services\Board::NAME }}</Text>
                <Pressable class="bar-button" cursor="pointer" @press="star" :tooltip="$starred ? 'Unstar' : 'Star'">
                    <Icon name="star" font="Lucide" size="16" :class="$starred ? 'starred' : 'chrome-icon'" />
                </Pressable>
                <View class="bar-button"><Icon name="users" font="Lucide" size="16" class="chrome-icon" /></View>
                <Row class="view-chip">
                    <Icon name="square-kanban" font="Lucide" size="16" class="view-icon" />
                    <Text class="view-text">Board</Text>
                </Row>
                <View class="bar-button"><Icon name="chevron-down" font="Lucide" size="16" class="chrome-icon" /></View>
            </Row>
            <Row class="boardbar-right">
                <View class="bar-button"><Icon name="rocket" font="Lucide" size="16" class="chrome-icon" /></View>
                <View class="bar-button"><Icon name="zap" font="Lucide" size="16" class="chrome-icon" /></View>
                <Row class="filter">
                    <Icon name="list-filter" font="Lucide" size="16" class="chrome-icon" />
                    <Text class="filter-text">Filters</Text>
                </Row>
                <View class="divider" />
                <Row class="faces">
                    <Text p-for="$member in $board->members()" p-key="$member->id" class="face"
                          :backgroundColor="$member->color" :tooltip="$member->name">{{ $member->initials() }}</Text>
                </Row>
                <Row class="share">
                    <Icon name="user-plus" font="Lucide" size="16" class="share-icon" />
                    <Text class="share-text">Share</Text>
                </Row>
                <View class="bar-button"><Icon name="ellipsis" font="Lucide" size="16" class="chrome-icon" /></View>
            </Row>
        </Row>

        <ScrollView class="canvas" horizontal="true" scrollbarWidth="thin">
            <Row class="lists">
                <ListColumn p-for="$list in $board->lists()" p-key="$list->id" :list="$list" :board="$board"
                            :dragging="$dragging" :over="$over" :revision="$revision"
                            @open="open" @dragStart="dragStart" @dragEnd="dragEnd" @over="hover" @drop="drop"
                            @add="add" @complete="complete" />
                <Row class="add-list">
                    <Icon name="plus" font="Lucide" size="16" class="chrome-icon" />
                    <Text class="add-list-text">Add another list</Text>
                </Row>
            </Row>
        </ScrollView>

        <CardDetail p-if="$openId !== null" :key="'detail-' . $openId" :board="$board" :cardId="$openId"
                    @close="close" @changed="changed" />
    </ImageBackground>
</template>

<style scoped>
    .window { flex: 1; background-color: #1D4E6E; }
    .appbar { height: 48px; padding: 0 8px; align-items: center; justify-content: space-between;
              background-color: #0000005C; border-bottom-width: 1px; border-color: #FFFFFF29; }
    .brand { gap: 4px; align-items: center; }
    .apps { width: 32px; height: 32px; border-radius: 3px; align-items: center; justify-content: center; }
    .logo { gap: 6px; align-items: center; padding: 0 8px; }
    .logo-mark { width: 20px; height: 20px; border-radius: 4px; background-color: #FFFFFF; align-items: center; justify-content: center; }
    .logo-glyph { color: var(--brand); }
    .logo-text { font-size: 18px; font-weight: 700; color: var(--chrome-text); }
    .menu { height: 32px; gap: 4px; padding: 0 8px; align-items: center; border-radius: 3px; }
    .menu:hover { background-color: var(--chrome); }
    .menu-text { font-size: 14px; font-weight: 500; color: var(--chrome-text); }
    .create { height: 32px; margin-left: 4px; padding: 0 12px; justify-content: center; border-radius: 3px; background-color: var(--chrome); }
    .create:hover { background-color: var(--chrome-hover); }
    .create-text { font-size: 14px; font-weight: 500; color: var(--chrome-text); }
    .tools { gap: 6px; align-items: center; }
    .search { width: 200px; height: 32px; gap: 8px; padding: 0 8px; align-items: center; border-radius: 6px;
              background-color: var(--chrome); border-width: 1px; border-color: #FFFFFF3D; }
    .search-text { font-size: 14px; color: var(--chrome-text); }
    .round { width: 32px; height: 32px; border-radius: 16px; align-items: center; justify-content: center; }
    .round:hover { background-color: var(--chrome); }
    .chrome-icon { color: var(--chrome-text); }
    .me { width: 24px; height: 24px; margin: 0 4px; border-radius: 12px; background-color: var(--brand); font-size: 10px;
          font-weight: 700; color: #FFFFFF; text-align: center; line-height: 24px; }

    .boardbar { height: 56px; padding: 0 10px 0 16px; align-items: center; justify-content: space-between; background-color: #0000003D; }
    .boardbar-left, .boardbar-right { gap: 4px; align-items: center; }
    .board-name { font-size: 18px; font-weight: 700; color: var(--chrome-text); padding: 0 10px 0 0; }
    .bar-button { width: 32px; height: 32px; border-radius: 3px; align-items: center; justify-content: center; }
    .bar-button:hover { background-color: var(--chrome); }
    .starred { color: var(--warning); }
    .view-chip { height: 32px; gap: 6px; padding: 0 12px; align-items: center; border-radius: 3px; background-color: #DFE1E6; }
    .view-icon { color: var(--text); }
    .view-text { font-size: 14px; font-weight: 500; }
    .filter { height: 32px; gap: 6px; padding: 0 12px; align-items: center; border-radius: 3px; }
    .filter:hover { background-color: var(--chrome); }
    .filter-text { font-size: 14px; font-weight: 500; color: var(--chrome-text); }
    .divider { width: 1px; height: 16px; margin: 0 8px; background-color: #FFFFFF3D; }
    .faces { gap: 0; padding: 0 8px; }
    .face { width: 28px; height: 28px; margin-left: -4px; border-radius: 14px; border-width: 2px; border-color: #FFFFFF;
            font-size: 11px; font-weight: 700; color: #FFFFFF; text-align: center; line-height: 24px; }
    .share { height: 32px; gap: 6px; padding: 0 12px; align-items: center; border-radius: 3px; background-color: #DFE1E6; }
    .share-icon { color: var(--text); }
    .share-text { font-size: 14px; font-weight: 500; }

    .canvas { flex: 1; }
    .lists { height: 100%; gap: 12px; padding: 12px 12px 16px 12px; align-items: flex-start; }
    .add-list { width: 272px; height: 44px; gap: 8px; padding: 0 12px; align-items: center; border-radius: 12px;
                background-color: #FFFFFF3D; }
    .add-list:hover { background-color: #FFFFFF33; }
    .add-list-text { font-size: 14px; font-weight: 500; color: var(--chrome-text); }
</style>
