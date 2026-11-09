<?php

declare(strict_types=1);

namespace Pam\Desktop\System;

use Pam\Desktop\Internal\DependencyTracker;
use Pam\Desktop\Internal\Runtime;
use Pam\Desktop\Modules\NativeModuleResult;
use Pam\Desktop\Modules\NativeModules;
use stdClass;

/**
 * Single-line text widths measured by the host, for code that lays text out
 * in PHP (chart axes and legends, canvas labels). The first read of a string
 * returns null and asks the host once; when the width arrives, the components
 * that read it render again. Widths stay in a bounded process cache.
 *
 * ```php
 * $width = TextMeasure::width('Hambúrguer', 12, 400, 'Inter, Arial, sans-serif') ?? $estimate;
 * ```
 */
final class TextMeasure
{
    private const int LIMIT = 2048;

    /** @var array<string, float> */
    private static array $widths = [];

    /** @var array<string, true> */
    private static array $pending = [];

    private static ?stdClass $source = null;

    private function __construct()
    {
    }

    public static function width(string $text, float $size, int $weight = 400, string $family = ''): ?float
    {
        if ($text === '' || $size < 1.0 || $size > 512.0 || strlen($text) > 1024 || str_contains($text, "\n") || strlen($family) > 256) {
            return null;
        }
        $key = $family . "\0" . $size . "\0" . $weight . "\0" . $text;
        if (isset(self::$widths[$key])) {
            return self::$widths[$key];
        }
        self::$source ??= new stdClass();
        DependencyTracker::read(self::$source, 'widths');
        if (!isset(self::$pending[$key]) && count(self::$pending) < 256) {
            self::$pending[$key] = true;
            NativeModules::call('text', 'measure', ['texts' => $text, 'size' => $size, 'weight' => $weight, 'family' => $family], static function (NativeModuleResult $result) use ($key): void {
                unset(self::$pending[$key]);
                $widths = $result->succeeded() ? (string) ($result->values()['widths'] ?? '') : '';
                if (!is_numeric($widths)) {
                    return;
                }
                if (count(self::$widths) >= self::LIMIT) {
                    self::$widths = array_slice(self::$widths, intdiv(self::LIMIT, 2), null, true);
                }
                self::$widths[$key] = (float) $widths;
                if (self::$source !== null) {
                    DependencyTracker::invalidate(self::$source, 'widths');
                    Runtime::scheduleRender();
                }
            }, static function (): void {
                // Hosts without the measurer (tests, older hosts) keep the caller's estimate.
            });
        }

        return null;
    }

    /** @internal Test seam: a known width, as if the host had measured it. */
    public static function remember(string $text, float $size, int $weight, string $family, float $width): void
    {
        self::$widths[$family . "\0" . $size . "\0" . $weight . "\0" . $text] = $width;
    }
}
