<?php

declare(strict_types=1);

namespace Pam\Desktop\UI;

use Closure;
use Pam\Desktop\{Element,NodeKind,PropKey,Renderable,EventKind,DragPayload};

/** Accepts only matching typed sources from the same native window. */
final class DropTarget extends Element
{
    public static function make(Renderable|false|null ...$children): self
    {
        return (new self(NodeKind::Column))->withChildren($children)->withProperty(PropKey::DropEnabled, true);
    }
    public function accepts(int|\BackedEnum $type): self
    {
        $type = $type instanceof \BackedEnum ? $type->value : $type;
        if (!is_int($type) || $type < 1 || $type > 65535) {
            throw new \InvalidArgumentException('Drop type must be a positive integer.');
        }return $this->withProperty(PropKey::DropType, $type);
    }
    /** @param Closure(DragPayload):mixed $handler */ public function onDrop(Closure $handler): self
    {
        return $this->typed(EventKind::Drop, $handler);
    }
    /** @param Closure(DragPayload):mixed $handler */ public function onDragEnter(Closure $handler): self
    {
        return $this->typed(EventKind::DragEnter, $handler);
    }
    /** @param Closure(DragPayload):mixed $handler */ public function onDragLeave(Closure $handler): self
    {
        return $this->typed(EventKind::DragLeave, $handler);
    }
    /** @param Closure(DragPayload):mixed $handler */ private function typed(EventKind $event, Closure $handler): self
    {
        return $this->withEvent($event, static fn (string $payload): mixed => $handler(DragPayload::fromPayload($payload)));
    }
}
