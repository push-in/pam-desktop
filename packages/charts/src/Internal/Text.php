<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts\Internal;

/**
 * Text measurement estimates for layout. Native text metrics are not available
 * when the display list is built, so widths are approximated from glyph counts.
 *
 * @internal
 */
final class Text
{
    private const float AVERAGE_GLYPH_WIDTH = 0.58;

    private function __construct()
    {
    }

    /** Estimated rendered width in the same units as `$size`. */
    public static function width(string $text, float $size): float
    {
        return mb_strlen($text) * $size * self::AVERAGE_GLYPH_WIDTH;
    }

    /**
     * Advance widths (font units of 2048 per em) of U+0020..U+00FF in fonts
     * whose metrics are public and shared by metric-compatible families
     * (Arial = Liberation Sans = Arimo; DejaVu Sans is the Linux UI font).
     */
    private const array ADVANCES = [
        'arial' => '569,569,727,1139,1139,1821,1366,391,682,682,797,1196,569,682,569,569,1139,1139,1139,1139,1139,1139,1139,1139,1139,1139,569,569,1196,1196,1196,1139,2079,1366,1366,1479,1479,1366,1251,1593,1479,569,1024,1366,1139,1706,1479,1593,1366,1593,1479,1366,1251,1479,1366,1933,1366,1366,1251,569,569,569,961,1139,682,1139,1139,1024,1139,1139,569,1139,1139,455,455,1024,455,1706,1139,1139,1139,1139,682,1024,569,1139,1024,1479,1024,1024,1024,684,532,684,1196,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,569,682,1139,1139,1139,1139,532,1139,682,1509,758,1139,1196,682,1509,1131,819,1124,682,682,682,1180,1100,569,682,682,748,1139,1708,1708,1708,1251,1366,1366,1366,1366,1366,1366,2048,1479,1366,1366,1366,1366,569,569,569,569,1479,1479,1593,1593,1593,1593,1593,1196,1593,1479,1479,1479,1479,1366,1366,1251,1139,1139,1139,1139,1139,1139,1821,1024,1139,1139,1139,1139,569,569,569,569,1139,1139,1139,1139,1139,1139,1139,1124,1251,1139,1139,1139,1139,1024,1139,1024',
        'arialBold' => '569,682,971,1139,1139,1821,1479,487,682,682,797,1196,569,682,569,569,1139,1139,1139,1139,1139,1139,1139,1139,1139,1139,682,682,1196,1196,1196,1251,1997,1479,1479,1479,1479,1366,1251,1593,1479,569,1139,1479,1251,1706,1479,1593,1366,1593,1479,1366,1251,1479,1366,1933,1366,1366,1251,682,569,682,1196,1139,682,1139,1251,1139,1251,1139,682,1251,1251,569,569,1139,569,1821,1251,1251,1251,1251,797,1139,682,1251,1139,1593,1139,1139,1024,797,573,797,1196,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,569,682,1139,1139,1139,1139,573,1139,682,1509,758,1139,1196,682,1509,1131,819,1124,682,682,682,1180,1139,569,682,682,748,1139,1708,1708,1708,1251,1479,1479,1479,1479,1479,1479,2048,1479,1366,1366,1366,1366,569,569,569,569,1479,1479,1593,1593,1593,1593,1593,1196,1593,1479,1479,1479,1479,1366,1366,1251,1139,1139,1139,1139,1139,1139,1821,1139,1139,1139,1139,1139,569,569,569,569,1251,1251,1251,1251,1251,1251,1251,1124,1251,1251,1251,1251,1251,1139,1251,1139',
        'dejavu' => '651,821,942,1716,1303,1946,1597,563,799,799,1024,1716,651,739,651,690,1303,1303,1303,1303,1303,1303,1303,1303,1303,1303,690,690,1716,1716,1716,1087,2048,1401,1405,1430,1577,1294,1178,1587,1540,604,604,1343,1141,1767,1532,1612,1235,1612,1423,1300,1251,1499,1401,2025,1403,1251,1403,799,690,799,1716,1024,1024,1255,1300,1126,1300,1260,721,1300,1298,569,569,1186,569,1995,1298,1253,1300,1300,842,1067,803,1298,1212,1675,1212,1212,1075,1303,690,1303,1716,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,651,821,1303,1303,1303,1303,690,1024,1024,2048,965,1253,1716,739,2048,1024,1024,1716,821,821,1024,1303,1303,651,1024,821,965,1253,1985,1985,1985,1087,1401,1401,1401,1401,1401,1401,1995,1430,1294,1294,1294,1294,604,604,604,604,1587,1532,1612,1612,1612,1612,1612,1716,1612,1499,1499,1499,1499,1251,1239,1290,1255,1255,1255,1255,1255,1255,2011,1126,1260,1260,1260,1260,569,569,569,569,1253,1298,1253,1253,1253,1253,1253,1716,1253,1298,1298,1298,1298,1212,1300,1212',
        'dejavuBold' => '713,934,1067,1716,1425,2052,1786,627,936,936,1071,1716,778,850,778,748,1425,1425,1425,1425,1425,1425,1425,1425,1425,1425,819,819,1716,1716,1716,1188,2048,1585,1561,1503,1700,1399,1399,1681,1714,762,762,1587,1305,2038,1714,1741,1501,1741,1577,1475,1397,1663,1585,2259,1579,1483,1485,936,748,936,1716,1024,1024,1382,1466,1214,1466,1389,891,1466,1458,702,702,1362,702,2134,1458,1407,1466,1466,1010,1219,979,1458,1335,1892,1321,1335,1192,1458,748,1458,1716,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,713,934,1425,1425,1303,1425,748,1024,1024,2048,1155,1323,1716,850,2048,1024,1024,1716,897,897,1024,1507,1303,778,1024,897,1155,1323,2120,2120,2120,1188,1585,1585,1585,1585,1585,1585,2222,1503,1399,1399,1399,1399,762,762,762,762,1716,1714,1741,1741,1741,1741,1741,1716,1741,1663,1663,1663,1663,1483,1511,1473,1382,1382,1382,1382,1382,1382,2146,1214,1389,1389,1389,1389,702,702,702,702,1407,1458,1407,1407,1407,1407,1407,1716,1407,1458,1458,1458,1458,1335,1466,1335',
    ];

