<?php

declare(strict_types=1);

namespace Pam\Desktop\System;

use Closure;
use Pam\Desktop\Internal\Runtime;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\ModuleResultStatus;
use Pam\Desktop\Modules\NativeModules;
use Pam\Desktop\NativeOperation;
use InvalidArgumentException;
use RuntimeException;

use function count;
use function is_string;

final class Share
{
    private function __construct()
    {
    }

    /**
     * Shares plain text through the share sheet of the system (macOS and
     * Windows). Linux has none: the call fails with "Sharing is not available
     * on this platform".
     *
     * `$opened` fires once the sheet was presented. When the native side
     * fails, `$failure` receives its message; without `$failure` the failure
     * is ignored (no exception).
     *
     * @param null|Closure(): void $opened
     * @param null|Closure(string): void $failure
     */
    public static function text(
        string $text,
        ?string $title = null,
        ?Closure $opened = null,
        ?Closure $failure = null,
    ): int {
        return Runtime::callNative(
            NativeOperation::Share,
            Wire::map([
                'text' => $text,
                'title' => $title ?? '',
            ]),
            static function (ModuleResultStatus $status, string $payload) use ($opened, $failure): void {
                if ($status === ModuleResultStatus::Failure) {
                    $failure?->__invoke($payload);

                    return;
                }
                $opened?->__invoke();
            },
        );
    }

    /**
     * Whether an application package is installed. Unsupported on desktop:
     * the callback receives null.
     * @param Closure(?bool): void $callback
     * @param null|Closure(string): void $failure
     */
    public static function isPackageInstalled(string $packageName, Closure $callback, ?Closure $failure = null): int
    {
        self::validatePackage($packageName);

        return NativeModules::call('files', 'isPackageInstalled', ['packageName' => $packageName],
            static function ($result) use ($callback, $failure): void {
                if ($result->status === ModuleResultStatus::Failure) {
                    if ($failure !== null) { $failure($result->payload); return; }
                    throw new RuntimeException($result->payload);
                }
                $values = Wire::decodeMap($result->payload);
                $callback(($values['supported'] ?? false) ? (bool) ($values['installed'] ?? false) : null);
            },
        );
    }

    /** Shares a file with one application package. Fails on desktop: the user picks the application in the sheet. */
    public static function fileToPackage(
        string $path,
        string $packageName,
        ?string $mimeType = null,
        ?string $text = null,
        ?Closure $opened = null,
        ?Closure $failure = null,
    ): int {
        self::validatePackage($packageName);

        return self::files([$path], $mimeType, opened: $opened, failure: $failure, text: $text, packageName: $packageName);
    }

    private static function validatePackage(string $packageName): void
    {
        if (strlen($packageName) > 255 || preg_match('/^[A-Za-z][A-Za-z0-9_]*(\\.[A-Za-z][A-Za-z0-9_]*)+$/D', $packageName) !== 1) {
            throw new InvalidArgumentException('Invalid package name.');
        }
    }

    /**
     * Shares private sandbox files through the share sheet of the system
     * (macOS and Windows). Linux has none: the call fails with "Sharing is
     * not available on this platform". `$packageName` targets one application
     * and is refused on desktop, where the user picks it in the sheet.
     *
     * `$text` (optional) travels beside the files, so a picture can be shared
     * with its caption/link.
     *
     * @param list<string> $paths Relative private file paths.
     * @param null|Closure(): void $opened
     * @param null|Closure(string): void $failure
     */
    public static function files(
        array $paths,
        ?string $mimeType = null,
        ?string $title = null,
        ?Closure $opened = null,
        ?Closure $failure = null,
        ?string $text = null,
        ?string $packageName = null,
    ): int {
        if ($packageName !== null) self::validatePackage($packageName);
        if ($paths === [] || count($paths) > 50 || !array_is_list($paths)) {
            throw new InvalidArgumentException('Share between 1 and 50 private files.');
        }
        $normalized = array_map(
            static fn (mixed $path): string => is_string($path)
                ? Files::privatePath($path)
                : throw new InvalidArgumentException('Shared file paths must be strings.'),
            $paths,
        );
        $mimeType = trim((string) $mimeType);
        if ($mimeType !== '' && preg_match('#^[a-z0-9!\#$&^_.+-]+/[a-z0-9!\#$&^_.+*-]+$#iD', $mimeType) !== 1) {
            throw new InvalidArgumentException('Share MIME type is invalid.');
        }

        return NativeModules::call(
            'files',
            'share',
            [
                'paths' => json_encode($normalized, JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES),
                'mimeType' => $mimeType,
                'title' => $title ?? '',
                'text' => $text ?? '',
                'packageName' => $packageName ?? '',
            ],
            static function ($result) use ($opened, $failure): void {
                if ($result->status === ModuleResultStatus::Failure) {
                    if ($failure !== null) {
                        $failure($result->payload);

                        return;
                    }
                    throw new RuntimeException($result->payload);
                }
                $opened?->__invoke();
            },
        );
    }
}
