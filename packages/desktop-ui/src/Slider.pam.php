<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

use Closure;
use InvalidArgumentException;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\GestureEvent;
use Pam\Desktop\KeyEvent;
use Pam\Desktop\LayoutEvent;
use Pam\Desktop\PressEvent;

/**
 * Controlled horizontal slider (a volume, a playback position, a setting):
 * a click or a drag sets the value, the arrow keys step it, Home and End
 * jump to the ends. The owner applies what `$changed` reports; `$committed`
 * runs once the pointer is released, for work too heavy to repeat while
 * dragging (seeking a stream). `min` and `max` may change while it is shown
 * (the length of the next track): set them on the same instance.
 */
final class Slider extends Component
{
    /** Accessibility label; the localized "Slider" when none is given. */
    public readonly string $label;

    #[State]
    public float $width = 0.0;

    #[State]
    public bool $hovered = false;

    #[State]
    public bool $dragging = false;

    /**
     * @param Closure(float):void $changed
     * @param ?Closure(float):void $committed
     * @param float $step 0 for a continuous value
     * @param int|string $fillColor the part before the thumb, at rest
     * @param int|string $activeColor the same part while hovered or dragged
     */
    public function __construct(
        #[State] public float $value,
        private readonly Closure $changed,
        #[State] public float $min = 0.0,
        #[State] public float $max = 1.0,
        public readonly float $step = 0.0,
        #[State] public bool $enabled = true,
        ?string $label = null,
        private readonly ?Closure $committed = null,
        public readonly int $trackHeight = 4,
        public readonly int $thumbSize = 12,
        public readonly int|string $trackColor = '#4D4D4D',
        public readonly int|string $fillColor = '#FFFFFF',
        public readonly int|string $activeColor = '#1ED760',
        public readonly int|string $thumbColor = '#FFFFFF',
        public readonly ?string $valueText = null,
    ) {
        if (!($max > $min) || $step < 0.0 || $step > $max - $min) {
            throw new InvalidArgumentException('Slider needs min < max and a step from 0 up to max - min.');
        }
        if ($trackHeight < 1 || $thumbSize < $trackHeight) {
            throw new InvalidArgumentException('Slider thumbSize must be at least its trackHeight (1 pixel or more).');
        }
        $this->label = $label ?? 'Slider';
    }

    /** The value as a share of the track, 0 to 1. */
    public function fraction(): float
    {
        if (!($this->max > $this->min)) {
            return 0.0;
        }

        return max(0.0, min(1.0, ($this->value - $this->min) / ($this->max - $this->min)));
    }

    public function measure(LayoutEvent $event): void
    {
        $this->width = $event->width;
    }

    public function press(PressEvent $event): void
    {
        $this->dragging = true;
        $this->seek($event->x);
    }

    public function release(): void
    {
        $this->dragging = false;
        if ($this->enabled && $this->committed !== null) {
            ($this->committed)($this->value);
        }
    }

    public function drag(GestureEvent $event): void
    {
        $this->dragging = true;
        $this->seek($event->x);
    }

    public function dragEnd(GestureEvent $event): void
    {
        $this->seek($event->x);
        $this->release();
    }

    public function key(KeyEvent $event): void
    {
        if (!$this->enabled || $event->control || $event->alt || $event->meta) {
            return;
        }
        $step = $this->step > 0.0 ? $this->step : ($this->max - $this->min) / 20;
        $next = match ($event->key) {
            'ArrowRight', 'ArrowUp' => $this->value + $step,
            'ArrowLeft', 'ArrowDown' => $this->value - $step,
            'PageUp' => $this->value + $step * 5,
            'PageDown' => $this->value - $step * 5,
            'Home' => $this->min,
            'End' => $this->max,
            default => null,
        };
        if ($next !== null) {
            $this->set($next);
            if ($this->committed !== null) {
                ($this->committed)($this->value);
            }
        }
    }

    public function hover(bool $hovered): void
    {
        $this->hovered = $hovered;
    }

    public function active(): bool
    {
        return $this->enabled && ($this->hovered || $this->dragging);
    }

    public function text(): string
    {
        return $this->valueText ?? (string) round($this->value, 2);
    }

    private function seek(float $x): void
    {
        if ($this->enabled && $this->width > 0.0 && $this->max > $this->min) {
            $this->set($this->min + max(0.0, min(1.0, $x / $this->width)) * ($this->max - $this->min));
        }
    }

    private function set(float $value): void
    {
        if ($this->step > 0.0) {
            $value = $this->min + round(($value - $this->min) / $this->step) * $this->step;
        }
        $value = max($this->min, min($this->max, $value));
        if ($value !== $this->value) {
            $this->value = $value;
            ($this->changed)($value);
        }
    }
}
?>
<template>
    <GestureDetector gestureType="pan" gestureDirection="horizontal" gestureMinDistance="1" :gestureEnabled="$enabled"
                     on:gestureUpdate="drag" on:gestureEnd="dragEnd" on:gestureCancel="release">
        <Pressable class="slider" :height="$thumbSize + 4" :enabled="$enabled" :opacity="$enabled ? 1 : 0.5"
                   on:layout="measure" on:pressIn="press" on:pressOut="release"
                   on:hoverIn="hover(true)" on:hoverOut="hover(false)" on:keyDown="key"
                   captureKeys="ArrowLeft,ArrowRight,ArrowUp,ArrowDown,PageUp,PageDown,Home,End"
                   accessibilityRole="adjustable" :accessibilityLabel="$label" :accessibilityValueMin="$min"
                   :accessibilityValueMax="$max" :accessibilityValueNow="$value" :accessibilityValueText="text()">
            <Row class="track" :height="$trackHeight" :borderRadius="$trackHeight / 2" :backgroundColor="$trackColor">
                <View :flexGrow="fraction()" :borderRadius="$trackHeight / 2"
                      :backgroundColor="active() ? $activeColor : $fillColor">
                    <View class="thumb" :width="$thumbSize" :height="$thumbSize" :borderRadius="$thumbSize / 2"
                          :top="($trackHeight - $thumbSize) / 2" :right="-$thumbSize / 2"
                          :backgroundColor="$thumbColor" :opacity="active() ? 1 : 0" />
                </View>
                <View :flexGrow="1 - fraction()" />
            </Row>
        </Pressable>
    </GestureDetector>
</template>
<style scoped>
.slider { justify-content: center; }
.track { flex-direction: row; }
.thumb { position: absolute; elevation: 2; }
</style>
