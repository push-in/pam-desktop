<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\LayoutEvent;
use Pam\Desktop\Shortcut;
use Pam\Desktop\System\ViewRef;
use Pam\Desktop\Internal\Runtime;
use Pam\Desktop\ViewMeasurement;

/**
 * Floating content anchored to an element: a menu, a picker, a details card.
 * It is drawn above the whole window, modals included, next to the element
 * whose `nativeRef` is `anchor`, on the `placement` side, and flips to the
 * other side when it does not fit. A press outside it or Escape emits
 * `close`; the owner decides whether `open` turns false.
 *
 * ```html
 * <Pressable nativeRef="emoji-button" on:press="toggleEmoji">…</Pressable>
 * <Popover :open="$emojiOpen" anchor="emoji-button" placement="top-end" on:close="closeEmoji">
 *     <EmojiGrid />
 * </Popover>
 * ```
 */
final class Popover extends Component
{
    #[State] public float $x = 0.0;
    #[State] public float $y = 0.0;
    /** The side it ended up on after flipping. */
    #[State] public string $side = 'bottom';
    #[State] public bool $placed = false;

    public readonly string $key;
    private ?ViewMeasurement $anchorBox = null;
    private float $width = 0.0;
    private float $height = 0.0;
    private bool $shown = false;
    private int $generation = 0;
    private ?string $escape = null;

    public function __construct(
        #[State] public bool $open,
        public string $anchor,
        public string $placement = 'bottom-start',
        public float $offset = 6.0,
        /** A press outside closes it (else presses reach what is below). */
        public bool $dismissible = true,
        public ?string $label = null,
    ) {
        new PopoverPlacement($placement);
        $this->key = 'popover-'.spl_object_id($this);
    }

    public function rendering(): void
    {
        if ($this->open === $this->shown) {
            return;
        }
        $this->shown = $this->open;
        $generation = ++$this->generation;
        if (!$this->open) {
            $this->placed = false;
            $this->anchorBox = null;
            $this->releaseEscape();

            return;
        }
        $this->escape ??= Shortcut::register('Escape', fn () => $this->dismiss());
        ViewRef::of($this->anchor)->measureInWindow(function (?ViewMeasurement $box) use ($generation): void {
            if ($generation !== $this->generation || $box === null) {
                return;
            }
            $this->anchorBox = $box;
            $this->reposition();
        });
    }

    public function measured(LayoutEvent $layout): void
    {
        if ($layout->width === $this->width && $layout->height === $this->height) {
            return;
        }
        $this->width = $layout->width;
        $this->height = $layout->height;
        $this->reposition();
    }

    public function dismiss(): void
    {
        if ($this->open) {
            $this->emit('close');
        }
    }

    public function unmount(): void
    {
        ++$this->generation;
        $this->releaseEscape();
    }

    /** Places the surface once both the anchor and the surface are measured. */
    private function reposition(): void
    {
        $box = $this->anchorBox;
        if ($box === null || $this->width <= 0.0) {
            return;
        }
        $window = Runtime::windowMetrics();
        $spot = (new PopoverPlacement($this->placement))->place(
            $box->windowX,
            $box->windowY,
            $box->width,
            $box->height,
            $this->width,
            $this->height,
            $window->width,
            $window->height,
            $this->offset,
        );
        $this->x = $spot['x'];
        $this->y = $spot['y'];
        $this->side = $spot['side'];
        $this->placed = true;
    }

    private function releaseEscape(): void
    {
        if ($this->escape !== null) {
            Shortcut::unregister($this->escape);
            $this->escape = null;
        }
    }
}
?>

<template>
    <Portal p-if="$open" :name="$key" host="window">
        <View class="layer" pointerEvents="box-none">
            <Pressable p-if="$dismissible" class="outside" cursor="default" on:press="dismiss" accessibilityLabel="Close" />
            <Column class="surface" :left="$x" :top="$y" :opacity="$placed ? 1 : 0" on:layout="measured"
                    :accessibilityLabel="$label">
                <Slot />
            </Column>
        </View>
    </Portal>
</template>

<style scoped>
    .layer { position: absolute; left: 0; top: 0; right: 0; bottom: 0; }
    .outside { position: absolute; left: 0; top: 0; right: 0; bottom: 0; }
    .surface { position: absolute; }
</style>
