<?php

declare(strict_types=1);

namespace App\Components;

use App\Domain\Playlist;
use App\Domain\Track;
use App\Services\Library;
use App\Services\Player;
use Pam\Desktop\Component;
use Pam\Desktop\MenuItem;
use Pam\Desktop\System\Clipboard;

final class PlaylistView extends Component
{
    public function __construct(
        public Library $library,
        public Player $player,
        public string $playlistId,
        public int $revision = 0,
    ) {
    }

    public function playlist(): Playlist
    {
        return $this->library->playlist($this->playlistId);
    }

    /** @return list<Track> */
    public function tracks(): array
    {
        return $this->library->tracks($this->playlist());
    }

    /** "8 songs, about 30 min" */
    public function summary(): string
    {
        $count = count($this->playlist()->trackIds);
        $minutes = (int) round($this->library->totalSeconds($this->playlist()) / 60);

        return ($count === 1 ? '1 song' : "{$count} songs").", about {$minutes} min";
    }

    public function playingHere(): bool
    {
        return $this->player->contextId === $this->playlistId && $this->player->playing();
    }

    public function current(Track $track): bool
    {
        return $this->player->isCurrent($track->id, $this->playlistId);
    }

    public function play(): void
    {
        $this->emit('play', ['id' => $this->playlistId]);
    }

    public function playTrack(string $trackId): void
    {
        $this->emit('play', ['id' => $this->playlistId, 'track' => $trackId]);
    }

    /** @return list<MenuItem> what a secondary click on a track offers */
    public function trackMenu(Track $track): array
    {
        $liked = $this->player->isLiked($track->id);

        return [
            MenuItem::make('Add to queue', fn () => $this->playTrack($track->id)),
            MenuItem::separator(),
            MenuItem::make('Go to artist', null),
            MenuItem::make('Go to album', null),
            MenuItem::make('View credits', null),
            MenuItem::separator(),
            MenuItem::make($liked ? 'Remove from your Liked Songs' : 'Save to your Liked Songs', fn () => $this->like($track->id)),
            MenuItem::submenu(
                'Share',
                MenuItem::make('Copy Song Link', static fn () => Clipboard::setText('https://open.spotify.com/track/'.$track->id)),
                MenuItem::make('Embed track', null),
            ),
        ];
    }

    public function like(string $trackId): void
    {
        $this->player->toggleLike($trackId);
        $this->emit('changed');
    }
}
?>

<template>
    <ScrollView class="scroll" scrollbarWidth="thin">
        <Column class="page">
            <Row class="hero" :backgroundColor="playlist()->tint">
                <View p-if="$playlistId === 'liked'" class="hero-liked">
                    <Icon name="heart" font="Lucide" size="72" class="white" />
                </View>
                <Image p-else :source="playlist()->cover" :class="playlist()->isArtist() ? 'hero-cover round' : 'hero-cover'" />
                <Column class="hero-text">
                    <Text class="kind">{{ playlist()->isArtist() ? 'Verified Artist' : playlist()->kind->value }}</Text>
                    <Text class="name" numberOfLines="1">{{ playlist()->name }}</Text>
                    <Text p-if="!playlist()->isArtist()" class="description" numberOfLines="2">{{ playlist()->description }}</Text>
                    <Text class="byline">
                        <Span class="owner">{{ playlist()->isArtist() ? playlist()->description : playlist()->owner }}</Span>
                        <Span p-if="!playlist()->isArtist()" class="stats"> • {{ summary() }}</Span>
                    </Text>
                </Column>
            </Row>

            <Column class="body">
                <Row class="actions">
                    <Pressable class="big-play" cursor="pointer" @press="play" :tooltip="playingHere() ? 'Pause' : 'Play'">
                        <Icon :name="playingHere() ? 'pause' : 'play'" font="Lucide" size="24" class="black" />
                    </Pressable>
                    <Icon name="shuffle" font="Lucide" size="28" :class="$player->shuffle ? 'green' : 'subdued'" />
                    <Icon name="circle-plus" font="Lucide" size="28" class="subdued" />
                    <Icon name="circle-arrow-down" font="Lucide" size="28" class="subdued" />
                    <Icon name="ellipsis" font="Lucide" size="28" class="subdued" />
                    <View class="spacer" />
                    <Text class="list-mode">List</Text>
                    <Icon name="list" font="Lucide" size="16" class="subdued" />
                </Row>

                <Row class="columns">
                    <Text class="col-number">#</Text>
                    <Text class="col-title">Title</Text>
                    <Text class="col-album">Album</Text>
                    <Icon name="clock-3" font="Lucide" size="16" class="col-time subdued" />
                </Row>

                <Pressable p-for="$track in tracks()" p-key="$track->id" :class="current($track) ? 'row row-on' : 'row'"
                           on:doubleTap="playTrack($track->id)" doubleTapDelay="300" :contextMenu="trackMenu($track)">
                    <View class="number">
                        <Text :class="current($track) ? 'index index-on' : 'index'">{{ $trackIndex + 1 }}</Text>
                        <Pressable class="row-play" cursor="pointer" on:press="playTrack($track->id)" :tooltip="'Play ' . $track->title">
                            <Icon :name="current($track) && $player->playing() ? 'pause' : 'play'" font="Lucide" size="14" class="white" />
                        </Pressable>
                    </View>
                    <Image :source="$track->cover" class="row-cover" />
                    <Column class="row-titles">
                        <Text :class="current($track) ? 'row-title green' : 'row-title'" numberOfLines="1">{{ $track->title }}</Text>
                        <Row class="row-artist-line">
                            <Text p-if="$track->explicit" class="explicit">E</Text>
                            <Text class="row-artist" numberOfLines="1">{{ $track->artist }}</Text>
                        </Row>
                    </Column>
                    <Text class="row-album" numberOfLines="1">{{ $track->album }}</Text>
                    <Row class="row-end">
                        <Pressable class="row-like" cursor="pointer" on:press="like($track->id)"
                                   :tooltip="$player->isLiked($track->id) ? 'Remove from Liked Songs' : 'Add to Liked Songs'">
                            <Icon p-if="$player->isLiked($track->id)" name="circle-check" font="Lucide" size="16" class="green" />
                            <Icon p-else name="circle-plus" font="Lucide" size="16" class="subdued" />
                        </Pressable>
                        <Text class="row-time">{{ $track->duration() }}</Text>
                    </Row>
                </Pressable>
            </Column>
        </Column>
    </ScrollView>
