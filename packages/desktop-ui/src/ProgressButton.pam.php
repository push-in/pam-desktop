<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

use Pam\Desktop\Component;

/** Controlled action preserving its label while the native indicator is busy. */
final class ProgressButton extends Component
{
    public function __construct(
        public string $label,
        public bool $loading = false,
        public bool $enabled = true,
        public int $backgroundColor = 0xFF2563EB,
        public int $textColor = 0xFFFFFFFF,
        public string $testId = '',
        public ?int $loadingBackgroundColor = null,
        public ?int $loadingTextColor = null,
        public ?int $loadingIndicatorColor = null,
    ) {}

    public function press(): void
    {
        if ($this->enabled && !$this->loading) {
            $this->emit('press');
        }
    }
}
?>
<template>
    <Pressable height="42" borderRadius="6" :backgroundColor="$loading && $loadingBackgroundColor !== null ? $loadingBackgroundColor : $backgroundColor" :enabled="$enabled && !$loading" :opacity="$loading || $enabled ? 1 : 0.38" :testId="$testId" :accessibilityLabel="$label" accessibilityRole="button" :accessibilityBusy="$loading" on:press="press">
        <Row height="100%" alignItems="center" justifyContent="center" gap="8">
            <ActivityIndicator p-if="$loading" size="16" :color="$loadingIndicatorColor ?? $loadingTextColor ?? $textColor" />
            <Text :textColor="$loading && $loadingTextColor !== null ? $loadingTextColor : $textColor" fontSize="14.4" lineHeight="21.6" fontWeight="500">{{ $label }}</Text>
        </Row>
    </Pressable>
</template>
