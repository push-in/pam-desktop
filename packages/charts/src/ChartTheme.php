<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts;

/**
 * Ink and surface colors shared by every chart.
 *
 * - `primary`: default single-series color and first palette hue.
 * - `ink`: value labels and titles, readable without relying on hue.
 * - `muted`: axis labels, legends and secondary text.
 * - `grid`: recessive grid lines (defaults to 12% ink).
 * - `surface`: chart background and marker knockouts.
 * - `legend`: legend text and marker metrics ({@see LegendStyle}).
 */
final readonly class ChartTheme
{
    private function __construct(
        public string $primary,
        public string $ink,
        public string $muted,
        public string $grid,
        public string $surface,
        public ChartPalette $palette,
        public LegendStyle $legend = new LegendStyle(),
    ) {
    }

    public static function dark(
        string $primary = '#19c5ff',
        string $ink = '#f4f7fa',
        string $muted = '#a0adbb',
        ?string $grid = null,
        string $surface = '#19222c',
        ?ChartPalette $palette = null,
    ): self {
        return self::make($primary, $ink, $muted, $grid, $surface, $palette);
    }

    public static function light(
        string $primary = '#0b7fd6',
        string $ink = '#101828',
        string $muted = '#667085',
        ?string $grid = null,
        string $surface = '#ffffff',
        ?ChartPalette $palette = null,
    ): self {
        return self::make($primary, $ink, $muted, $grid, $surface, $palette);
    }

    public static function make(
        string $primary,
        string $ink,
        string $muted,
        ?string $grid = null,
        string $surface = '#00000000',
        ?ChartPalette $palette = null,
    ): self {
        $primary = Internal\Color::normalize($primary);

        return new self(
            $primary,
            Internal\Color::normalize($ink),
            Internal\Color::normalize($muted),
            $grid === null ? Internal\Color::alpha($ink, 0.12) : Internal\Color::normalize($grid),
            Internal\Color::normalize($surface),
            $palette ?? ChartPalette::categorical($primary),
        );
    }

    /**
     * Builds a theme from template props such as `['primary' => '#19c5ff', 'ink' => '#fff']`.
     * Missing keys fall back to the dark theme.
     *
     * @param array<array-key, mixed> $values
     */
    public static function fromArray(array $values, ?self $base = null): self
    {
        $base ??= self::dark();
        $read = static function (string $key, string $default) use ($values): string {
            $value = $values[$key] ?? null;

            return is_string($value) || is_int($value) ? Internal\Color::fromMixed($value) : $default;
        };

        return self::make(
            $read('primary', $base->primary),
            $read('ink', $base->ink),
            $read('muted', $base->muted),
            $read('grid', $base->grid),
            $read('surface', $base->surface),
            is_string($values['primary'] ?? null) || is_int($values['primary'] ?? null) ? null : $base->palette,
        )->legendStyle(self::legendFromArray($values, $base->legend));
    }

    /**
     * Legend metrics from `legendFontSize`, `legendFontWeight` (400..700),
     * `legendMarkerRadius`, `legendMarkerGap`, `legendItemGap` and
     * `legendValueGap` keys.
     *
     * @param array<array-key, mixed> $values
     */
    private static function legendFromArray(array $values, LegendStyle $base): LegendStyle
    {
        $number = static fn (string $key, float $default): float => is_int($values[$key] ?? null) || is_float($values[$key] ?? null) || (is_string($values[$key] ?? null) && is_numeric($values[$key]))
            ? (float) $values[$key]
            : $default;
        $weight = $values['legendFontWeight'] ?? null;
        $weight = is_numeric($weight) ? match (true) {
            (int) $weight >= 700 => \Pam\Desktop\Canvas\FontWeight::Bold,
            (int) $weight >= 600 => \Pam\Desktop\Canvas\FontWeight::SemiBold,
            (int) $weight >= 500 => \Pam\Desktop\Canvas\FontWeight::Medium,
            default => \Pam\Desktop\Canvas\FontWeight::Regular,
        } : $base->fontWeight;

        return new LegendStyle(
            $number('legendFontSize', $base->fontSize),
            $weight,
            $number('legendMarkerRadius', $base->markerRadius),
            $number('legendMarkerGap', $base->markerGap),
            $number('legendItemGap', $base->itemGap),
            $number('legendValueGap', $base->valueGap),
        );
    }

    public function palette(ChartPalette $palette): self
    {
        return new self($this->primary, $this->ink, $this->muted, $this->grid, $this->surface, $palette, $this->legend);
    }

    /** The same colours with other legend metrics. */
    public function legendStyle(LegendStyle $legend): self
    {
        return new self($this->primary, $this->ink, $this->muted, $this->grid, $this->surface, $this->palette, $legend);
    }

    /** Color for a series index: the series' own color, otherwise the palette hue. */
    public function seriesColor(?string $color, int $index): string
    {
        return $color ?? $this->palette->color($index);
    }
}
