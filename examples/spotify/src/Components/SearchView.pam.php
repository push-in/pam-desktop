<?php

declare(strict_types=1);

namespace App\Components;

use App\Domain\Playlist;
use App\Domain\Track;
use App\Services\Library;
use App\Services\Player;
use Pam\Desktop\Component;

final class SearchView extends Component
{
    /** @var list<array{name: string, color: string}> */
    public array $genres = [
        ['name' => 'Music', 'color' => '#DC148C'], ['name' => 'Podcasts', 'color' => '#006450'],
        ['name' => 'Live Events', 'color' => '#8400E7'], ['name' => 'Made For You', 'color' => '#1E3264'],
        ['name' => 'New Releases', 'color' => '#608108'], ['name' => 'Pop', 'color' => '#477D95'],
        ['name' => 'Hip-Hop', 'color' => '#BA5D07'], ['name' => 'Rock', 'color' => '#E91429'],
        ['name' => 'Latin', 'color' => '#E1118C'], ['name' => 'Mood', 'color' => '#E1118C'],
        ['name' => 'Chill', 'color' => '#D84000'], ['name' => 'Focus', 'color' => '#503750'],
    ];

    public function __construct(
        public Library $library,
        public Player $player,
        public string $query = '',
        public int $revision = 0,
    ) {
    }

    /** @return list<list<array{name: string, color: string}>> */
    public function genreRows(): array
    {
        return array_chunk($this->genres, 4);
    }

    /** @return array{tracks: list<Track>, playlists: list<Playlist>} */
    public function results(): array
    {
        return $this->library->search($this->query);
    }

    public function top(): ?Playlist
    {
        return $this->results()['playlists'][0] ?? null;
    }

    /** @return list<Track> */
    public function songs(): array
    {
        return array_slice($this->results()['tracks'], 0, 4);
    }

    /** The playlist a found song plays from: the first that holds it. */
    public function playSong(string $trackId): void
    {
        foreach (['liked', 'discover', 'daily1', 'daily2', 'release', 'chill', 'focus', 'lofi', 'jazz', 'roadtrip', 'workout', 'sleep', 'throwback', 'soul'] as $id) {
            if (in_array($trackId, $this->library->playlist($id)->trackIds, true)) {
                $this->emit('play', ['id' => $id, 'track' => $trackId]);

                return;
            }
        }
    }

    public function open(string $id): void
    {
        $this->emit('open', $id);
    }
}
?>

<template>
    <ScrollView class="scroll" scrollbarWidth="thin">
        <Column class="page">
            <Column p-if="trim($query) === ''" class="browse">
                <Text class="heading">Browse all</Text>
                <Row p-for="$row in genreRows()" p-key="$row[0]['name']" class="genre-row">
                    <View p-for="$genre in $row" p-key="$genre['name']" class="genre" :backgroundColor="$genre['color']">
                        <Text class="genre-name">{{ $genre['name'] }}</Text>
                    </View>
                </Row>
            </Column>

            <Text p-else-if="top() === null && songs() === []" class="empty">No results found for “{{ $query }}”</Text>

            <Row p-else class="results">
                <Column p-if="top() !== null" class="top-column">
                    <Text class="heading">Top result</Text>
                    <Pressable class="top" cursor="pointer" on:press="open(top()->id)">
                        <Image :source="top()->cover" :class="top()->isArtist() ? 'top-cover round' : 'top-cover'" />
                        <Text class="top-name" numberOfLines="1">{{ top()->name }}</Text>
                        <Text class="top-kind">{{ top()->subtitle() }}</Text>
                    </Pressable>
                </Column>
                <Column p-if="songs() !== []" class="songs-column">
                    <Text class="heading">Songs</Text>
                    <Pressable p-for="$song in songs()" p-key="$song->id" class="song" cursor="pointer"
                               on:press="playSong($song->id)">
                        <Image :source="$song->cover" class="song-cover" />
                        <Column class="song-titles">
                            <Text :class="$player->isCurrent($song->id) ? 'song-title green' : 'song-title'" numberOfLines="1">{{ $song->title }}</Text>
                            <Text class="song-artist" numberOfLines="1">{{ $song->artist }}</Text>
                        </Column>
                        <Text class="song-time">{{ $song->duration() }}</Text>
                    </Pressable>
                </Column>
            </Row>
        </Column>
    </ScrollView>
</template>

<style scoped>
    .scroll { flex: 1; }
    .page { padding: 24px; }
    .heading { font-size: 24px; font-weight: 700; margin-bottom: 16px; }
    .browse { gap: 0; }
    .genre-row { gap: 24px; margin-bottom: 24px; }
    .genre { flex: 1; height: 120px; padding: 16px; border-radius: 8px; overflow: hidden; }
    .genre-name { font-size: 24px; font-weight: 700; }
    .empty { font-size: 20px; font-weight: 700; padding-top: 64px; text-align: center; }
    .results { gap: 24px; }
    .top-column { width: 420px; }
    .top { gap: 8px; padding: 20px; border-radius: 8px; background-color: var(--tinted); }
    .top:hover { background-color: var(--tinted-hover); }
    .top-cover { width: 92px; height: 92px; border-radius: 4px; margin-bottom: 12px; }
    .round { border-radius: 46px; }
    .top-name { font-size: 32px; font-weight: 700; }
    .top-kind { font-size: 14px; color: var(--subdued); }
    .songs-column { flex: 1; }
    .song { flex-direction: row; height: 56px; gap: 12px; padding: 0 8px; align-items: center; border-radius: 4px; }
    .song:hover { background-color: var(--tinted); }
    .song-cover { width: 40px; height: 40px; border-radius: 4px; }
    .song-titles { flex: 1; gap: 2px; }
    .song-title { font-size: 16px; }
    .song-artist { font-size: 14px; color: var(--subdued); }
    .song-time { font-size: 14px; color: var(--subdued); }
    .green { color: var(--green); }
</style>
