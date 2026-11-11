<?php

declare(strict_types=1);

namespace App\Services;

use App\Domain\Playlist;
use App\Domain\PlaylistKind;
use App\Domain\Track;

/** The catalogue the clone plays: fictional artists, real-looking shelves. */
final class Library
{
    public const string USER = 'David';

    /** @var array<string, Track> */
    private array $tracks = [];

    /** @var array<string, Playlist> */
    private array $playlists = [];

    public function __construct()
    {
        $songs = [
            ['Neon Harbor', 'The Midnight Arcade', 'Afterglow City', 214, 'daily1'],
            ['Paper Satellites', 'Lumen Fields', 'Orbit Season', 198, 'daily2'],
            ['Slow Motion Summer', 'Coral Static', 'Tidal Pop', 187, 'daily3'],
            ['Glass Houses', 'Vera Nox', 'Night Swim', 241, 'discover', true],
            ['Velvet Engine', 'The Midnight Arcade', 'Afterglow City', 203, 'daily1'],
            ['Low Tide Radio', 'Coral Static', 'Tidal Pop', 176, 'daily3'],
            ['Golden Hour Again', 'Mira Sol', 'Sunday Records', 229, 'release'],
            ['Fireflies in Traffic', 'Lumen Fields', 'Orbit Season', 255, 'daily2'],
            ['Hollow Moon', 'Vera Nox', 'Night Swim', 199, 'discover'],
            ['Coffee & Rain', 'Ellis Park Trio', 'Blue Corner', 312, 'coffee'],
            ['Morning Pages', 'Ellis Park Trio', 'Blue Corner', 268, 'jazz'],
            ['Quiet Streets', 'Hana Okabe', 'Still Water', 154, 'piano'],
            ['Paper Boats', 'Hana Okabe', 'Still Water', 171, 'piano'],
            ['Tape Hiss Lullaby', 'loft kid', 'bedroom tapes', 132, 'lofi'],
            ['Window Seat', 'loft kid', 'bedroom tapes', 148, 'lofi'],
            ['Deep Work', 'Ambient Office', 'Focus Fields', 360, 'focus'],
            ['Signal Fire', 'Black Pines', 'Northern Roads', 233, 'rock', true],
            ['Highway Ghosts', 'Black Pines', 'Northern Roads', 276, 'roadtrip'],
            ['Sugar Static', 'Pixie Riot', 'Bubblegum Thunder', 182, 'pop'],
            ['Heartbeat Machine', 'Pixie Riot', 'Bubblegum Thunder', 195, 'pop'],
            ['Cassette Kids', 'The Rewinds', 'Side B', 207, 'throwback'],
            ['Run It Back', 'Kilo Pulse', 'Max Reps', 188, 'workout', true],
            ['Dreamless', 'Soft Weather', 'Night Light', 302, 'sleep'],
            ['Velours', 'Amaro Viana', 'Saudade Nova', 221, 'latin'],
            ['Sunday Soul', 'June & The Keys', 'Warm Room', 246, 'soul'],
            ['Indigo Bloom', 'Lumen Fields', 'Orbit Season', 238, 'indie'],
            ['Chill Circuit', 'Coral Static', 'Tidal Pop', 201, 'chill'],
            ['Last Train Home', 'Vera Nox', 'Night Swim', 263, 'discover'],
        ];
        foreach ($songs as $index => $song) {
            [$title, $artist, $album, $seconds, $cover] = $song;
            $id = 't'.($index + 1);
            $this->tracks[$id] = new Track($id, $title, $artist, $album, $seconds, self::cover($cover), $song[5] ?? false);
        }
        $pick = static fn (int ...$numbers): array => array_map(static fn (int $number): string => 't'.$number, $numbers);
        foreach ([
            ['liked', 'Liked Songs', PlaylistKind::Playlist, 'liked', self::USER, 'Your favourites, all in one place.', '#5038A0', $pick(4, 1, 7, 10, 19, 12, 25, 3), true],
            ['daily1', 'Daily Mix 1', PlaylistKind::Playlist, 'daily1', 'Spotify', 'The Midnight Arcade, Vera Nox, Lumen Fields and more', '#3F6E8C', $pick(1, 5, 4, 9, 8, 28, 26, 2)],
            ['discover', 'Discover Weekly', PlaylistKind::Playlist, 'discover', 'Spotify', 'Your weekly mixtape of fresh music. Enjoy new music and deep cuts picked for you.', '#2E5F4F', $pick(9, 4, 28, 7, 24, 26, 17, 13, 3, 25)],
            ['release', 'Release Radar', PlaylistKind::Playlist, 'release', 'Spotify', 'Catch all the latest music from artists you follow.', '#8C4A2F', $pick(7, 19, 22, 26, 2, 24)],
            ['daily2', 'Daily Mix 2', PlaylistKind::Playlist, 'daily2', 'Spotify', 'Lumen Fields, Coral Static, Mira Sol and more', '#6B4C8C', $pick(2, 8, 26, 6, 3, 27, 7)],
            ['chill', 'Chill Vibes', PlaylistKind::Playlist, 'chill', 'Spotify', 'Kick back to the best new and recent chill hits.', '#4C6B8C', $pick(27, 6, 3, 14, 15, 12)],
            ['focus', 'Deep Focus', PlaylistKind::Playlist, 'focus', 'Spotify', 'Keep calm and focus with ambient and post-rock music.', '#3A4F6E', $pick(16, 12, 13, 23, 14)],
            ['lofi', 'lofi beats', PlaylistKind::Playlist, 'lofi', 'Spotify', 'Chill beats, lofi vibes, new tracks every week.', '#7A5A3E', $pick(14, 15, 10, 13, 16)],
            ['jazz', 'Jazz Classics', PlaylistKind::Playlist, 'jazz', 'Spotify', 'The definitive jazz records, from swing to modal.', '#8C6A2F', $pick(11, 10, 25)],
            ['afterglow', 'Afterglow City', PlaylistKind::Album, 'daily1', 'The Midnight Arcade', 'Album • 2024', '#3F6E8C', $pick(1, 5)],
            ['arcade', 'The Midnight Arcade', PlaylistKind::Artist, 'artist1', 'Artist', '2,418,302 monthly listeners', '#5A3F6E', $pick(1, 5)],
            ['vera', 'Vera Nox', PlaylistKind::Artist, 'artist2', 'Artist', '1,103,555 monthly listeners', '#2F4F6E', $pick(4, 9, 28)],
            ['lumen', 'Lumen Fields', PlaylistKind::Artist, 'artist3', 'Artist', '884,019 monthly listeners', '#3E6E5A', $pick(2, 8, 26)],
            ['roadtrip', 'Road Trip', PlaylistKind::Playlist, 'roadtrip', self::USER, 'Windows down.', '#8C3F3F', $pick(18, 17, 21, 19, 20)],
            ['workout', 'Beast Mode', PlaylistKind::Playlist, 'workout', 'Spotify', 'Get your beast mode on!', '#8C2F4A', $pick(22, 17, 20, 19)],
            ['sleep', 'Sleep', PlaylistKind::Playlist, 'sleep', 'Spotify', 'Gentle ambient piano to help you fall asleep.', '#2F3F5A', $pick(23, 12, 13, 16)],
            ['throwback', 'Throwback Thursday', PlaylistKind::Playlist, 'throwback', 'Spotify', 'Nostalgia never sounded so good.', '#6E5A2F', $pick(21, 18, 20, 5)],
            ['soul', 'Soul Lounge', PlaylistKind::Playlist, 'soul', 'Spotify', 'Smooth soul for slow evenings.', '#6E3F2F', $pick(25, 24, 11, 10)],
        ] as $row) {
            [$id, $name, $kind, $cover, $owner, $description, $tint, $trackIds] = $row;
            $this->playlists[$id] = new Playlist($id, $name, $kind, self::cover($cover), $owner, $description, $tint, $trackIds, $row[8] ?? false);
        }
    }

