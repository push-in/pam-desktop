<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\Internal\Runtime;
use Pam\Desktop\LayoutEvent;
use Pam\Desktop\System\ComponentTimer;
use Pam\Desktop\System\ViewRef;
use Pam\Desktop\ViewMeasurement;

/**
 * A rich tooltip: a title, an optional line of description and the keyboard
 * shortcut, shown after the pointer rests on the children for `delay`
 * milliseconds and hidden when it leaves. It never takes presses. For plain
 * text, the `tooltip` attribute of any element is lighter (the system's).
 *
 * ```html
 * <Tooltip title="Mention someone" shortcut="Ctrl+Shift+2" placement="top">
 *     <Icon name="at-sign" font="Lucide" size="16" />
 * </Tooltip>
 * ```
 */
final class Tooltip extends Component
{
    #[State] public bool $visible = false;
    #[State] public float $x = 0.0;
    #[State] public float $y = 0.0;
    #[State] public bool $placed = false;

    public readonly string $key;
    private ?ComponentTimer $timer = null;
    private ?ViewMeasurement $anchorBox = null;
    private float $width = 0.0;
    private float $height = 0.0;

    public function __construct(
        public string $title,
        public ?string $description = null,
        public ?string $shortcut = null,
        public string $placement = 'top',
        public int $delay = 500,
        public float $offset = 6.0,
        public int|string $background = '#1F1F1F',
        public int|string $color = '#F2F2F2',
        public int|string $muted = '#A8A8A8',
        public int|string $border = '#3A3A3A',
    ) {
        new PopoverPlacement($placement);
        $this->key = 'tooltip-'.spl_object_id($this);
    }

    public function enter(): void
    {
        $this->timer?->cancel();
        $this->timer = $this->timers()->timeout($this->delay, function (): void {
            $this->timer = null;
            ViewRef::of($this->key)->measureInWindow(function (?ViewMeasurement $box): void {
                if ($box === null) {
                    return;
                }
                $this->anchorBox = $box;
                $this->visible = true;
                $this->reposition();
            });
        });
    }

    public function leave(): void
    {
        $this->timer?->cancel();
        $this->timer = null;
        $this->visible = false;
        $this->placed = false;
    }

    public function measured(LayoutEvent $layout): void
    {
        $this->width = $layout->width;
        $this->height = $layout->height;
        $this->reposition();
    }

    private function reposition(): void
    {
        if ($this->anchorBox === null || $this->width <= 0.0) {
            return;
        }
        $window = Runtime::windowMetrics();
        $box = $this->anchorBox;
        $spot = (new PopoverPlacement($this->placement))->place(
            $box->windowX, $box->windowY, $box->width, $box->height,
            $this->width, $this->height, $window->width, $window->height, $this->offset,
        );
        $this->x = $spot['x'];
        $this->y = $spot['y'];
        $this->placed = true;
    }
}
?>

<template>
    <View :nativeRef="$key" on:hoverIn="enter" on:hoverOut="leave" on:pressIn="leave">
        <Slot />
        <Portal p-if="$visible" :name="$key" host="window">
            <Column class="tip" pointerEvents="none" :left="$x" :top="$y" :opacity="$placed ? 1 : 0"
                    :backgroundColor="$background" :borderColor="$border" on:layout="measured" accessibilityRole="text">
                <Row class="head">
                    <Text class="title" :textColor="$color">{{ $title }}</Text>
                    <Text p-if="$shortcut !== null" class="shortcut" :textColor="$muted" :borderColor="$border">{{ $shortcut }}</Text>
                </Row>
                <Text p-if="$description !== null" class="description" :textColor="$muted">{{ $description }}</Text>
            </Column>
        </Portal>
    </View>
</template>

<style scoped>
    .tip { position: absolute; max-width: 280px; padding: 6px 8px; gap: 2px; border-width: 1px; border-radius: 6px; box-shadow: 0 4px 12px #00000059; }
    .head { gap: 8px; align-items: center; }
    .title { font-size: 12px; font-weight: 600; }
    .shortcut { font-size: 11px; padding: 0 4px; border-width: 1px; border-radius: 3px; }
    .description { font-size: 12px; line-height: 16px; }
</style>
