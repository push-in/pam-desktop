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
            $key = hash('sha256', $data).'|'.$size.'|'.$level->value;
            if (isset(self::$cache[$key])) return self::$cache[$key];
            $code = new QrCode($data, $size, $level);
            if (count(self::$cache) >= 4) array_shift(self::$cache);
            self::$cache[$key] = $code;
            return $code;
        });
        self::$registered = true;
    }
    /** Release retained scenes when the application clears sensitive UI data. */
    public static function clearCache(): void { self::$cache = []; }
}