    /** CSS families that resolve to a table above. */
    private const array FAMILIES = [
        'arial' => 'arial', 'helvetica' => 'arial', 'helvetica neue' => 'arial', 'liberation sans' => 'arial', 'arimo' => 'arial',
        'dejavu sans' => 'dejavu', 'sans-serif' => 'dejavu', 'system-ui' => 'dejavu',
    ];

    /**
     * Width of `$text` in the first family of the CSS list `$family` that has
     * known metrics (Chromium falls back the same way when a family is not
     * installed), from the font's own advances; the glyph-count estimate
     * otherwise. Weights from 600 use the bold face, like Chromium does for
     * fonts without a semibold. The host's measurement
     * ({@see \Pam\Desktop\System\TextMeasure}) wins once it has answered:
     * it resolves the family like Chromium and shapes the text (kerning,
     * which canvas `measureText` applies: "7 de nov." in Arial Bold 11px is
     * 46.86, the advance sum 47.68); the table and the estimate only cover
     * the first render and hosts that cannot measure.
     */
    public static function measure(string $text, float $size, int $weight = 400, string $family = ''): float
    {
        $measured = \Pam\Desktop\System\TextMeasure::width($text, $size, $weight, $family === '' ? 'sans-serif' : $family);
        if ($measured !== null) {
            return $measured;
        }
        $table = null;
        foreach (explode(',', $family === '' ? 'sans-serif' : $family) as $name) {
            $name = strtolower(trim($name, " \t\"'"));
            if (isset(self::FAMILIES[$name])) {
                $table = self::FAMILIES[$name] . ($weight >= 600 ? 'Bold' : '');
                break;
            }
        }
        if ($table === null) {
            // Other fonts: the estimate until the host answers (asked once,
            // cached; the chart's component renders again then).
            return self::width($text, $size);
        }
        static $parsed = [];
        $advances = $parsed[$table] ??= array_map('intval', explode(',', self::ADVANCES[$table]));
        $units = 0;
        foreach (mb_str_split($text) as $character) {
            $code = mb_ord($character);
            $advance = $code >= 32 && $code <= 255 ? $advances[$code - 32] : 0;
            $units += $advance > 0 ? $advance : 1188; // ~0.58 em for glyphs outside the table
        }

        return $units / 2048 * $size;
    }

    /** Ascent of the first known family of `$family` (font units of 2048 per em). */
    public static function ascent(float $size, string $family = ''): float
    {
        foreach (explode(',', $family === '' ? 'sans-serif' : $family) as $name) {
            $name = strtolower(trim($name, " \t\"'"));
            if (isset(self::FAMILIES[$name])) {
                return (self::FAMILIES[$name] === 'arial' ? 1854 : 1901) / 2048 * $size;
            }
        }

        return 0.9 * $size;
    }

    /**
     * Offset from a canvas `textBaseline: 'middle'` point down to the
     * alphabetic baseline, as Blink computes it: the middle of the em box
     * made of the OS/2 typo ascent and descent normalised to the font size,
     * each a LayoutUnit (1/64 px). Arial 12px: (9.3125 - 2.6875) / 2.
     */
    public static function middle(float $size, string $family = ''): float
    {
        [$ascent, $descent] = [1491, 431]; // Arial / Liberation Sans OS/2 typo metrics
        foreach (explode(',', $family === '' ? 'sans-serif' : $family) as $name) {
            $name = strtolower(trim($name, " \t\"'"));
            if (isset(self::FAMILIES[$name])) {
                [$ascent, $descent] = self::FAMILIES[$name] === 'arial' ? [1491, 431] : [1556, 492];
                break;
            }
        }
        $top = round($size * $ascent / ($ascent + $descent) * 64) / 64;

        return ($top - ($size - $top)) / 2;
    }

    /** Baseline offset that visually centers text of `$size` on a vertical center. */
    public static function baselineOffset(float $size): float
    {
        return $size * 0.35;
    }

    /** Default number formatting: thousands separator, no decimals for integers. */
    public static function number(float $value): string
    {
        $decimals = abs($value - round($value)) < 1e-9 || abs($value) >= 100 ? 0 : 1;

        return number_format($value, $decimals, ',', '.');
    }
}
