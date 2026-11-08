<?php

declare(strict_types=1);

namespace Pam\Desktop\Canvas;

use Closure;
use Pam\Desktop\Element;
use Pam\Desktop\EventKind;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\PropKey;
use Pam\Desktop\Renderable;

/** Retained native canvas. Coordinates and pointer positions are logical pixels. */
final class CanvasView implements Renderable
{
    private ?Closure $handler = null;

    private function __construct(private readonly CanvasScene $scene) {}

    public static function make(CanvasScene $scene, int $revision = 1): self
    {
        return new self($scene);
    }

    public function dp(): self { return $this; }

    /** @param Closure(CanvasEventKind, float, float): void $handler */
    public function onPointer(Closure $handler): self
    {
        $copy = clone $this;
        $copy->handler = $handler;
        return $copy;
    }

    public function toElement(): Element
    {
        $view = \Pam\Desktop\UI\Canvas::make()->property(PropKey::CanvasCommands, Internal\DesktopCommands::encode($this->scene));
        if ($this->handler === null) return $view;
        $handler = $this->handler;
        return $view->on(EventKind::Native, static function (string $payload) use ($handler): void {
            $values = Wire::decodeMap($payload);
            $kind = CanvasEventKind::tryFrom((int) ($values['event'] ?? 4)) ?? CanvasEventKind::PointerCancel;
            $handler($kind, (float) ($values['x'] ?? 0), (float) ($values['y'] ?? 0));
        });
    }
}