</template>

<style scoped>
    .scroll { flex: 1; }
    .page { min-height: 100%; background-color: var(--panel); }
    .hero { height: 300px; gap: 24px; padding: 24px; align-items: flex-end; }
    .hero-cover { width: 232px; height: 232px; border-radius: 4px; elevation: 12; }
    .hero-liked { width: 232px; height: 232px; border-radius: 4px; background-color: #5038A0; align-items: center;
                  justify-content: center; elevation: 12; }
    .round { border-radius: 116px; }
    .hero-text { flex: 1; gap: 8px; }
    .kind { font-size: 14px; font-weight: 500; }
    .name { font-size: 72px; line-height: 80px; font-weight: 900; }
    .description { font-size: 14px; color: #FFFFFFB3; }
    .byline { font-size: 14px; }
    .owner { font-weight: 700; }
    .stats { color: #FFFFFFB3; }

    .body { padding: 0 24px 32px 24px; background-color: #00000033; }
    .actions { height: 104px; gap: 32px; align-items: center; }
    .big-play { width: 56px; height: 56px; border-radius: 28px; background-color: var(--green); align-items: center;
                justify-content: center; }
    .big-play:hover { transform: scale(1.04); background-color: var(--green-hover); }
    .spacer { flex: 1; }
    .list-mode { font-size: 14px; color: var(--subdued); margin-right: -24px; }

    .columns { height: 36px; gap: 16px; padding: 0 16px; align-items: center; border-bottom-width: 1px;
               border-color: var(--divider); margin-bottom: 16px; }
    .col-number { width: 16px; font-size: 16px; color: var(--subdued); text-align: right; }
    .col-title { flex: 4; font-size: 14px; color: var(--subdued); }
    .col-album { flex: 3; font-size: 14px; color: var(--subdued); }
    .col-time { width: 112px; text-align: right; }

    .row { flex-direction: row; height: 56px; gap: 16px; padding: 0 16px; align-items: center; border-radius: 4px; transition: background-color 150ms; }
    .row:hover { background-color: var(--tinted); }
    .row-on { background-color: var(--tinted-hover); }
    .number { width: 16px; height: 16px; align-items: center; justify-content: center; }
    .index { font-size: 16px; color: var(--subdued); text-align: right; transition: opacity 120ms; }
    .index-on { color: var(--green); }
    .row:hover .index { opacity: 0; }
    .row-play { position: absolute; width: 16px; height: 16px; align-items: center; justify-content: center; opacity: 0; transition: opacity 120ms; }
    .row:hover .row-play { opacity: 1; }
    .row-cover { width: 40px; height: 40px; border-radius: 4px; }
    .row-titles { flex: 4; margin-left: -4px; gap: 2px; }
    .row-title { font-size: 16px; }
    .row-artist-line { gap: 6px; align-items: center; }
    .explicit { width: 16px; height: 16px; border-radius: 2px; background-color: var(--subdued); font-size: 10px;
                font-weight: 700; color: var(--base); text-align: center; line-height: 16px; }
    .row-artist { font-size: 14px; color: var(--subdued); }
    .row:hover .row-artist { color: var(--text); }
    .row-album { flex: 3; font-size: 14px; color: var(--subdued); }
    .row-end { width: 112px; gap: 16px; align-items: center; justify-content: flex-end; }
    .row-like { width: 16px; height: 16px; }
    .row-time { min-width: 40px; font-size: 14px; color: var(--subdued); text-align: right; }
    .white { color: var(--text); }
    .black { color: var(--base); }
    .green { color: var(--green); }
    .subdued { color: var(--subdued); }
</style>
