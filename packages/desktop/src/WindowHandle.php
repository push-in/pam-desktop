<?php

declare(strict_types=1);

namespace Pam\Desktop;

final readonly class WindowHandle
{
    public function __construct(
        public string $id,
        private Invocation $invocation,
    ) {
        Identifier::assert($id, 'The window handle identifier');
    }

    public function title(string $title): self
    {
        $this->invocation->effect(WindowEffect::title($title, $this->id));

        return $this;
    }

    public function show(): self
    {
        $this->invocation->effect(WindowEffect::visible(true, $this->id));

        return $this;
    }

    public function hide(): self
    {
        $this->invocation->effect(WindowEffect::visible(false, $this->id));

        return $this;
    }

    public function focus(): self
    {
        $this->invocation->effect(WindowEffect::focus($this->id));

        return $this;
    }

    public function close(): self
    {
        $this->invocation->effect(WindowEffect::close($this->id));

        return $this;
    }

    public function fullscreen(bool $enabled = true): self
    {
        $this->invocation->effect(WindowEffect::fullscreen($enabled, $this->id));

        return $this;
    }

    public function maximize(bool $enabled = true): self
    {
        $this->invocation->effect(WindowEffect::maximized($enabled, $this->id));

        return $this;
    }

    public function alwaysOnTop(bool $enabled = true): self
    {
        $this->invocation->effect(WindowEffect::alwaysOnTop($enabled, $this->id));

        return $this;
    }
}
