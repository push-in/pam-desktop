<?php

declare(strict_types=1);

namespace App\Services;

use App\Domain\Entry;
use App\Domain\FileKind;

/**
 * The folder the editor opened (this example, on disk): the explorer tree,
 * read one folder at a time as it is expanded, the open tabs and their
 * highlighted lines.
 */
final class Workspace
{
    /** Never listed: generated or downloaded, as `files.exclude` hides them. */
    private const array HIDDEN = ['.git', '.pam-desktop', 'dist'];

    /** @var array<string, true> */
    private array $expanded = [];

    /** @var list<string> */
    public array $tabs = [];

    public ?string $active = null;

    /** @var array<string, list<list<array{0: string, 1: string}>>> highlighted lines by path */
    private array $documents = [];

    public function __construct(public readonly string $root)
    {
        $this->expanded = ['src' => true, 'src/Components' => true, 'src/Services' => true];
    }

    public function name(): string
    {
        return strtoupper(basename($this->root));
    }

    /** @return list<Entry> the rows of the explorer, folders first, each level sorted */
    public function entries(string $folder = '', int $depth = 0): array
    {
        $rows = [];
        foreach ($this->children($folder) as [$name, $isFolder]) {
            $path = ltrim($folder.'/'.$name, '/');
            $open = $isFolder && isset($this->expanded[$path]);
            $rows[] = new Entry($path, $name, $isFolder, $depth, $open);
            if ($open) {
                array_push($rows, ...$this->entries($path, $depth + 1));
            }
        }

        return $rows;
    }

    public function toggle(string $path): void
    {
        if (isset($this->expanded[$path])) {
            unset($this->expanded[$path]);
        } else {
            $this->expanded[$path] = true;
        }
    }

    public function collapseAll(): void
    {
        $this->expanded = [];
    }

    public function open(string $path): void
    {
        if (!FileKind::readable(basename($path)) || !is_file($this->root.'/'.$path)) {
            return;
        }
        if (!in_array($path, $this->tabs, true)) {
            $this->tabs[] = $path;
        }
        $this->active = $path;
    }

    public function close(string $path): void
    {
        $index = array_search($path, $this->tabs, true);
        if ($index === false) {
            return;
        }
        array_splice($this->tabs, $index, 1);
        unset($this->documents[$path]);
        if ($this->active === $path) {
            $this->active = $this->tabs[min($index, count($this->tabs) - 1)] ?? null;
        }
    }

    /** @return list<list<array{0: string, 1: string}>> */
    public function lines(string $path): array
    {
        return $this->documents[$path] ??= Highlighter::lines($path, (string) file_get_contents($this->root.'/'.$path));
    }

    /** @return list<string> every readable file, for Quick Open */
    public function files(string $folder = ''): array
    {
        $files = [];
        foreach ($this->children($folder) as [$name, $isFolder]) {
            $path = ltrim($folder.'/'.$name, '/');
            if ($isFolder) {
                if ($name !== 'vendor' && $name !== 'assets') {
                    array_push($files, ...$this->files($path));
                }
            } elseif (FileKind::readable($name)) {
                $files[] = $path;
            }
        }

        return $files;
    }

    /** @return list<array{0: string, 1: bool}> name and whether it is a folder */
    private function children(string $folder): array
    {
        $directory = rtrim($this->root.'/'.$folder, '/');
        $names = @scandir($directory) ?: [];
        $folders = [];
        $files = [];
        foreach ($names as $name) {
            if ($name === '.' || $name === '..' || in_array($name, self::HIDDEN, true)) {
                continue;
            }
            if (is_dir($directory.'/'.$name)) {
                $folders[] = [$name, true];
            } else {
                $files[] = [$name, false];
            }
        }
        $byName = static fn (array $a, array $b): int => strnatcasecmp($a[0], $b[0]);
        usort($folders, $byName);
        usort($files, $byName);

        return [...$folders, ...$files];
    }
}
