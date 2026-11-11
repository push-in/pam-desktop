<?php

declare(strict_types=1);

namespace App\Services;

use App\Domain\Playlist;
use App\Domain\Track;

/**
 * What is playing. The position comes from the clock (where playback
 * started and when), so nothing has to tick while the window is hidden; the
 * bar only asks for it to draw.
 */
final class Player
{
    /** @var list<string> */
    private array $queue = [];

    private int $index = 0;

    private float $offset = 0.0;

    private ?float $startedAt = null;

    public ?string $contextId = null;

    public bool $shuffle = false;

    /** off, all or one */
    public string $repeat = 'off';

    public float $volume = 0.7;

    public bool $muted = false;

    /** @var array<string, true> */
    private array $liked = [];

    public function __construct(private readonly Library $library, private readonly \Closure $now)
    {
        foreach ($library->playlist('liked')->trackIds as $id) {
            $this->liked[$id] = true;
        }
        $this->queue = $library->playlist('discover')->trackIds;
        $this->contextId = 'discover';
        $this->offset = 74.0;
    }

    public static function make(Library $library): self
    {
        return new self($library, static fn (): float => microtime(true));
    }

    public function track(): ?Track
    {
        $id = $this->queue[$this->index] ?? null;

        return $id === null ? null : $this->library->track($id);
    }

    public function playing(): bool
    {
        return $this->startedAt !== null;
    }

    public function isCurrent(string $trackId, ?string $contextId = null): bool
    {
        return $this->track()?->id === $trackId && ($contextId === null || $contextId === $this->contextId);
    }

    /** Plays a playlist, from one of its tracks or its first. */
    public function play(Playlist $playlist, ?string $trackId = null): void
    {
        $this->queue = $playlist->trackIds;
        if ($this->shuffle) {
            shuffle($this->queue);
        }
        $this->contextId = $playlist->id;
        $index = $trackId === null ? false : array_search($trackId, $this->queue, true);
        $this->index = $index === false ? 0 : $index;
        $this->offset = 0.0;
        $this->startedAt = ($this->now)();
    }

    public function toggle(): void
    {
        if ($this->playing()) {
            $this->offset = $this->position();
            $this->startedAt = null;
        } else {
            $this->startedAt = ($this->now)();
        }
    }

    public function position(): float
    {
        $track = $this->track();
        if ($track === null) {
            return 0.0;
        }
        $elapsed = $this->startedAt === null ? 0.0 : ($this->now)() - $this->startedAt;

        return min((float) $track->seconds, $this->offset + $elapsed);
    }

    public function seek(float $seconds): void
    {
        $this->offset = max(0.0, $seconds);
        if ($this->playing()) {
            $this->startedAt = ($this->now)();
        }
    }

    public function next(): void
    {
        if ($this->repeat === 'one') {
            $this->seek(0);

            return;
        }
        $last = $this->index >= count($this->queue) - 1;
        if ($last && $this->repeat === 'off') {
            $this->offset = 0.0;
            $this->startedAt = null;
            $this->index = 0;

            return;
        }
        $this->index = $last ? 0 : $this->index + 1;
        $this->seek(0);
    }

    /** Back to the start of the track, or the previous one in its first three seconds. */
    public function previous(): void
    {
        if ($this->position() > 3 || $this->index === 0) {
            $this->seek(0);

            return;
        }
        $this->index--;
        $this->seek(0);
    }

    /** Moves on when the track ran out; true when it did. */
    public function advance(): bool
    {
        $track = $this->track();
        if ($track === null || !$this->playing() || $this->position() < $track->seconds) {
            return false;
        }
        $this->next();

        return true;
    }

    public function cycleRepeat(): void
    {
        $this->repeat = match ($this->repeat) {
            'off' => 'all',
            'all' => 'one',
            default => 'off',
        };
    }

    public function isLiked(string $trackId): bool
    {
        return isset($this->liked[$trackId]);
    }

    public function toggleLike(string $trackId): void
    {
        if (isset($this->liked[$trackId])) {
            unset($this->liked[$trackId]);
        } else {
            $this->liked[$trackId] = true;
        }
    }

    public function setVolume(float $volume): void
    {
        $this->volume = max(0.0, min(1.0, $volume));
        $this->muted = $this->volume === 0.0;
    }

    public function audible(): float
    {
        return $this->muted ? 0.0 : $this->volume;
    }

    public function volumeIcon(): string
    {
        $volume = $this->audible();

        return match (true) {
            $volume === 0.0 => 'volume-x',
            $volume < 0.34 => 'volume',
            $volume < 0.67 => 'volume-1',
            default => 'volume-2',
        };
    }
}
