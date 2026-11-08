<?php

declare(strict_types=1);

namespace Pam\Desktop\Canvas\Internal;

use InvalidArgumentException;
use Pam\Desktop\Canvas\CanvasScene;
use Pam\Desktop\Canvas\DisplayCommandKind as Kind;
use Pam\Desktop\Canvas\Path;

/** Adapts the shared immutable scene vocabulary to the native desktop drawing protocol. */
final class DesktopCommands
{
    public static function encode(CanvasScene $scene): string
    {
        $commands = [];
        foreach ($scene->commands as $command) {
            $a = $command->arguments;
            $entries = match ($command->kind) {
                Kind::FillRect => [['kind' => 1, 'x' => $a[0], 'y' => $a[1], 'width' => $a[2], 'height' => $a[3], 'color' => self::color($a[4])]],
                Kind::RoundRect => [['kind' => 2, 'x' => $a[0], 'y' => $a[1], 'width' => $a[2], 'height' => $a[3], 'radius' => $a[4], 'color' => self::color($a[5])]],
                Kind::Circle => [['kind' => 3, 'centerX' => $a[0], 'centerY' => $a[1], 'radius' => $a[2], 'color' => self::color($a[3])]],
                Kind::Line => [['kind' => 4, 'startX' => $a[0], 'startY' => $a[1], 'endX' => $a[2], 'endY' => $a[3], 'color' => self::color($a[4]), 'width' => $a[5]]],
                Kind::Path => [['kind' => 5, 'path' => Path::assert($a[0]), $a[3] === 2 ? 'stroke' : 'fill' => self::color($a[1]), 'width' => $a[2]]],
                Kind::GradientPath => [['kind' => 5, 'path' => Path::assert($a[0]), 'gradient' => ['x0' => $a[1], 'y0' => $a[2], 'x1' => $a[3], 'y1' => $a[4], 'start' => self::color($a[5]), 'end' => self::color($a[6])]]],
                Kind::Text, Kind::Label => [['kind' => 6, 'text' => $a[0], 'x' => $a[1], 'y' => $a[2], 'size' => $a[3], 'color' => self::color($a[4]), 'align' => $a[5] ?? 1, 'weight' => match ($a[6] ?? 1) { 2 => 500, 3 => 600, 4 => 700, default => 400 }]],
                Kind::Arc => self::arc($a),
                Kind::Sector => self::sector($a),
                Kind::Polygon, Kind::Polyline => self::points($a, $command->kind === Kind::Polygon),
                Kind::DashedLine => self::dashes($a),
                default => throw new InvalidArgumentException('Unsupported desktop canvas command '.$command->kind->name.'.'),
            };
            array_push($commands, ...$entries);
            if (count($commands) > CanvasScene::MAX_COMMANDS) throw new InvalidArgumentException('Canvas scene expands beyond 10,000 primitives.');
        }
        $json = json_encode($commands, JSON_THROW_ON_ERROR | JSON_UNESCAPED_UNICODE);
        if (strlen($json) > 1_000_000) throw new InvalidArgumentException('Canvas scene exceeds 1 MiB.');
        return $json;
    }

    private static function color(string $value): int
    {
        $hex = substr($value, 1);
        if (strlen($hex) === 3) $hex = $hex[0].$hex[0].$hex[1].$hex[1].$hex[2].$hex[2];
        if (strlen($hex) === 6) $hex .= 'ff';
        if (strlen($hex) !== 8 || preg_match('/^[0-9a-f]+$/iD', $hex) !== 1) throw new InvalidArgumentException('Invalid canvas color.');
        return (hexdec(substr($hex, 6, 2)) << 24) | hexdec(substr($hex, 0, 6));
    }

    /** @param list<float|int|string> $a @return list<array<string, mixed>> */
    private static function arc(array $a): array
    {
        [$cx, $cy, $radius, $start, $sweep, $color, $width, $cap] = $a;
        if ($radius <= 0 || $sweep == 0) return [];
        $path = self::arcPath(null, $cx, $cy, $radius, $start, $sweep);
        $entries = [['kind' => 5, 'path' => $path->spec(), 'stroke' => self::color($color), 'width' => $width]];
        if ($cap === 2 && abs($sweep) < 360) {
            foreach ([$start, $start + $sweep] as $degrees) {
                $radians = deg2rad($degrees - 90);
                $entries[] = ['kind' => 3, 'centerX' => $cx + cos($radians) * $radius, 'centerY' => $cy + sin($radians) * $radius, 'radius' => $width / 2, 'color' => self::color($color)];
            }
        }
        return $entries;
    }

