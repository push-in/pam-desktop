<?php

declare(strict_types=1);

namespace App\Components;

use App\Domain\Track;
use App\Services\Player;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\System\TimerHandle;
use Pam\Desktop\System\Timers;
use Pam\DesktopUi\Slider;

/**
 * The bar at the bottom: the track, the transport and the volume. It ticks
 * once a second only while something plays and the window is visible.
 */
final class NowPlayingBar extends Component
{
    #[State]
    public int $tick = 0;

    public Slider $progress;

    public Slider $level;

    private ?TimerHandle $timer = null;

    private bool $hidden = false;

    public function __construct(public Player $player, public int $revision = 0)
    {
    }

    public function mount(): void
    {
        $this->progress = $this->progressSlider();
        $this->level = new Slider(
            $this->player->audible(),
            fn (float $volume) => $this->volumeTo($volume),
            label: 'Change volume',
        );
        $this->sync();
    }

    public function rendering(): void
    {
        $this->sync();
    }

    public function paused(): void
    {
        $this->hidden = true;
        $this->sync();
    }

    public function resumed(): void
    {
        $this->hidden = false;
        $this->sync();
    }

    public function unmount(): void
    {
        $this->timer?->cancel();
        $this->timer = null;
    }

    public function track(): ?Track
    {
        return $this->player->track();
    }

    public function toggle(): void
    {
        $this->player->toggle();
        $this->changed();
    }

    public function next(): void
    {
        $this->player->next();
        $this->changed();
    }

    public function previous(): void
    {
        $this->player->previous();
        $this->changed();
    }

    public function shuffle(): void
    {
        $this->player->shuffle = !$this->player->shuffle;
        $this->tick++;
    }

    public function repeat(): void
    {
        $this->player->cycleRepeat();
        $this->tick++;
    }

    public function like(): void
    {
        if (($track = $this->track()) !== null) {
            $this->player->toggleLike($track->id);
            $this->changed();
        }
    }

    public function mute(): void
    {
        $this->player->muted = !$this->player->muted && $this->player->volume > 0;
        $this->level->value = $this->player->audible();
        $this->tick++;
    }

    public function elapsed(): string
    {
        return Track::clock($this->player->position());
    }

    public function liked(): bool
    {
        return ($track = $this->track()) !== null && $this->player->isLiked($track->id);
    }

    private function seekTo(float $seconds): void
    {
        $this->player->seek($seconds);
        $this->tick++;
    }

    private function volumeTo(float $volume): void
    {
        $this->player->setVolume($volume);
        $this->tick++;
    }

    /** The rest of the screen shows what plays (green rows, the play buttons). */
    private function changed(): void
    {
        $this->tick++;
        $this->emit('changed');
    }

    private function length(): float
    {
        return (float) max(1, $this->player->track()?->seconds ?? 1);
    }

    /** The position slider of the current track: seeking happens on release. */
    private function progressSlider(): Slider
    {
        return new Slider(
            $this->player->position(),
            fn (float $seconds) => $this->tick++,
            max: $this->length(),
            committed: fn (float $seconds) => $this->seekTo($seconds),
            label: 'Change progress',
        );
    }

    /** Keeps the sliders on the player and the timer running only while it is useful. */
    private function sync(): void
    {
        if (!$this->progress->dragging) {
            $this->progress->value = $this->player->position();
        }
        $this->progress->max = $this->length();
        $wanted = $this->player->playing() && !$this->hidden;
        if ($wanted && $this->timer === null) {
            $this->timer = Timers::every(1000, function (): void {
                if ($this->player->advance()) {
                    $this->changed();
                } else {
                    $this->tick++;
                }
            });
        } elseif (!$wanted && $this->timer !== null) {
            $this->timer->cancel();
            $this->timer = null;
        }
    }
}
?>

