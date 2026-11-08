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
    /** @param Closure(bool):void $changed */
    public function __construct(#[State] public bool $checked, private readonly Closure $changed, #[State] public bool $enabled = true, ?string $label = null, ?SelectTheme $theme = null, public readonly int $size = 24)
    {
        $this->label = $label ?? \Pam\Desktop\I18n\FrameworkText::get('checkbox.label');
        if ($size < 16 || $size > 48) {
            throw new \InvalidArgumentException("Checkbox size must be16–48 pixels.");
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
    public function checkedState(): int
    {
        return ($this->checked ? AccessibilityCheckedState::Checked : AccessibilityCheckedState::Unchecked)->value;
    }
}
?>
<template><Pressable class="checkbox" :width="$size" :height="$size" :enabled="$enabled" on:press="toggle" on:keyDown="key" captureKeys="Enter,Space" accessibilityRole="checkbox" :accessibilityLabel="$label" :accessibilityCheckedState="checkedState()" :backgroundColor="$checked ? $theme->highlight : $theme->surface" :borderColor="$theme->border" :opacity="$enabled ? 1 : 0.5" alignItems="center" justifyContent="center"><Text p-if="$checked" fontSize="16" lineHeight="20" :textColor="$theme->text">✓</Text></Pressable></template>
<style scoped>
.checkbox { border-width: 1px; border-radius: 5px; }
.checkbox:focus { border-color: #818CF8; border-width: 2px; }
</style>
