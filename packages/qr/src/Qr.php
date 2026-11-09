<?php

declare(strict_types=1);
namespace Pam\Desktop\Qr;
use InvalidArgumentException;
use Pam\Desktop\TemplateRegistry;

final class Qr
{
    private static bool $registered = false;
    /** @var array<string,QrCode> At most four bounded scenes; keys never retain raw payment data. */
    private static array $cache = [];
    public static function register(): void
    {
        if (self::$registered) return;
        TemplateRegistry::component('QrCode', static function (array $props): QrCode {
            $data = $props['data'] ?? null;
            $size = filter_var($props['size'] ?? 256, FILTER_VALIDATE_INT);
            $level = $props['correction'] ?? ErrorCorrection::Medium;
            if (!$level instanceof ErrorCorrection) {
                $value = is_int($level) ? $level : (is_string($level) ? filter_var($level, FILTER_VALIDATE_INT) : false);
                $level = is_int($value) ? ErrorCorrection::tryFrom($value) : null;
            }
            if (!is_string($data) || !is_int($size) || $level === null) throw new InvalidArgumentException('QrCode requires data, an integer size and a valid ErrorCorrection enum.');
            $margin = filter_var($props['margin'] ?? QrCode::QUIET_MODULES, FILTER_VALIDATE_INT);
            if (!is_int($margin)) throw new InvalidArgumentException('QrCode margin must be an integer number of modules.');
            $stretch = filter_var($props['stretch'] ?? false, FILTER_VALIDATE_BOOL);
            $image = isset($props['image']) ? filter_var($props['image'], FILTER_VALIDATE_INT) : null;
            if ($image === false) throw new InvalidArgumentException('QrCode image must be an integer number of pixels.');
            $key = hash('sha256', $data).'|'.$size.'|'.$level->value.'|'.$margin.'|'.(int) $stretch.'|'.($image ?? 0);
            if (isset(self::$cache[$key])) return self::$cache[$key];
            $code = new QrCode($data, $size, $level, $margin, $stretch, $image);
            if (count(self::$cache) >= 4) array_shift(self::$cache);
            self::$cache[$key] = $code;
            return $code;
        });
        self::$registered = true;
    }
    /** Release retained scenes when the application clears sensitive UI data. */
    public static function clearCache(): void { self::$cache = []; }
}
