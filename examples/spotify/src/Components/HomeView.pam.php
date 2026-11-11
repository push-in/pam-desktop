<?php

declare(strict_types=1);

namespace App\Components;

use App\Domain\Playlist;
use App\Services\Library;
use App\Services\Player;
use Pam\Desktop\Component;

final class HomeView extends Component
{
    /** @var list<string> */
    public array $chips = ['All', 'Music', 'Podcasts'];

    public function __construct(public Library $library, public Player $player, public int $revision = 0)
    {
    }

    /** @return list<list<Playlist>> the quick picks, four to a row */
    public function quickRows(): array
    {
        return array_chunk($this->library->recents(), 4);
    }

    public function open(string $id): void
    {
        $this->emit('open', $id);
    }

    public function play(string $id): void
    {
        $this->emit('play', $id);
    }

    public function playingIn(Playlist $playlist): bool
    {
        return $this->player->contextId === $playlist->id && $this->player->playing();
    }
}
?>

<template>
    <ScrollView class="scroll" scrollbarWidth="thin">
        <Column class="page">
            <Row class="chips">
                <View p-for="$chip in $chips" p-key="$chip" :class="$chip === 'All' ? 'chip chip-on' : 'chip'">
                    <Text :class="$chip === 'All' ? 'chip-text chip-text-on' : 'chip-text'">{{ $chip }}</Text>
                </View>
            </Row>

            <Row p-for="$row in quickRows()" p-key="$row[0]->id" class="quick-row">
                <Pressable p-for="$item in $row" p-key="$item->id" class="quick" cursor="pointer" on:press="open($item->id)">
                    <View p-if="$item->id === 'liked'" class="quick-liked">
                        <Icon name="heart" font="Lucide" size="18" class="white" />
                    </View>
                    <Image p-else :source="$item->cover" class="quick-cover" />
                    <Text class="quick-name" numberOfLines="2">{{ $item->name }}</Text>
                    <Pressable :class="playingIn($item) ? 'quick-play shown' : 'quick-play'" cursor="pointer"
                               on:press="play($item->id)" :tooltip="'Play ' . $item->name">
                        <Icon :name="playingIn($item) ? 'pause' : 'play'" font="Lucide" size="16" class="black" />
                    </Pressable>
                </Pressable>
            </Row>

            <Column p-for="$shelf in $library->shelves()" p-key="$shelf['title']" class="shelf">
                <Row class="shelf-head">
                    <Text class="shelf-title">{{ $shelf['title'] }}</Text>
                    <Text class="show-all">Show all</Text>
                </Row>
                <Row class="cards">
                    <Pressable p-for="$item in $shelf['items']" p-key="$item->id" class="card" cursor="pointer"
                               on:press="open($item->id)">
                        <View class="art">
                            <Image :source="$item->cover" :class="$item->isArtist() ? 'card-cover round' : 'card-cover'" />
                            <Pressable :class="playingIn($item) ? 'card-play shown' : 'card-play'" cursor="pointer"
                                       on:press="play($item->id)" :tooltip="'Play ' . $item->name">
                                <Icon :name="playingIn($item) ? 'pause' : 'play'" font="Lucide" size="20" class="black" />
                            </Pressable>
                        </View>
                        <Text class="card-name" numberOfLines="1">{{ $item->name }}</Text>
                        <Text class="card-text" numberOfLines="2">{{ $item->isArtist() ? 'Artist' : $item->description }}</Text>
                    </Pressable>
                </Row>
            </Column>
        </Column>
    </ScrollView>
</template>

<style scoped>
    .scroll { flex: 1; }
    .page { padding: 16px 24px 32px 24px; gap: 8px; }
    .chips { gap: 8px; padding-bottom: 16px; }
    .chip { height: 32px; padding: 0 12px; justify-content: center; border-radius: 16px; background-color: var(--tinted); }
    .chip-on { background-color: var(--text); }
    .chip-text { font-size: 14px; }
    .chip-text-on { color: var(--base); }

    .quick-row { gap: 8px; }
    .quick { flex: 1; flex-direction: row; height: 48px; gap: 8px; align-items: center; border-radius: 4px;
             background-color: var(--tinted); }
    .quick { transition: background-color 200ms; }
    .quick:hover { background-color: var(--tinted-hover); }
    .quick-cover { width: 48px; height: 48px; border-top-left-radius: 4px; border-bottom-left-radius: 4px; }
    .quick-liked { width: 48px; height: 48px; border-top-left-radius: 4px; border-bottom-left-radius: 4px;
                   background-color: #5038A0; align-items: center; justify-content: center; }
    .quick-name { flex: 1; font-size: 14px; font-weight: 700; }
    .quick-play { width: 32px; height: 32px; margin-right: 8px; border-radius: 16px; background-color: var(--green);
                  align-items: center; justify-content: center; opacity: 0; elevation: 4; transition: opacity 200ms; }
    .quick:hover .quick-play { opacity: 1; }
    .quick-play:hover { transform: scale(1.04); background-color: var(--green-hover); }

    .shelf { gap: 8px; padding-top: 24px; }
    .shelf-head { align-items: baseline; justify-content: space-between; padding: 0 12px; }
    .shelf-title { font-size: 24px; font-weight: 700; }
    .show-all { font-size: 14px; font-weight: 700; color: var(--subdued); }
    .cards { overflow: hidden; }
    .card { width: 180px; gap: 4px; padding: 12px; border-radius: 6px; transition: background-color 300ms; }
    .card:hover { background-color: var(--elevated); }
    .art { width: 156px; height: 156px; margin-bottom: 4px; }
    .card-cover { width: 156px; height: 156px; border-radius: 6px; }
    .round { border-radius: 78px; }
    .card-play { position: absolute; right: 8px; bottom: 8px; width: 48px; height: 48px; border-radius: 24px;
                 background-color: var(--green); align-items: center; justify-content: center; opacity: 0;
                 transform: translateY(8px); elevation: 6; transition: opacity 250ms, transform 250ms ease-out; }
    .card:hover .card-play { opacity: 1; transform: translateY(0px); }
    .card-play:hover { transform: scale(1.04); background-color: var(--green-hover); }
    .shown { opacity: 1; transform: translateY(0px); }
    .card-name { font-size: 16px; font-weight: 500; }
    .card-text { font-size: 14px; line-height: 20px; color: var(--subdued); }
    .white { color: var(--text); }
    .black { color: var(--base); }
</style>
