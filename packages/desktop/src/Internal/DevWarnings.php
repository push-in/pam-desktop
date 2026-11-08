<?php

declare(strict_types=1);

namespace Pam\Desktop\Internal;

use Pam\Desktop\BuildConfiguration;
use Pam\Desktop\BuildMode;

use function count;

/**
 * Development-only diagnostics, reported once per source location.
 *
 * @internal
 */
final class DevWarnings
{
    private const MAX_MESSAGES = 64;

    /** @var array<string, true> */
    private static array $reported = [];

    /** @var list<string> */
    private static array $messages = [];

    private function __construct()
    {
    }

    public static function warn(string $message, string $location): void
    {
        $fingerprint = $location.'|'.$message;
        if (isset(self::$reported[$fingerprint]) || count(self::$reported) >= self::MAX_MESSAGES) {
            return;
        }
        if (BuildConfiguration::mode() !== BuildMode::Development) {
            return;
        }
        self::$reported[$fingerprint] = true;
        $line = "Pam Desktop warning: {$message} ({$location}).";
        self::$messages[] = $line;
        error_log($line);
    }

    /** @return list<string> */
    public static function messages(): array
    {
        return self::$messages;
    }

    public static function reset(): void
    {
        self::$reported = [];
        self::$messages = [];
    }
}
