<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;

/** Bounded thumbnail with pointer and keyboard-accessible controlled actions. */
final class MediaTile extends Component
{
    use Concerns\LocalizedText;

    #[State(hotReload: false)] public bool $hovered = false;
    #[State(hotReload: false)] public bool $focused = false;

    public function __construct(
        public string $source,
        public string $label,
        public bool $primary = false,
        public bool $enabled = true,
        /** Border and badge colour of the primary image (Tailwind indigo-500 by default). */
        public int|string $accent = 0xFF6366F1,
    ) {
        // Templates hand colours over as packed ARGB integers.
        if (is_string($accent) && preg_match('/^#[0-9A-Fa-f]{6}$/D', $accent) !== 1) {
            throw new \InvalidArgumentException('MediaTile accent must be #RRGGBB or an ARGB integer.');
        }
    }

    public function enter(): void
    {
        $this->hovered = true;
    }
    public function leave(): void
    {
        $this->hovered = false;
    }
    public function focus(): void
    {
        $this->focused = true;
    }
    public function activate(): void
    {
        if ($this->enabled) {
            $this->focused = !$this->focused;
        }
    }
    public function makePrimary(): void
    {
        if ($this->enabled && !$this->primary) {
            $this->emit('primary');
        }
    }
    public function remove(): void
    {
        if ($this->enabled) {
            $this->emit('remove');
        }
    }
    public function paused(): void
    {
        $this->hovered = $this->focused = false;
    }
}
?>
<template>
    <Pressable aspectRatio="1" width="100%" borderWidth="1" :borderColor="$primary ? $accent : '#293241'" borderRadius="12" overflow="hidden" :enabled="$enabled" on:hoverIn="enter" on:hoverOut="leave" on:focus="focus" on:press="activate" :accessibilityLabel="$label" backgroundColor="#101827">
        <Image :source="$source" width="100%" height="100%" resizeMode="cover" :accessibilityLabel="$label" />
        <View p-if="$primary" position="absolute" left="12" top="12" paddingVertical="4" paddingHorizontal="8" borderRadius="8" :backgroundColor="$accent"><Text textColor="#FFFFFF" fontSize="8" lineHeight="12" fontWeight="900">PRINCIPAL</Text></View>
        <Row p-if="$hovered || $focused" position="absolute" left="0" top="0" width="100%" height="100%" backgroundColor="#99000000" alignItems="center" justifyContent="center" gap="8">
            <Button p-if="!$primary" label="★" accessibilityLabel="Definir imagem principal" on:press="makePrimary" :enabled="$enabled" width="32" height="32" borderRadius="16" backgroundColor="#2563EB" textColor="#FFFFFF" />
            <Button label="×" :accessibilityLabel="uiText('media.remove')" on:press="remove" :enabled="$enabled" width="32" height="32" borderRadius="16" backgroundColor="#F43F5E" textColor="#FFFFFF" />
        </Row>
    </Pressable>
</template>