<template>
    <Row class="bar">
        <Row class="now">
            <Image p-if="track() !== null" :source="track()->cover" class="cover" />
            <Column class="titles">
                <Text class="title" numberOfLines="1">{{ track()?->title ?? '' }}</Text>
                <Text class="artist" numberOfLines="1">{{ track()?->artist ?? '' }}</Text>
            </Column>
            <Pressable class="icon-button" cursor="pointer" @press="like"
                       :tooltip="liked() ? 'Remove from Liked Songs' : 'Save to Your Library'">
                <Icon p-if="liked()" name="circle-check" font="Lucide" size="16" class="liked" />
                <Icon p-else name="circle-plus" font="Lucide" size="16" class="dim" />
            </Pressable>
        </Row>

        <Column class="transport">
            <Row class="controls">
                <Tooltip :title="$player->shuffle ? 'Disable shuffle' : 'Enable shuffle'" shortcut="Ctrl+S" placement="top">
                    <Pressable class="icon-button" cursor="pointer" @press="shuffle">
                        <Icon name="shuffle" font="Lucide" size="16" :class="$player->shuffle ? 'on' : 'dim'" />
                    </Pressable>
                </Tooltip>
                <Tooltip title="Previous" shortcut="Ctrl+Left" placement="top">
                    <Pressable class="icon-button" cursor="pointer" @press="previous">
                        <Icon name="skip-back" font="Lucide" size="16" class="dim" />
                    </Pressable>
                </Tooltip>
                <Tooltip :title="$player->playing() ? 'Pause' : 'Play'" shortcut="Space" placement="top">
                    <Pressable class="play" cursor="pointer" @press="toggle">
                        <Icon :name="$player->playing() ? 'pause' : 'play'" font="Lucide" size="16" class="play-icon" />
                    </Pressable>
                </Tooltip>
                <Tooltip title="Next" shortcut="Ctrl+Right" placement="top">
                    <Pressable class="icon-button" cursor="pointer" @press="next">
                        <Icon name="skip-forward" font="Lucide" size="16" class="dim" />
                    </Pressable>
                </Tooltip>
                <Tooltip :title="$player->repeat === 'off' ? 'Enable repeat' : ($player->repeat === 'all' ? 'Enable repeat one' : 'Disable repeat')"
                         shortcut="Ctrl+R" placement="top">
                    <Pressable class="icon-button" cursor="pointer" @press="repeat">
                        <Icon :name="$player->repeat === 'one' ? 'repeat-1' : 'repeat'" font="Lucide" size="16"
                              :class="$player->repeat === 'off' ? 'dim' : 'on'" />
                    </Pressable>
                </Tooltip>
            </Row>
            <Row class="timeline">
                <Text class="clock">{{ elapsed() }}</Text>
                <View class="track-slot"><Slot :content="$progress" /></View>
                <Text class="clock">{{ track()?->duration() ?? '0:00' }}</Text>
            </Row>
        </Column>

        <Row class="extras">
            <Icon name="square-play" font="Lucide" size="16" class="dim" />
            <Icon name="mic-vocal" font="Lucide" size="16" class="dim" />
            <Icon name="list-music" font="Lucide" size="16" class="dim" />
            <Icon name="monitor-speaker" font="Lucide" size="16" class="dim" />
            <Pressable class="icon-button" cursor="pointer" @press="mute" tooltip="Mute">
                <Icon :name="$player->volumeIcon()" font="Lucide" size="16" class="dim" />
            </Pressable>
            <View class="volume-slot"><Slot :content="$level" /></View>
            <Icon name="maximize-2" font="Lucide" size="16" class="dim" />
        </Row>
    </Row>
</template>

<style scoped>
    .bar { height: 72px; padding: 0 8px; align-items: center; justify-content: space-between; }
    .now { width: 30%; min-width: 180px; gap: 12px; align-items: center; }
    .cover { width: 56px; height: 56px; border-radius: 4px; }
    .titles { flex-shrink: 1; gap: 2px; }
    .title { font-size: 14px; font-weight: 500; }
    .artist { font-size: 12px; color: var(--subdued); }
    .icon-button { width: 32px; height: 32px; align-items: center; justify-content: center; }
    .dim { color: var(--subdued); }
    .icon-button:hover .dim { color: var(--text); }
    .on { color: var(--green); }
    .liked { color: var(--green); }
    .transport { width: 40%; max-width: 722px; gap: 4px; align-items: center; }
    .controls { gap: 8px; align-items: center; }
    .play { width: 32px; height: 32px; border-radius: 16px; background-color: var(--text); align-items: center;
            justify-content: center; margin: 0 8px; }
    .play { transition: transform 120ms; }
    .play:hover { transform: scale(1.06); }
    .play-icon { color: var(--base); }
    .timeline { width: 100%; gap: 8px; align-items: center; }
    .clock { min-width: 40px; font-size: 12px; color: var(--subdued); text-align: center; }
    .track-slot { flex: 1; }
    .extras { width: 30%; min-width: 180px; gap: 12px; align-items: center; justify-content: flex-end; padding-right: 8px; }
    .volume-slot { width: 93px; }
</style>
