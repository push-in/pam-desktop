<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

use InvalidArgumentException;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\GestureEvent;
use Pam\Desktop\KeyEvent;
use Pam\Desktop\Renderable;

/**
 * Two panes and a divider that resizes them: a sidebar and an editor, a
 * list and a thread. The first child is one pane, the second the other;
 * `sized` says which one has `size` points (along `direction`), the other
 * takes the rest. Dragging the divider (or Left/Right, Up/Down with it
 * focused) emits `resize` with the new size, kept between `min` and `max`.
 *
 * ```html
 * <SplitView :size="$sidebar" min="170" max="480" on:resize="resizeSidebar">
 *     <Explorer />
 *     <Editor />
 * </SplitView>
 * ```
 */
final class SplitView extends Component
{
    #[State] public bool $hovered = false;
    #[State] public bool $dragging = false;
    private float $start = 0.0;

    public function __construct(
        #[State] public float $size,
        public string $direction = 'horizontal',
        public float $min = 0.0,
        public float $max = 100_000.0,
        public string $sized = 'start',
        public bool $resizable = true,
        public float $step = 10.0,
        public int|string $dividerColor = '#2B2B2B',
        public int|string $activeColor = '#0078D4',
        public float $thickness = 1.0,
        /** The width that takes the pointer, centred on the line. */
        public float $hitArea = 6.0,
        public ?string $label = null,
    ) {
        if (!in_array($direction, ['horizontal', 'vertical'], true) || !in_array($sized, ['start', 'end'], true)) {
            throw new InvalidArgumentException('SplitView direction is horizontal or vertical, and sized is start or end.');
        }
        if ($min < 0.0 || $max < $min) {
            throw new InvalidArgumentException('SplitView needs 0 <= min <= max.');
        }
    }

    public function horizontal(): bool
    {
        return $this->direction === 'horizontal';
    }

    /** The child shown in pane 0 or 1. */
    public function pane(int $index): ?Renderable
    {
        $children = array_values(array_filter($this->slot(), static fn (mixed $child): bool => $child instanceof Renderable));

        return $children[$index] ?? null;
    }

    public function fixed(int $index): bool
    {
        return ($index === 0) === ($this->sized === 'start');
    }

    public function active(): bool
    {
        return $this->dragging || $this->hovered;
    }

    public function begin(): void
    {
        if (!$this->resizable) {
            return;
        }
        $this->start = $this->size;
        $this->dragging = true;
    }

    public function drag(GestureEvent $event): void
    {
        if (!$this->dragging) {
            return;
        }
        $delta = $this->horizontal() ? $event->translationX : $event->translationY;
        $this->resize($this->start + ($this->sized === 'start' ? $delta : -$delta));
    }

    public function end(): void
    {
        $this->dragging = false;
    }

    public function hover(bool $inside): void
    {
        $this->hovered = $inside && $this->resizable;
    }

    public function key(KeyEvent $event): void
    {
        if (!$this->resizable || $event->control || $event->alt || $event->meta) {
            return;
        }
        $grow = $this->sized === 'start' ? 1.0 : -1.0;
        $delta = match ($event->key) {
            'ArrowRight', 'ArrowDown' => $this->step * $grow,
            'ArrowLeft', 'ArrowUp' => -$this->step * $grow,
            'Home' => -INF,
            'End' => INF,
            default => null,
        };
        if ($delta !== null) {
            $this->resize(is_infinite($delta) ? ($delta > 0 ? $this->max : $this->min) : $this->size + $delta);
        }
    }

    private function resize(float $size): void
    {
        $size = round(max($this->min, min($this->max, $size)));
        if ($size === $this->size) {
            return;
        }
        $this->size = $size;
        $this->emit('resize', $size);
    }
}
?>

<template>
    <View class="split" :flexDirection="horizontal() ? 'row' : 'column'">
        <View class="pane" :flexBasis="fixed(0) ? $size : 0" :flexGrow="fixed(0) ? 0 : 1" :flexShrink="fixed(0) ? 0 : 1">
            <Slot :content="pane(0)" />
        </View>
        <GestureDetector gestureType="pan" :gestureDirection="horizontal() ? 'horizontal' : 'vertical'" gestureMinDistance="1"
                         :gestureEnabled="$resizable" on:gestureBegin="begin" on:gestureUpdate="drag"
                         on:gestureEnd="end" on:gestureCancel="end">
            <Pressable p-if="horizontal()" class="divider across" :width="$hitArea"
                       :marginLeft="($thickness - $hitArea) / 2" :marginRight="($thickness - $hitArea) / 2"
                       :enabled="$resizable" :cursor="$resizable ? 'col-resize' : 'default'"
                       on:hoverIn="hover(true)" on:hoverOut="hover(false)" on:keyDown="key"
                       captureKeys="ArrowLeft,ArrowRight,ArrowUp,ArrowDown,Home,End"
                       accessibilityRole="adjustable" :accessibilityLabel="$label ?? 'Resize'"
                       :accessibilityValueMin="$min" :accessibilityValueMax="$max" :accessibilityValueNow="$size">
                <View class="line" :width="$thickness" :backgroundColor="active() ? $activeColor : $dividerColor" />
            </Pressable>
            <Pressable p-else class="divider down" :height="$hitArea"
                       :marginTop="($thickness - $hitArea) / 2" :marginBottom="($thickness - $hitArea) / 2"
                       :enabled="$resizable" :cursor="$resizable ? 'row-resize' : 'default'"
                       on:hoverIn="hover(true)" on:hoverOut="hover(false)" on:keyDown="key"
                       captureKeys="ArrowLeft,ArrowRight,ArrowUp,ArrowDown,Home,End"
                       accessibilityRole="adjustable" :accessibilityLabel="$label ?? 'Resize'"
                       :accessibilityValueMin="$min" :accessibilityValueMax="$max" :accessibilityValueNow="$size">
                <View class="line" :height="$thickness" :backgroundColor="active() ? $activeColor : $dividerColor" />
            </Pressable>
        </GestureDetector>
        <View class="pane" :flexBasis="fixed(1) ? $size : 0" :flexGrow="fixed(1) ? 0 : 1" :flexShrink="fixed(1) ? 0 : 1">
            <Slot :content="pane(1)" />
        </View>
    </View>
</template>

<style scoped>
    .split { flex: 1; min-width: 0; min-height: 0; }
    .pane { min-width: 0; min-height: 0; overflow: hidden; }
    .divider { z-index: 1; }
    .across { flex-direction: row; justify-content: center; align-self: stretch; }
    .down { flex-direction: column; justify-content: center; align-self: stretch; }
    .line { align-self: stretch; transition: background-color 120ms; }
</style>