    public function track(string $id): Track
    {
        return $this->tracks[$id];
    }

    public function playlist(string $id): Playlist
    {
        return $this->playlists[$id];
    }

    /** @return list<Playlist> the library panel, pinned first */
    public function saved(?PlaylistKind $kind = null): array
    {
        $ids = ['liked', 'daily1', 'discover', 'roadtrip', 'release', 'afterglow', 'arcade', 'chill', 'vera', 'focus', 'lumen', 'lofi', 'jazz'];

        return array_values(array_filter(
            array_map(fn (string $id): Playlist => $this->playlists[$id], $ids),
            static fn (Playlist $playlist): bool => $kind === null || $playlist->kind === $kind,
        ));
    }

    /** @return list<Playlist> the eight quick picks at the top of Home */
    public function recents(): array
    {
        return array_map(fn (string $id): Playlist => $this->playlists[$id], ['liked', 'daily1', 'discover', 'chill', 'arcade', 'focus', 'release', 'lofi']);
    }

    /** @return list<array{title: string, items: list<Playlist>}> */
    public function shelves(): array
    {
        $shelf = fn (string $title, string ...$ids): array => [
            'title' => $title,
            'items' => array_map(fn (string $id): Playlist => $this->playlists[$id], $ids),
        ];

        return [
            $shelf('Made For '.self::USER, 'daily1', 'daily2', 'discover', 'release', 'chill', 'focus'),
            $shelf('Your favorite artists', 'arcade', 'vera', 'lumen'),
            $shelf('Jump back in', 'roadtrip', 'workout', 'sleep', 'throwback', 'soul', 'jazz'),
        ];
    }

    /** @return list<Track> */
    public function tracks(Playlist $playlist): array
    {
        return array_map(fn (string $id): Track => $this->tracks[$id], $playlist->trackIds);
    }

    public function totalSeconds(Playlist $playlist): int
    {
        return array_sum(array_map(fn (string $id): int => $this->tracks[$id]->seconds, $playlist->trackIds));
    }

    /**
     * Tracks, then playlists and artists whose name matches.
     *
     * @return array{tracks: list<Track>, playlists: list<Playlist>}
     */
    public function search(string $query): array
    {
        $query = trim($query);
        if ($query === '') {
            return ['tracks' => [], 'playlists' => []];
        }
        $matches = static fn (string ...$fields): bool => array_any($fields, static fn (string $field): bool => mb_stripos($field, $query) !== false);

        return [
            'tracks' => array_values(array_filter($this->tracks, static fn (Track $track): bool => $matches($track->title, $track->artist, $track->album))),
            'playlists' => array_values(array_filter($this->playlists, static fn (Playlist $playlist): bool => $matches($playlist->name, $playlist->owner))),
        ];
    }

    private static function cover(string $name): string
    {
        return "asset://assets/covers/{$name}.jpg";
    }
}
