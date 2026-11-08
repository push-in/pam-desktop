<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts;

/**
 * Entry point of the library: registers the chart template tags once per process.
 *
 * Call Charts::register() in index.php before App::run(). The drawing protocol
 * uses native GTK, Core Graphics and Direct2D surfaces through pam-desktop-canvas.
 */
final class Charts
{
    private static bool $registered = false;

    private function __construct()
    {
    }

    /** Registers `<LineChart>`, `<AreaChart>`, `<BarChart>`, `<DonutChart>`, `<Sparkline>` and `<ProgressRing>`. */
    public static function register(): void
    {
        if (self::$registered) {
            return;
        }
        (new ChartsPluginProvider())->register();
        self::$registered = true;
    }

    /** Forgets the registration flag (tests). */
    public static function reset(): void
    {
        self::$registered = false;
    }
}
