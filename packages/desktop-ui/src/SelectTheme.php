<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

/** Explicit tokens let desktop products match their existing design system. */
final readonly class SelectTheme
{
    public function __construct(
        public int $surface = 0xFFFFFFFF,
        public int $text = 0xFF0F172A,
        public int $muted = 0xFF64748B,
        public int $border = 0xFFCBD5E1,
        public int $highlight = 0xFFDBEAFE,
        public ?int $inputSurface = null,
        public ?int $popupSurface = null,
        public ?int $filterSurface = null,
        public ?int $optionText = null,
        public int|float $optionFontSize = 14,
        public int $optionFontWeight = 700,
        public string $fontFamily = 'system-ui',
        public float $popupElevation = 0,
        public ?int $popupBorder = null,
        public ?int $filterBorder = null,
        public ?int $selectedSurface = null,
        public ?int $selectedText = null,
        public ?int $clearIconColor = null,
        public ?int $triggerIconColor = null,
        public ?string $iconFont = null,
        public float $popupMinWidth = 0,
        public float $popupOffset = 4,
        public ?int $popupShadowColor = null,
        public float $popupShadowOffsetY = 0,
        public float $popupShadowBlur = 0,
        /** Option row height without the select-mode gap (null: 36, or 64 with images). PrimeVue: 2 × 8 padding + font size × line height. */
        public ?float $optionHeight = null,
        /** Field border while focused or open (null keeps the built-in focus style). */
        public ?int $focusBorder = null,
        /** Focus ring drawn around the field while focused or open, like `box-shadow: 0 0 0 <width> <color>`. */
        public ?int $focusRing = null,
        public float $focusRingWidth = 2,
        /** Placeholder colour of the field (null: the platform's dimmed text colour). */
        public ?int $placeholderColor = null,
    ) {
        if ($popupMinWidth < 0 || $popupMinWidth > 1024 || $popupOffset < 0 || $popupOffset > 32
            || $popupShadowBlur < 0 || $popupShadowBlur > 64 || abs($popupShadowOffsetY) > 64
            || $optionFontSize < 8 || $optionFontSize > 32
            || ($optionHeight !== null && (!is_finite($optionHeight) || $optionHeight < 16 || $optionHeight > 96))
            || !is_finite($focusRingWidth) || $focusRingWidth < 0 || $focusRingWidth > 8) {
            throw new \InvalidArgumentException('Invalid select theme geometry.');
        }
    }
}
