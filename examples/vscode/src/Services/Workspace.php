<?php

declare(strict_types=1);

namespace App\Services;

use App\Domain\FileKind;
use Closure;
use Pam\Desktop\MenuItem;
use Pam\DesktopUi\TreeNode;

/**
 * The folder the editor opened (this example, on disk): the explorer tree,
 * read one folder at a time as it is expanded, the open tabs and their
 * highlighted lines.
 */
final class Workspace
{
    /** Never listed: generated or downloaded, as `files.exclude` hides them. */
    private const array HIDDEN = ['.git', '.pam-desktop', 'dist'];

    private const array EXPANDED = ['src', 'src/Components', 'src/Services'];

    /** @var list<string> */
    public array $tabs = [];

    public ?string $active = null;

    /** @var array<string, list<list<array{0: string, 1: string}>>> highlighted lines by path */
    private array $documents = [];

    public function __construct(public readonly string $root)
    {
    }

    /** @var list<TreeNode>|null the explorer's top level, built once */
    private ?array $tree = null;

    /** @var (Closure(string, string): list<MenuItem>)|null the row menu, given the path and its kind */
    public ?Closure $menu = null;

    /**
     * The explorer as tree nodes: a folder is read the first time it opens,
     * folders first, each level sorted.
     *
     * @return list<TreeNode>
     */
    public function tree(string $folder = ''): array
    {
        if ($folder === '' && $this->tree !== null) {
            return $this->tree;
        }
        $nodes = [];
        foreach ($this->children($folder) as [$name, $isFolder]) {
            $path = ltrim($folder.'/'.$name, '/');
            $nodes[] = new TreeNode(
                $path,
                $name,
                $isFolder ? fn (): array => $this->tree($path) : null,
                icon: $isFolder ? null : FileKind::icon($name),
                iconColor: $isFolder ? null : FileKind::colour($name),
                contextMenu: $this->menu === null ? [] : ($this->menu)($path, $isFolder ? 'folder' : 'file'),
            );
        }

        return $folder === '' ? $this->tree = $nodes : $nodes;
    }

    /** @return list<string> the folders open when the explorer first shows */
    public function expanded(): array
    {
        return self::EXPANDED;
    }

    public function name(): string
    {
        return strtoupper(basename($this->root));
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
