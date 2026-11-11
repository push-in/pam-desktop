<?php

declare(strict_types=1);

namespace App\Domain;

/** A playlist, an album or an artist: something the library lists and the player plays. */
final readonly class Playlist
{
    /** @param list<string> $trackIds */
    public function __construct(
        public string $id,
        public string $name,
        public PlaylistKind $kind,
        public string $cover,
        public string $owner,
        public string $description,
        public string $tint,
        public array $trackIds,
        public bool $pinned = false,
    ) {
    }

    public function isArtist(): bool
    {
        return $this->kind === PlaylistKind::Artist;
    }

    /** "Playlist • Spotify", as the library subtitles it. */
    public function subtitle(): string
    {
        return $this->isArtist() ? 'Artist' : $this->kind->value.' • '.$this->owner;
    }
}
