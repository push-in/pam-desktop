<?php

declare(strict_types=1);

namespace Pam\Desktop\UI;

use Closure;
use Pam\Desktop\{Element,NodeKind,PropKey,Renderable,EventKind,DragPayload};

/** Typed in-app native drag. The system pasteboard carries an opaque nonce, never the data. */
final class DragSource extends Element
{
    public static function make(Renderable|false|null ...$children): self
    {
        return (new self(NodeKind::Column))->withChildren($children)->withProperty(PropKey::Draggable, true);
    }
    public function payload(int|\BackedEnum $type, string $data): self
    {
        $type = $type instanceof \BackedEnum ? $type->value : $type;
        if (!is_int($type) || $type < 1 || $type > 65535 || strlen($data) > 512) {
            throw new \InvalidArgumentException('Drag type must be a positive integer; payload is limited to 512 bytes.');
        }return $this->withProperty(PropKey::DragType, $type)->withProperty(PropKey::DragData, $data);
    }
    /** @param Closure(DragPayload):mixed $handler */
    public function onDragStart(Closure $handler): self
    {
        return $this->withEvent(EventKind::DragStart, static fn (string $payload): mixed => $handler(DragPayload::fromPayload($payload)));
    }
    public function onDragEnd(Closure $handler): self
    {
        return $this->withEvent(EventKind::DragEnd, $handler);
    }
}
