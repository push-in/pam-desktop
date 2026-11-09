<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

use Closure;
use Pam\Desktop\Appearance;

/**
 * Explicit toast tokens, for products whose design system restyles the
 * PrimeVue toast (e.g. an opaque surface with a severity edge on the left).
 * Every null token keeps the Aura look of the severity.
 *
 * ```php
 * Toast::service()->configure(theme: new ToastTheme(surface: 0xFF151D27, border: 0xFF1F2A36, edgeWidth: 3));
 * ```
 *
 * A `Closure(): ToastTheme` is called on every render of the stack, so toasts
 * on screen follow a live light/dark switch (see {@see self::adaptive()}).
 */
final readonly class ToastTheme
{
    /**
     * @param array<int, int> $accents icon/close colour per ToastSeverity value (default: the severity accent)
     * @param array<int, int> $edges left edge colour per ToastSeverity value (default: the accent)
     */
    public function __construct(
        public ?int $surface = null,
        public ?int $border = null,
        public float $edgeWidth = 0,
        public array $accents = [],
        public array $edges = [],
        public ?int $summaryColor = null,
        public float $summaryFontSize = 14,
        public float $summaryLineHeight = 21,
        public int $summaryFontWeight = 500,
        public ?int $detailColor = null,
        public float $detailFontSize = 14,
        public float $detailLineHeight = 21,
        public int $detailFontWeight = 400,
        /** Space between the summary and the detail. */
        public float $detailGap = 8,
        public float $paddingVertical = 12,
        public float $paddingHorizontal = 12,
        public float $radius = 6,
        public int $shadowColor = 0x0A000000,
        public float $shadowOffsetY = 4,
        public float $shadowBlur = 8,
        public float $iconSize = 18,
        public float $closeIconSize = 14,
        /** Moves the 28px close button from its slot (PrimeVue pulls it up and right). */
        public float $closeOffsetX = 0,
        public float $closeOffsetY = 0,
    ) {
        foreach ([$summaryFontSize, $detailFontSize] as $size) {
            if (!($size >= 8 && $size <= 32)) throw new \InvalidArgumentException('Invalid toast font size.');
        }
        if (!($edgeWidth >= 0 && $edgeWidth <= 8) || !($radius >= 0 && $radius <= 32) || !($iconSize >= 8 && $iconSize <= 48)
            || !($closeIconSize >= 8 && $closeIconSize <= 28) || abs($closeOffsetX) > 14 || abs($closeOffsetY) > 14
            || !($paddingVertical >= 0 && $paddingVertical <= 48) || !($paddingHorizontal >= 0 && $paddingHorizontal <= 48)
            || !($detailGap >= 0 && $detailGap <= 32) || !($shadowBlur >= 0 && $shadowBlur <= 64) || abs($shadowOffsetY) > 64) {
            throw new \InvalidArgumentException('Invalid toast theme geometry.');
        }
    }

    /**
     * The theme to draw now: the theme itself, or what its Closure returns.
     *
     * @param self|(Closure(): self)|null $theme
     */
    public static function resolve(self|Closure|null $theme): self
    {
        if (!$theme instanceof Closure) {
            return $theme ?? new self();
        }
        $resolved = $theme();
        if (!$resolved instanceof self) {
            throw new \InvalidArgumentException('A ToastTheme closure must return a ToastTheme.');
        }

        return $resolved;
    }

    /**
     * Picks `$dark` or `$light` by the effective scheme on every render.
     *
     * @return Closure(): self
     */
    public static function adaptive(self $light, self $dark): Closure
    {
        return static fn (): self => Appearance::isDark() ? $dark : $light;
    }

    public function accent(ToastSeverity $severity): int { return $this->accents[$severity->value] ?? $severity->accent(); }
    public function edge(ToastSeverity $severity): int { return $this->edges[$severity->value] ?? $this->accent($severity); }
    public function surface(ToastSeverity $severity): int { return $this->surface ?? $severity->surface(); }
    public function border(ToastSeverity $severity): int { return $this->border ?? $severity->border(); }
    public function summary(ToastSeverity $severity): int { return $this->summaryColor ?? $this->accent($severity); }
    public function detail(ToastSeverity $severity): int { return $this->detailColor ?? $this->accent($severity); }
}
