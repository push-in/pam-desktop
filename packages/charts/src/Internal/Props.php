<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts\Internal;

use Closure;
use InvalidArgumentException;
use Pam\Desktop\Charts\ChartTheme;
use Pam\Desktop\Charts\Curve;
use Pam\Desktop\Charts\LegendPosition;
use Pam\Desktop\Charts\Series;
use Pam\Desktop\Charts\Slice;

/**
 * Coerces template props, which may arrive as strings, into typed chart inputs.
 *
 * @internal
 */
final class Props
{
    private function __construct()
    {
    }

    public static function bool(mixed $value, bool $default = false): bool
    {
        if (is_bool($value)) {
            return $value;
        }

        if ($value === null) {
            return $default;
        }

        if (is_int($value) || is_float($value)) {
            return $value !== 0 && $value !== 0.0;
        }

        return is_string($value)
            ? filter_var($value, FILTER_VALIDATE_BOOL, FILTER_NULL_ON_FAILURE) ?? $default
            : $default;
    }

    public static function int(mixed $value, int $default = 0): int
    {
        if (is_int($value)) {
            return $value;
        }

        if (is_float($value) && is_finite($value)) {
            return (int) $value;
        }

        return is_string($value) && preg_match('/^-?\d+$/D', trim($value)) === 1 ? (int) $value : $default;
    }

    public static function nullableInt(mixed $value): ?int
    {
        return $value === null || $value === '' ? null : self::int($value);
    }

    public static function float(mixed $value, float $default = 0.0): float
    {
        if (is_int($value) || is_float($value)) {
            return is_finite((float) $value) ? (float) $value : $default;
        }

        return is_string($value) && is_numeric(trim($value)) ? (float) trim($value) : $default;
    }

    /**
     * A chart color from a CSS hex string or a packed ARGB integer (how template attributes arrive).
     */
    public static function color(mixed $value): ?string
    {
        if (is_string($value) && str_starts_with(trim($value), '#')) {
            return Color::normalize(trim($value));
        }

        return is_int($value) ? Color::fromArgb($value) : null;
    }

    public static function string(mixed $value, ?string $default = null): ?string
    {
        if (is_string($value)) {
            return $value;
        }

        return is_int($value) || is_float($value) ? (string) $value : $default;
    }

    /**
     * Numbers from a list or a comma-separated string such as `"3, 5, 8"`.
     *
     * @return list<float>
     */
    public static function floatList(mixed $value): array
    {
        if (is_string($value)) {
            $value = $value === '' ? [] : array_map('trim', explode(',', $value));
        }

        if (!is_array($value)) {
            return [];
        }

        $numbers = [];

        foreach ($value as $item) {
            if (!is_int($item) && !is_float($item) && !(is_string($item) && is_numeric($item))) {
                throw new InvalidArgumentException('Chart values must be numbers.');
            }

            $numbers[] = (float) $item;
        }

        return Series::validValues($numbers);
    }

    /**
     * Strings from a list or a comma-separated string.
     *
     * @return list<string>
     */
    public static function stringList(mixed $value): array
    {
        if (is_string($value)) {
            $value = $value === '' ? [] : array_map('trim', explode(',', $value));
        }

        if (!is_array($value)) {
            return [];
        }

        return array_values(array_map(
            static fn (mixed $item): string => self::string($item, '') ?? '',
            $value,
        ));
    }

    /**
     * Series from `[['label' => 'A', 'values' => [1, 2], 'color' => '#19c5ff'], ...]`,
     * or a bare list of numbers for a single unnamed series.
     *
     * @return list<Series>
     */
    public static function series(mixed $value): array
    {
        if (!is_array($value)) {
            return [];
        }

        if ($value !== [] && !is_array(reset($value))) {
            return [Series::make('', self::floatList($value))];
        }

        $series = [];

        foreach ($value as $item) {
            if (!is_array($item)) {
                throw new InvalidArgumentException('Chart series must be arrays with label and values.');
            }

            $entry = Series::make(self::string($item['label'] ?? null, '') ?? '', self::floatList($item['values'] ?? []));
            $color = self::color($item['color'] ?? null);
            $series[] = $color === null ? $entry : $entry->color($color);
        }

        return $series;
    }

    /**
     * Slices from `[['label' => 'Pix', 'value' => 70, 'color' => '#19c5ff'], ...]`.
     *
     * @return list<Slice>
     */
    public static function slices(mixed $value): array
    {
        if (!is_array($value)) {
            return [];
        }

        $slices = [];

        foreach ($value as $item) {
            if (!is_array($item)) {
                throw new InvalidArgumentException('Donut slices must be arrays with label and value.');
            }

            $slice = Slice::make(self::string($item['label'] ?? null, '') ?? '', self::float($item['value'] ?? 0.0));
            $color = self::color($item['color'] ?? null);
            $slices[] = $color === null ? $slice : $slice->color($color);
        }

        return $slices;
    }

    public static function theme(mixed $value): ?ChartTheme
    {
        if ($value instanceof ChartTheme) {
            return $value;
        }

        if (is_array($value)) {
            return ChartTheme::fromArray($value);
        }

        return match (is_string($value) ? strtolower($value) : null) {
            'dark' => ChartTheme::dark(),
            'light' => ChartTheme::light(),
            default => null,
        };
    }

    public static function curve(mixed $value): Curve
    {
        if ($value instanceof Curve) {
            return $value;
        }

        if (is_string($value)) {
            return match (strtolower(trim($value))) {
                'monotone', 'smooth' => Curve::Monotone,
                'linear', '' => Curve::Linear,
                default => Curve::tryFrom(self::int($value)) ?? Curve::Linear,
            };
        }

        return Curve::tryFrom(self::int($value)) ?? Curve::Linear;
    }

    public static function legend(mixed $value): LegendPosition
    {
        if ($value instanceof LegendPosition) {
            return $value;
        }

        if (is_bool($value)) {
            return $value ? LegendPosition::Bottom : LegendPosition::None;
        }

        if (is_string($value)) {
            return match (strtolower(trim($value))) {
                'top' => LegendPosition::Top,
                'right' => LegendPosition::Right,
                'bottom', 'true' => LegendPosition::Bottom,
                'none', 'false', '' => LegendPosition::None,
                default => LegendPosition::tryFrom(self::int($value)) ?? LegendPosition::None,
            };
        }

        return LegendPosition::tryFrom(self::int($value)) ?? LegendPosition::None;
    }

    /**
     * @return (Closure(float): string)|null
     */
    public static function formatter(mixed $value): ?Closure
    {
        if (!$value instanceof Closure) {
            return null;
        }

        return static function (float $number) use ($value): string {
            $formatted = $value($number);

            return is_string($formatted) || is_int($formatted) || is_float($formatted) ? (string) $formatted : '';
        };
    }

    /**
     * Component event handler registered by the template renderer, if any.
     *
     * @param array<string, mixed> $props
     */
    public static function event(array $props, string $name): ?Closure
    {
        $events = $props['__pamComponentEvents'] ?? null;
        $handler = is_array($events) ? ($events[$name] ?? null) : null;

        return $handler instanceof Closure ? $handler : null;
    }
}
