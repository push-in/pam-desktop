<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final readonly class Window
{
    private function __construct(
        public string $title,
        public int $width,
        public int $height,
        public int $minWidth,
        public int $minHeight,
        public bool $resizable,
        public bool $visible,
        public WindowTheme $theme,
    ) {
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

    public function size(int $width, int $height): self
    {
        return new self(
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
    public function toArray(): array
    {
        return [
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

