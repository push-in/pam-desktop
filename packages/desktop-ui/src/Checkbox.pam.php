<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

use Closure;
use Pam\Desktop\AccessibilityCheckedState;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\KeyEvent;

/** Controlled checkbox composed of native primitives; the owner applies emitted values. */
final class Checkbox extends Component
{
    public readonly SelectTheme $theme;
    /** Accessibility label; the localized "Checkbox" when none is given. */
    public readonly string $label;
    /**
     * @param Closure(bool):void $changed
     * @param int $size the control's slot (PrimeVue `.p-checkbox`)
     * @param ?int $boxSize the painted box at the slot's top-left (`.p-checkbox-box`), `$size` when null
     * @param float $radius corner radius of the box
     */
    public function __construct(#[State] public bool $checked, private readonly Closure $changed, #[State] public bool $enabled = true, ?string $label = null, ?SelectTheme $theme = null, public readonly int $size = 24, public readonly ?int $boxSize = null, public readonly float $radius = 5.0)
    {
        $this->label = $label ?? \Pam\Desktop\I18n\FrameworkText::get('checkbox.label');
        if ($size < 16 || $size > 48) {
            throw new \InvalidArgumentException("Checkbox size must be16–48 pixels.");
        }
        if ($boxSize !== null && ($boxSize < 8 || $boxSize > $size)) {
            throw new \InvalidArgumentException("Checkbox boxSize must be 8 pixels up to its size.");
        }$this->theme = $theme ?? new SelectTheme();
    }
    public function toggle(): void
    {
        if ($this->enabled) {
            ($this->changed)(!$this->checked);
        }
    }
    public function key(KeyEvent|string $payload): void
    {
        $event = $payload instanceof KeyEvent ? $payload : KeyEvent::fromPayload($payload);
        if (!$event->repeat && !$event->control && !$event->alt && !$event->meta && in_array($event->key, [' ','Enter'], true)) {
            $this->toggle();
        }
    }
    /** A checked box is filled and bordered with the highlight (PrimeVue); with an icon font its check is the font's `check` at 70% of the box. */
    public function checkSize(): int
    {
        return (int) round($this->size * 0.7);
    }

    public function box(): int
    {
        return $this->boxSize ?? $this->size;
    }

    public function checkedState(): int
    {
        return ($this->checked ? AccessibilityCheckedState::Checked : AccessibilityCheckedState::Unchecked)->value;
    }
}
?>
<template><Pressable class="checkbox" :width="box()" :height="box()" :marginRight="$size - box()" :marginBottom="$size - box()" :borderRadius="$radius" :enabled="$enabled" on:press="toggle" on:keyDown="key" captureKeys="Enter,Space" accessibilityRole="checkbox" :accessibilityLabel="$label" :accessibilityCheckedState="checkedState()" :backgroundColor="$checked ? $theme->highlight : $theme->surface" :borderColor="$checked ? $theme->highlight : $theme->border" :opacity="$enabled ? 1 : 0.5" alignItems="center" justifyContent="center"><Icon p-if="$checked && $theme->iconFont !== null" name="check" :font="$theme->iconFont" :size="checkSize()" :color="$theme->text" /><Text p-else-if="$checked" fontSize="16" lineHeight="20" :textColor="$theme->text">✓</Text></Pressable></template>
<style scoped>
.checkbox { border-width: 1px; }
.checkbox:focus { border-color: #818CF8; border-width: 2px; }
</style>
