<?php

declare(strict_types=1);

namespace Pam\Desktop;

use function array_key_exists;
use function getenv;
use function in_array;
use function is_string;

use const PHP_OS_FAMILY;

/**
 * The desktop operating system the application runs on.
 *
 * The native host exports it before PHP starts, so style queries
 * (`@media (platform: macos)`) and application code agree with the window
 * that is actually on screen.
 */
final class Platform
{
    public const string Linux = 'linux';
    public const string MacOS = 'macos';
    public const string Windows = 'windows';

    public const string ENVIRONMENT = 'PAM_DESKTOP_PLATFORM';

    private static ?string $current = null;

    private function __construct()
    {
    }

    /** `linux`, `macos` or `windows`. */
    public static function os(): string
    {
        if (self::$current !== null) {
            return self::$current;
        }
        $exported = getenv(self::ENVIRONMENT);
        if (is_string($exported) && in_array($exported, [self::Linux, self::MacOS, self::Windows], true)) {
            return self::$current = $exported;
        }

        return self::$current = match (PHP_OS_FAMILY) {
            'Darwin' => self::MacOS,
            'Windows' => self::Windows,
            default => self::Linux,
        };
    }

    public static function isLinux(): bool
    {
        return self::os() === self::Linux;
    }

    public static function isMacOS(): bool
    {
        return self::os() === self::MacOS;
    }

    public static function isWindows(): bool
    {
        return self::os() === self::Windows;
    }

    /**
     * Picks the value for the current system, like React Native's
     * `Platform.select`: `Platform::select(['macos' => 'Cmd', 'default' => 'Ctrl'])`.
     *
     * @template T
     * @param array<string, T> $values keyed by `linux`, `macos`, `windows` and optionally `default`
     * @return T|null
     */
    public static function select(array $values): mixed
    {
        $os = self::os();
        if (array_key_exists($os, $values)) {
            return $values[$os];
        }

        return $values['default'] ?? null;
    }

    /** @internal Tests and tooling pin the platform. */
    public static function use(?string $os): void
    {
        self::$current = $os;
    }
}
