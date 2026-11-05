<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Closure;
use Pam\Desktop\Internal\ShellEvents;

/**
 * The native file dialogs of the operating system.
 *
 * ```php
 * Dialog::openFile(
 *     fn (?string $path) => $path && $this->load($path),
 *     filters: [FileFilter::images()],
 * );
 * ```
 *
 * Every dialog reports `null` (or an empty list) when the user cancels.
 */
final class Dialog
{
    private function __construct()
    {
    }

    /**
     * @param Closure(?string): mixed $completed
     * @param list<FileFilter> $filters
     */
    public static function openFile(Closure $completed, string $title = '', array $filters = [], ?string $directory = null): void
    {
        self::pick('openFile', $title, $filters, $directory, static fn (array $paths): mixed => $completed($paths[0] ?? null));
    }

    /**
     * @param Closure(list<string>): mixed $completed
     * @param list<FileFilter> $filters
     */
    public static function openFiles(Closure $completed, string $title = '', array $filters = [], ?string $directory = null): void
    {
        self::pick('openFile', $title, $filters, $directory, $completed, ['multiple' => true]);
    }

    /**
     * @param Closure(?string): mixed $completed
     * @param list<FileFilter> $filters
     */
    public static function saveFile(
        Closure $completed,
        string $defaultName = '',
        string $title = '',
        array $filters = [],
        ?string $directory = null,
    ): void {
        self::pick(
            'saveFile',
            $title,
            $filters,
            $directory,
            static fn (array $paths): mixed => $completed($paths[0] ?? null),
            ['defaultName' => $defaultName],
        );
    }

    /** @param Closure(?string): mixed $completed */
    public static function selectFolder(Closure $completed, string $title = '', ?string $directory = null): void
    {
        self::pick('selectFolder', $title, [], $directory, static fn (array $paths): mixed => $completed($paths[0] ?? null));
    }

    /**
     * @param list<FileFilter> $filters
     * @param Closure(list<string>): mixed $completed
     * @param array<string, bool|float|int|string> $extra
     */
    private static function pick(string $method, string $title, array $filters, ?string $directory, Closure $completed, array $extra = []): void
    {
        $encodedFilters = [];
        foreach ($filters as $filter) {
            $encodedFilters[] = ['name' => $filter->name, 'extensions' => $filter->extensions];
        }
        ShellEvents::send(
            $method,
            [
                'title' => $title,
                'directory' => $directory ?? '',
                'filters' => json_encode($encodedFilters, JSON_UNESCAPED_UNICODE | JSON_THROW_ON_ERROR),
            ] + $extra,
            static function (array $values) use ($completed): void {
                $paths = json_decode(is_string($values['paths'] ?? null) ? $values['paths'] : '[]', true);
                $completed(array_values(array_filter(is_array($paths) ? $paths : [], is_string(...))));
            },
            static function () use ($completed): void {
                $completed([]);
            },
        );
    }
}
