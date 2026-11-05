<?php

declare(strict_types=1);

namespace Pam\Desktop\UI;

use Closure;
use InvalidArgumentException;
use Pam\Desktop\Element;
use Pam\Desktop\EventKind;
use Pam\Desktop\NodeKind;
use Pam\Desktop\PropKey;
use Pam\Desktop\Renderable;

/**
 * A secondary window: its children are the content of a native window of
 * their own, laid out against that window's size. Render it like any other
 * element; the window opens while the element is in the tree (and visible)
 * and closes when it leaves.
 *
 * ```php
 * $this->settingsOpen
 *     ? Window::make(new SettingsScreen())
 *         ->title('Settings')
 *         ->size(520, 420)
 *         ->onClose(fn () => $this->settingsOpen = false)
 *     : null,
 * ```
 *
 * The main window is controlled with the `Pam\Desktop\Window` facade.
 */
final class Window extends Element
{
    public static function make(Renderable|false|null ...$children): self
    {
        return (new self(NodeKind::Modal))
            ->withChildren($children)
            ->withProperty(PropKey::WindowSurface, true)
            ->withProperty(PropKey::Visible, true);
    }

    public function title(string $title): self
    {
        if (strlen($title) > 512) {
            throw new InvalidArgumentException('Window titles cannot exceed 512 bytes.');
        }

        return $this->withProperty(PropKey::WindowTitle, $title);
    }

    /** The size of the window's content area when it opens. */
    public function size(float $width, float $height): self
    {
        if ($width < 1 || $height < 1) {
            throw new InvalidArgumentException('A window needs a positive size.');
        }

        return $this
            ->withProperty(PropKey::WindowWidth, $width)
            ->withProperty(PropKey::WindowHeight, $height);
    }

    public function minimumSize(float $width, float $height): self
    {
        if ($width < 1 || $height < 1) {
            throw new InvalidArgumentException('A window needs a positive minimum size.');
        }

        return $this
            ->withProperty(PropKey::WindowMinWidth, $width)
            ->withProperty(PropKey::WindowMinHeight, $height);
    }

    public function resizable(bool $resizable = true): self
    {
        return $this->withProperty(PropKey::WindowResizable, $resizable);
    }

    /** Blocks the main window while this one is open, like a dialog. */
    public function modal(bool $modal = true): self
    {
        return $this->withProperty(PropKey::WindowModal, $modal);
    }

    /** Hides the window without unmounting its content. */
    public function open(bool $open = true): self
    {
        return $this->withProperty(PropKey::Visible, $open);
    }

    /**
     * The user asked to close the window. The window stays open until the
     * application stops rendering it (or renders it with `open(false)`).
     */
    public function onClose(Closure $handler): self
    {
        return $this->withEvent(EventKind::ModalRequestClose, static fn (mixed $payload = ''): mixed => $handler());
    }
}
