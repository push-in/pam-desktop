<?php

declare(strict_types=1);

namespace Pam\Desktop\UI;

use Pam\Desktop\Element;
use Pam\Desktop\KeyboardAvoidingBehavior;
use Pam\Desktop\NodeKind;
use Pam\Desktop\PropKey;
use Pam\Desktop\Renderable;

final class KeyboardAvoidingView extends Element
{
    public static function make(
        Renderable $content,
        KeyboardAvoidingBehavior $behavior = KeyboardAvoidingBehavior::Resize,
    ): self {
        return (new self(NodeKind::KeyboardAvoidingView))
            ->withChildren([$content])
            ->withProperty(PropKey::KeyboardBehavior, $behavior->value);
    }

    public function verticalOffset(float $offset): self
    {
        return $this->withProperty(PropKey::KeyboardVerticalOffset, $offset);
    }

    public function avoidingEnabled(bool $enabled = true): self
    {
        return $this->withProperty(PropKey::KeyboardAvoidingEnabled, $enabled);
    }
}
