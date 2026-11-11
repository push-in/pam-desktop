<?php

declare(strict_types=1);

namespace App\Domain;

/** The icon, colour and language a file shows by its name, as VS Code's Seti theme. */
final class FileKind
{
    /** @var array<string, array{0: string, 1: string, 2: string}> extension => icon, colour, language */
    private const array KINDS = [
        'pam.php' => ['file-code', '#E37933', 'PAM Component'],
        'php' => ['file-code', '#A074C4', 'PHP'],
        'css' => ['hash', '#519ABA', 'CSS'],
        'json' => ['braces', '#CBCB41', 'JSON'],
        'lock' => ['lock', '#6D8086', 'JSON'],
        'md' => ['info', '#519ABA', 'Markdown'],
        'ttf' => ['type', '#E37933', 'Font'],
        'txt' => ['file-text', '#6D8086', 'Plain Text'],
        'png' => ['image', '#A074C4', 'Image'],
        'jpg' => ['image', '#A074C4', 'Image'],
        'gitignore' => ['git-branch', '#41535B', 'Ignore'],
    ];

    public static function extension(string $name): string
    {
        if (str_ends_with($name, '.pam.php')) {
            return 'pam.php';
        }
        $dot = strrpos($name, '.');

        return $dot === false ? '' : strtolower(substr($name, $dot + 1));
    }

    public static function icon(string $name): string
    {
        return self::KINDS[self::extension($name)][0] ?? 'file';
    }

    public static function colour(string $name): string
    {
        return self::KINDS[self::extension($name)][1] ?? '#C5C5C5';
    }

    public static function language(string $name): string
    {
        return self::KINDS[self::extension($name)][2] ?? 'Plain Text';
    }

    public static function readable(string $name): bool
    {
        return in_array(self::extension($name), ['pam.php', 'php', 'css', 'json', 'lock', 'md', 'txt', 'gitignore', ''], true);
    }
}