    /** @param list<float|int|string> $a @return list<array<string, mixed>> */
    private static function sector(array $a): array
    {
        [$cx, $cy, $outer, $inner, $start, $sweep, $color] = $a;
        if ($outer <= 0 || $sweep == 0) return [];
        $sweep = max(-360.0, min(360.0, $sweep));
        $inner = min($outer, max(0.0, $inner));
        $path = self::arcPath(null, $cx, $cy, $outer, $start, $sweep);
        if ($inner > 0) {
            $angle = deg2rad($start + $sweep - 90);
            $path = $path->lineTo($cx + cos($angle) * $inner, $cy + sin($angle) * $inner);
            $path = self::arcPath($path, $cx, $cy, $inner, $start + $sweep, -$sweep);
        } else { $path = $path->lineTo($cx, $cy); }
        return [['kind' => 5, 'path' => $path->close()->spec(), 'fill' => self::color($color)]];
    }

    private static function arcPath(?Path $path, float $cx, float $cy, float $radius, float $start, float $sweep): Path
    {
        $angle = deg2rad($start - 90);
        $sweep = deg2rad(max(-360, min(360, $sweep)));
        $count = max(1, (int) ceil(abs($sweep) / (M_PI / 2)));
        $delta = $sweep / $count;
        $path ??= Path::start($cx + cos($angle) * $radius, $cy + sin($angle) * $radius);
        for ($i = 0; $i < $count; $i++) {
            $end = $angle + $delta;
            $tangent = 4 / 3 * tan($delta / 4);
            $path = $path->curveTo(
                $cx + (cos($angle) - $tangent * sin($angle)) * $radius,
                $cy + (sin($angle) + $tangent * cos($angle)) * $radius,
                $cx + (cos($end) + $tangent * sin($end)) * $radius,
                $cy + (sin($end) - $tangent * cos($end)) * $radius,
                $cx + cos($end) * $radius, $cy + sin($end) * $radius,
            );
            $angle = $end;
        }
        return $path;
    }

    /** @param list<float|int|string> $a @return list<array<string, mixed>> */
    private static function points(array $a, bool $fill): array
    {
        $points = explode(' ', $a[0]);
        $path = null;
        foreach ($points as $point) {
            [$x, $y] = array_map(floatval(...), explode(',', $point));
            $path = $path === null ? Path::start($x, $y) : $path->lineTo($x, $y);
        }
        if ($path === null) return [];
        $entries = [['kind' => 5, 'path' => ($fill ? $path->close() : $path)->spec(), $fill ? 'fill' : 'stroke' => self::color($a[1]), 'width' => $a[2] ?? 0]];
        if (!$fill && ($a[3] ?? 1) === 2) {
            foreach ([$points[0], $points[count($points) - 1]] as $point) {
                [$x, $y] = array_map(floatval(...), explode(',', $point));
                $entries[] = ['kind' => 3, 'centerX' => $x, 'centerY' => $y, 'radius' => $a[2] / 2, 'color' => self::color($a[1])];
            }
        }
        return $entries;
    }

    /** @param list<float|int|string> $a @return list<array<string, mixed>> */
    private static function dashes(array $a): array
    {
        [$x1, $y1, $x2, $y2, $color, $width, $dash, $gap] = $a;
        $length = hypot($x2 - $x1, $y2 - $y1);
        if ($length <= 0) return [];
        if ($dash <= 0 || $gap <= 0 || ceil($length / ($dash + $gap)) > 1000) throw new InvalidArgumentException('Canvas dashed line exceeds 1000 segments.');
        $entries = [];
        for ($at = 0.0; $at < $length; $at += $dash + $gap) {
            $end = min($length, $at + $dash);
            $path = Path::start($x1 + ($x2 - $x1) * $at / $length, $y1 + ($y2 - $y1) * $at / $length)
                ->lineTo($x1 + ($x2 - $x1) * $end / $length, $y1 + ($y2 - $y1) * $end / $length);
            $entries[] = ['kind' => 5, 'path' => $path->spec(), 'stroke' => self::color($color), 'width' => $width];
        }
        return $entries;
    }
}
