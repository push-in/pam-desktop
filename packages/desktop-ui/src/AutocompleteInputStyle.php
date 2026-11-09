<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

/** Input typography and geometry, independent of suggestion rendering. */
final readonly class AutocompleteInputStyle
{
    public function __construct(
        public int $height = 40,
        public int $fontSize = 14,
        public int $fontWeight = 400,
        public int $paddingHorizontal = 12,
        public int $radius = 6,
        /** Line box of the value (CSS `line-height`); null keeps the font's own. Tailwind's preflight makes inputs inherit 1.5. */
        public ?float $lineHeight = null,
    ) {
        if ($height < 24 || $height > 96 || $fontSize < 10 || $fontSize > 32
            || $fontWeight < 100 || $fontWeight > 900 || $paddingHorizontal < 0
            || $paddingHorizontal > 48 || $radius < 0 || $radius > 48
            || ($lineHeight !== null && (!is_finite($lineHeight) || $lineHeight < 8 || $lineHeight > 96))) {
            throw new \InvalidArgumentException('Invalid autocomplete input geometry.');
        }
    }
}
