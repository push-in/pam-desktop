<?php

declare(strict_types=1);

namespace Pam\Desktop\System;

use InvalidArgumentException;
use Pam\Desktop\Modules\NativeModuleResult;
use Pam\Desktop\Modules\NativeModules;

/**
 * Short messages shown over the application by the host. One toast shows at
 * a time; a new one replaces it. Pressing a toast dismisses it.
 *
 * ```php
 * Toast::show('Copied');
 * Toast::success('Saved', 'Your changes are safe.');
 * Toast::message('No connection', 'Trying again in 5 s', ToastType::Error, ToastPosition::Bottom);
 * ```
 */
final class Toast
{
    private function __construct()
    {
    }

    /** A plain capsule near the bottom of the window. */
    public static function show(string $message, bool $long = false): void
    {
        if (trim($message) === '') {
            throw new InvalidArgumentException('A toast needs a message.');
        }
        self::send(['message' => $message, 'long' => $long]);
    }

    /**
     * A card with a title, an optional message and an accent strip.
     *
     * @param int $durationMs how long it stays, 500 to 60000
     * @param float $offset distance from the edge of the window
     * @param int|null $accentColor ARGB; the colour of `$type` when null
     * @param string|null $iconGlyph a glyph of `$iconFontFamily` shown before the text
     */
    public static function message(
        string $title,
        string $message = '',
        ToastType $type = ToastType::Info,
        ToastPosition $position = ToastPosition::Top,
        int $durationMs = 4000,
        float $offset = 40.0,
        ?int $accentColor = null,
        int $backgroundColor = 0xFFFFFFFF,
        int $titleColor = 0xFF000000,
        int $messageColor = 0xFF979797,
        float $titleSize = 13.0,
        float $messageSize = 12.0,
        float $cornerRadius = 8.0,
        bool $fitContent = false,
        ?string $iconGlyph = null,
        ?string $iconFontFamily = null,
        ?string $iconSource = null,
        ?int $iconColor = null,
    ): void {
        if (trim($title) === '') {
            throw new InvalidArgumentException('A toast needs a title.');
        }
        $payload = [
            'title' => $title,
            'message' => $message,
            'bottom' => $position === ToastPosition::Bottom,
            'durationMs' => max(500, min(60_000, $durationMs)),
            'offset' => max(0.0, $offset),
            'accentColor' => $accentColor ?? $type->accentColor(),
            'backgroundColor' => $backgroundColor,
            'titleColor' => $titleColor,
            'messageColor' => $messageColor,
            'titleSize' => max(1.0, $titleSize),
            'messageSize' => max(1.0, $messageSize),
            'cornerRadius' => max(0.0, min(64.0, $cornerRadius)),
            'fitContent' => $fitContent,
        ];
        if ($iconGlyph !== null && $iconGlyph !== '') {
            $payload['iconGlyph'] = $iconGlyph;
            $payload['iconFontFamily'] = $iconFontFamily ?? '';
        } elseif ($iconSource !== null && $iconSource !== '') {
            $payload['iconSource'] = $iconSource;
        }
        if ($iconColor !== null) {
            $payload['iconColor'] = $iconColor;
        }
        self::send($payload);
    }

    public static function success(string $title, string $message = '', ToastPosition $position = ToastPosition::Top): void
    {
        self::message($title, $message, ToastType::Success, $position);
    }

    public static function error(string $title, string $message = '', ToastPosition $position = ToastPosition::Top): void
    {
        self::message($title, $message, ToastType::Error, $position);
    }

    public static function info(string $title, string $message = '', ToastPosition $position = ToastPosition::Top): void
    {
        self::message($title, $message, ToastType::Info, $position);
    }

    /** @param array<string, string|int|float|bool> $payload */
    private static function send(array $payload): void
    {
        NativeModules::call('toast', 'show', $payload, static function (NativeModuleResult $result): void {
        }, static function (): void {
        });
    }
}
