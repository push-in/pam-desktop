<?php

declare(strict_types=1);

namespace Pam\Desktop\UI;

use Closure;
use JsonException;
use Pam\Desktop\Element;
use Pam\Desktop\EventKind;
use Pam\Desktop\Internal\BinaryValue;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\NativeMenuItem;
use Pam\Desktop\NodeKind;
use Pam\Desktop\PropKey;
use Pam\Desktop\Renderable;

final class InteractionRegion extends Element
{
    public static function make(Renderable|false|null ...$children): self
    {
        return (new self(NodeKind::Pressable))
            ->withChildren($children)
            ->withProperty(PropKey::Draggable, false)
            ->withProperty(PropKey::DropEnabled, false);
    }

    public function draggable(string $data, bool $enabled = true): self
    {
        return $this
            ->withProperty(PropKey::Draggable, $enabled)
            ->withProperty(PropKey::DragData, $data);
    }

    public function acceptsDrop(bool $enabled = true): self
    {
        return $this->withProperty(PropKey::DropEnabled, $enabled);
    }

    /** @param list<NativeMenuItem> $items */
    public function contextMenu(array $items): self
    {
        try {
            $encoded = json_encode(
                array_map(
                    static fn (NativeMenuItem $item): array => [
                        'id' => $item->id,
                        'title' => $item->title,
                        'destructive' => $item->destructive,
                        'disabled' => $item->disabled,
                    ],
                    $items,
                ),
                JSON_THROW_ON_ERROR | JSON_UNESCAPED_UNICODE,
            );
        } catch (JsonException $error) {
            throw new \InvalidArgumentException('Native menu items cannot be encoded.', 0, $error);
        }

        return $this->withProperty(
            PropKey::ContextMenuItems,
            new BinaryValue($encoded),
        );
    }

    public function onDragStart(Closure $handler): self
    {
        return $this->withEvent(EventKind::DragStart, $handler);
    }

    public function onDragEnd(Closure $handler): self
    {
        return $this->withEvent(EventKind::DragEnd, $handler);
    }

    /** @param Closure(string): void $handler */
    public function onDrop(Closure $handler): self
    {
        return $this->withEvent(
            EventKind::Drop,
            static fn (string $payload): mixed => $handler(
                (string) (Wire::decodeMap($payload)['data'] ?? ''),
            ),
        );
    }

    /** @param Closure(string): void $handler */
    public function onMenuAction(Closure $handler): self
    {
        return $this->withEvent(
            EventKind::MenuAction,
            static fn (string $payload): mixed => $handler(
                (string) (Wire::decodeMap($payload)['id'] ?? ''),
            ),
        );
    }
}
