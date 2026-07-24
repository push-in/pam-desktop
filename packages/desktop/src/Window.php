<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final readonly class Window
{
    private function __construct(
        public string $entry,
        public string $title,
        public int $width,
        public int $height,
        public int $minWidth,
        public int $minHeight,
        public bool $resizable,
        public bool $visible,
        public WindowTheme $theme,
    ) {
        if (
            $entry === ''
            || str_starts_with($entry, '/')
            || preg_match('~(^|[\\\\/])\.\.([\\\\/]|$)~', $entry) === 1
        ) {
            throw new InvalidArgumentException(
                'The desktop entry must be a relative path inside the project.',
            );
        }
        if (trim($title) === '') {
            throw new InvalidArgumentException('The window title cannot be empty.');
        }

        if ($minWidth < 320 || $minHeight < 240) {
            throw new InvalidArgumentException('The minimum window size is 320x240.');
        }

        if ($width < $minWidth || $height < $minHeight) {
            throw new InvalidArgumentException(
                'The initial window size cannot be smaller than its minimum size.',
            );
        }
    }

    public static function create(string $title): self
    {
        return new self(
            entry: 'resources/index.html',
            title: $title,
            width: 1120,
            height: 720,
            minWidth: 720,
            minHeight: 520,
            resizable: true,
            visible: true,
            theme: WindowTheme::System,
        );
    }

    public function entry(string $entry): self
    {
        return new self(
            entry: $entry,
            title: $this->title,
            width: $this->width,
            height: $this->height,
            minWidth: $this->minWidth,
            minHeight: $this->minHeight,
            resizable: $this->resizable,
            visible: $this->visible,
            theme: $this->theme,
        );
    }

    public function load(string $path): self
    {
        return $this->entry($path);
    }

    public function size(int $width, int $height): self
    {
        return new self(
            entry: $this->entry,
            title: $this->title,
            width: $width,
            height: $height,
            minWidth: $this->minWidth,
            minHeight: $this->minHeight,
            resizable: $this->resizable,
            visible: $this->visible,
            theme: $this->theme,
        );
    }

    public function minimumSize(int $width, int $height): self
    {
        return new self(
            entry: $this->entry,
            title: $this->title,
            width: $this->width,
            height: $this->height,
            minWidth: $width,
            minHeight: $height,
            resizable: $this->resizable,
            visible: $this->visible,
            theme: $this->theme,
        );
    }

    public function resizable(bool $resizable = true): self
    {
        return new self(
            entry: $this->entry,
            title: $this->title,
            width: $this->width,
            height: $this->height,
            minWidth: $this->minWidth,
            minHeight: $this->minHeight,
            resizable: $resizable,
            visible: $this->visible,
            theme: $this->theme,
        );
    }

    public function visible(bool $visible = true): self
    {
        return new self(
            entry: $this->entry,
            title: $this->title,
            width: $this->width,
            height: $this->height,
            minWidth: $this->minWidth,
            minHeight: $this->minHeight,
            resizable: $this->resizable,
            visible: $visible,
            theme: $this->theme,
        );
    }

    public function theme(WindowTheme $theme): self
    {
        return new self(
            entry: $this->entry,
            title: $this->title,
            width: $this->width,
            height: $this->height,
            minWidth: $this->minWidth,
            minHeight: $this->minHeight,
            resizable: $this->resizable,
            visible: $this->visible,
            theme: $theme,
        );
    }

    /**
     * @return array{
     *     id: string,
     *     entry: string,
     *     title: string,
     *     width: int,
     *     height: int,
     *     minWidth: int,
     *     minHeight: int,
     *     resizable: bool,
     *     visible: bool,
     *     theme: int
     * }
     */
    public function toArray(string $id = 'main'): array
    {
        Identifier::assert($id, 'The window identifier');

        return [
            'id' => $id,
            'entry' => $this->entry,
            'title' => $this->title,
            'width' => $this->width,
            'height' => $this->height,
            'minWidth' => $this->minWidth,
            'minHeight' => $this->minHeight,
            'resizable' => $this->resizable,
            'visible' => $this->visible,
            'theme' => $this->theme->value,
        ];
    }
}
