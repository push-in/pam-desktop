<?php

declare(strict_types=1);

namespace App\Domain;

final readonly class Track
{
    public function __construct(
        public string $id,
        public string $title,
        public string $artist,
        public string $album,
        public int $seconds,
        public string $cover,
        public bool $explicit = false,
    ) {
    }

    public function duration(): string
    {
        return self::clock($this->seconds);
    }

    public static function clock(int|float $seconds): string
    {
        $seconds = max(0, (int) $seconds);

        return intdiv($seconds, 60).':'.str_pad((string) ($seconds % 60), 2, '0', STR_PAD_LEFT);
    }
}
