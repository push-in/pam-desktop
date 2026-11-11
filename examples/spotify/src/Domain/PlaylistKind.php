<?php

declare(strict_types=1);

namespace App\Domain;

enum PlaylistKind: string
{
    case Playlist = 'Playlist';
    case Album = 'Album';
    case Artist = 'Artist';
}
