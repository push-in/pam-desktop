<?php

declare(strict_types=1);

namespace App\Components;

use App\Domain\Playlist;
use App\Domain\PlaylistKind;
use App\Services\Library;
use App\Services\Player;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;

final class LibraryPanel extends Component
{
    /** @var list<PlaylistKind> */
    public array $filters = [PlaylistKind::Playlist, PlaylistKind::Artist, PlaylistKind::Album];

    #[State]
    public ?string $filter = null;

    public function __construct(
        public Library $library,
        public Player $player,
        public ?string $activeId = null,
        public int $revision = 0,
    ) {
    }

    /** @return list<Playlist> */
    public function items(): array
    {
        return $this->library->saved($this->filter === null ? null : PlaylistKind::from($this->filter));
    }

    public function choose(string $kind): void
    {
        $this->filter = $this->filter === $kind ? null : $kind;
    }

    public function open(string $id): void
    {
        $this->emit('open', $id);
    }

    public function chip(string $kind): string
    {
        return $this->filter === $kind ? 'chip chip-on' : 'chip';
    }

    public function chipText(string $kind): string
    {
        return $this->filter === $kind ? 'chip-text chip-text-on' : 'chip-text';
    }

    public function rowClass(Playlist $playlist): string
    {
        return $playlist->id === $this->activeId ? 'item item-on' : 'item';
    }

    public function playing(Playlist $playlist): bool
    {
        return $this->player->contextId === $playlist->id;
    }
}
?>

<template>
    <Column class="panel">
        <Row class="header">
            <Row class="heading">
                <Icon name="library" font="Lucide" size="22" class="subdued" />
                <Text class="heading-text">Your Library</Text>
            </Row>
            <Row class="create" tooltip="Create a playlist, folder or Jam">
                <Icon name="plus" font="Lucide" size="16" class="text" />
                <Text class="create-text">Create</Text>
            </Row>
        </Row>
        <Row class="chips">
            <Pressable p-for="$kind in $filters" p-key="$kind->value" :class="chip($kind->value)" cursor="pointer"
                       on:press="choose($kind->value)">
                <Text :class="chipText($kind->value)">{{ $kind->value }}s</Text>
            </Pressable>
        </Row>
        <Row class="tools">
            <Icon name="search" font="Lucide" size="16" class="subdued" />
            <Row class="sort">
                <Text class="sort-text">Recents</Text>
                <Icon name="list" font="Lucide" size="16" class="subdued" />
            </Row>
        </Row>
        <ScrollView class="scroll" scrollbarWidth="thin">
            <Column class="list">
                <Pressable p-for="$item in items()" p-key="$item->id" :class="rowClass($item)" cursor="pointer"
                           on:press="open($item->id)">
                    <View p-if="$item->id === 'liked'" class="liked-cover">
                        <Icon name="heart" font="Lucide" size="20" class="text" />
                    </View>
                    <Image p-else :source="$item->cover" :class="$item->isArtist() ? 'cover round' : 'cover'" />
                    <Column class="meta">
                        <Text :class="playing($item) ? 'name playing' : 'name'" numberOfLines="1">{{ $item->name }}</Text>
                        <Row class="sub">
                            <Icon p-if="$item->pinned" name="pin" font="Lucide" size="12" class="pin" />
                            <Text class="subtitle" numberOfLines="1">{{ $item->subtitle() }}</Text>
                        </Row>
                    </Column>
                    <Icon p-if="playing($item) && $player->playing()" name="audio-lines" font="Lucide" size="16" class="pin" />
                </Pressable>
            </Column>
        </ScrollView>
    </Column>
</template>

<style scoped>
    .panel { width: 280px; border-radius: 8px; background-color: var(--panel); }
    .header { height: 56px; padding: 0 16px 0 24px; align-items: center; justify-content: space-between; }
    .heading { gap: 12px; align-items: center; }
    .heading-text { font-size: 16px; font-weight: 700; }
    .subdued { color: var(--subdued); }
    .text { color: var(--text); }
    .create { height: 32px; gap: 6px; padding: 0 14px 0 10px; align-items: center; border-radius: 16px;
              background-color: var(--elevated); }
    .create:hover { background-color: var(--highlight); }
    .create-text { font-size: 14px; font-weight: 700; }
    .chips { gap: 8px; padding: 0 16px 8px 16px; }
    .chip { height: 32px; padding: 0 12px; justify-content: center; border-radius: 16px; background-color: var(--tinted); }
    .chip:hover { background-color: var(--tinted-hover); }
    .chip-on { background-color: var(--text); }
    .chip-on:hover { background-color: var(--text); }
    .chip-text { font-size: 14px; }
    .chip-text-on { color: var(--base); }
    .tools { height: 40px; padding: 0 16px; align-items: center; justify-content: space-between; }
    .sort { gap: 6px; align-items: center; }
    .sort-text { font-size: 14px; color: var(--subdued); }
    .scroll { flex: 1; }
    .list { padding: 0 8px 8px 8px; }
    .item { flex-direction: row; height: 64px; gap: 12px; padding: 8px; align-items: center; border-radius: 6px; }
    .item:hover { background-color: var(--tinted); }
    .item-on { background-color: var(--tinted-hover); }
    .item-on:hover { background-color: var(--tinted-hover); }
    .cover { width: 48px; height: 48px; border-radius: 4px; }
    .round { border-radius: 24px; }
    .liked-cover { width: 48px; height: 48px; border-radius: 4px; background-color: #5038A0; align-items: center;
                   justify-content: center; }
    .meta { flex: 1; gap: 2px; }
    .name { font-size: 16px; }
    .playing { color: var(--green); }
    .sub { gap: 6px; align-items: center; }
    .pin { color: var(--green); }
    .subtitle { font-size: 14px; color: var(--subdued); }
</style>
