<?php

declare(strict_types=1);

namespace Pam\Desktop;

abstract readonly class DesktopWindow
{
    final public function __construct(private WindowHandle $window)
    {
    }

    public function id(): string
    {
        return $this->window->id;
    }

    public function title(string $title): static
    {
        $this->window->title($title);

        return $this;
    }

    public function show(): static
    {
        $this->window->show();

        return $this;
    }

    public function hide(): static
    {
        $this->window->hide();

        return $this;
    }

    public function focus(): static
    {
        $this->window->focus();

        return $this;
    }

    public function close(): static
    {
        $this->window->close();

        return $this;
    }

    public function fullscreen(bool $enabled = true): static
    {
        $this->window->fullscreen($enabled);

        return $this;
    }

    public function maximize(bool $enabled = true): static
    {
        $this->window->maximize($enabled);

        return $this;
    }
}
