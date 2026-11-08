<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts;

use Closure;
use Pam\Desktop\Charts\Internal\Props;
use Pam\Desktop\TemplateRegistry;

/**
 * Registers the declarative chart tags:
 * `<LineChart>`, `<AreaChart>`, `<BarChart>`, `<DonutChart>`, `<Sparkline>` and `<ProgressRing>`.
 *
 * Boolean and numeric props may arrive as strings from templates and are coerced.
 * `@select` on line, area and bar charts receives the selected x index or null.
 */
final class ChartsPluginProvider
{
    public const string LINE = 'LineChart';
    public const string AREA = 'AreaChart';
    public const string BAR = 'BarChart';
    public const string DONUT = 'DonutChart';
    public const string SPARKLINE = 'Sparkline';
    public const string PROGRESS_RING = 'ProgressRing';

    public function register(): void
    {
        TemplateRegistry::component(self::LINE, static fn (array $props): LineChart => self::lineChart($props, false));
        TemplateRegistry::component(self::AREA, static fn (array $props): LineChart => self::lineChart($props, true));
        TemplateRegistry::component(self::BAR, static fn (array $props): BarChart => self::barChart($props));
        TemplateRegistry::component(self::DONUT, static fn (array $props): DonutChart => self::donutChart($props));
        TemplateRegistry::component(self::SPARKLINE, static fn (array $props): Sparkline => self::sparkline($props));
        TemplateRegistry::component(self::PROGRESS_RING, static fn (array $props): ProgressRing => self::progressRing($props));
    }

    public function boot(): void
    {
    }

    /**
     * @param array<string, mixed> $props
     */
    public static function lineChart(array $props, bool $area): LineChart
    {
        $chart = LineChart::make(Props::float($props['width'] ?? null, 328.0), Props::float($props['height'] ?? null, 160.0))
            ->curve(Props::curve($props['curve'] ?? null))
            ->tension(Props::float($props['tension'] ?? null, 0.4))
            ->lineWidth(Props::float($props['lineWidth'] ?? null, LineChart::LINE_WIDTH))
            ->markers(Props::bool($props['markers'] ?? null), Props::float($props['pointRadius'] ?? null, 3.0), Props::color($props['pointBackground'] ?? null))
            ->areaOpacity(Props::float($props['areaStartOpacity'] ?? null, 0.35), Props::float($props['areaEndOpacity'] ?? null, 0.0))
            ->area(Props::bool($props['area'] ?? null, $area));

        return self::cartesian($chart, $props);
    }

    /**
     * @param array<string, mixed> $props
     */
    public static function barChart(array $props): BarChart
    {
        $chart = BarChart::make(Props::float($props['width'] ?? null, 328.0), Props::float($props['height'] ?? null, 160.0))
            ->barRadius(Props::float($props['barRadius'] ?? null, 4.0));

        if (Props::bool($props['stacked'] ?? null)) {
            $chart = $chart->stacked();
        }

        return self::cartesian($chart, $props);
    }

    /**
     * @param array<string, mixed> $props
     */
    public static function donutChart(array $props): DonutChart
    {
        $chart = DonutChart::make(Props::float($props['size'] ?? null, 160.0))
            ->slices(Props::slices($props['slices'] ?? null))
            ->gap(Props::float($props['gap'] ?? null, 2.0))
            ->center(Props::string($props['title'] ?? null), Props::string($props['subtitle'] ?? null))
            ->legend(Props::legend($props['legend'] ?? null));
        $chart = $chart->legendValues(Props::bool($props['legendValues'] ?? null, true))
            ->legendSpacing(Props::float($props['legendSpacing'] ?? null, DonutChart::LEGEND_ROW_HEIGHT))
            ->legendInline(Props::bool($props['legendInline'] ?? null));
        $chart = $chart->thickness(Props::float($props['thickness'] ?? null, 18.0));
        $theme = Props::theme($props['theme'] ?? null);

        return $theme === null ? $chart : $chart->theme($theme);
    }

    /**
     * @param array<string, mixed> $props
     */
    public static function sparkline(array $props): Sparkline
    {
        $chart = Sparkline::make(Props::float($props['width'] ?? null, 96.0), Props::float($props['height'] ?? null, 32.0))
            ->values(Props::floatList($props['values'] ?? null))
            ->curve(Props::curve($props['curve'] ?? null))
            ->area(Props::bool($props['area'] ?? null))
            ->endMarker(Props::bool($props['endMarker'] ?? null));
        $color = Props::color($props['color'] ?? null);
        $chart = $color === null ? $chart : $chart->color($color);
        $theme = Props::theme($props['theme'] ?? null);

        return $theme === null ? $chart : $chart->theme($theme);
    }

    /**
     * @param array<string, mixed> $props
     */
    public static function progressRing(array $props): ProgressRing
    {
        $chart = ProgressRing::make(Props::float($props['size'] ?? null, 56.0))
            ->progress(Props::float($props['progress'] ?? null))
            ->thickness(Props::float($props['thickness'] ?? null, 6.0))
            ->label(Props::string($props['label'] ?? null));
        $color = Props::color($props['color'] ?? null);
        $chart = $color === null ? $chart : $chart->color($color);
        $track = Props::color($props['track'] ?? null);
        $chart = $track === null ? $chart : $chart->track($track);
        $theme = Props::theme($props['theme'] ?? null);

        return $theme === null ? $chart : $chart->theme($theme);
    }

    /**
     * @template T of CartesianChart
     * @param T $chart
     * @param array<string, mixed> $props
     * @return T
     */
    private static function cartesian(CartesianChart $chart, array $props): CartesianChart
    {
        foreach (Props::series($props['series'] ?? $props['values'] ?? null) as $series) {
            $chart = $chart->series($series);
        }

        $chart = $chart
            ->labels(Props::stringList($props['labels'] ?? null))
            ->grid(Props::int($props['grid'] ?? null, 3))
            ->beginAtZero(Props::bool($props['beginAtZero'] ?? null, $chart instanceof BarChart))
            ->axis(Props::bool($props['axis'] ?? null, true))
            ->valueLabels(Props::bool($props['valueLabels'] ?? null))
            ->tooltip(Props::bool($props['tooltip'] ?? null))
            ->legend(Props::legend($props['legend'] ?? null))
            ->highlight(Props::nullableInt($props['highlight'] ?? null));

        $formatter = Props::formatter($props['valueFormatter'] ?? null);
        $chart = $formatter === null ? $chart : $chart->valueFormatter($formatter);
        $theme = Props::theme($props['theme'] ?? null);
        $chart = $theme === null ? $chart : $chart->theme($theme);
        $select = Props::event($props, 'select');

        return $select === null
            ? $chart
            : $chart->onSelect(static function (?int $index) use ($select): void {
                $select($index);
            });
    }
}
