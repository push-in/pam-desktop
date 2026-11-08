<?php

declare(strict_types=1);

namespace Pam\Desktop\UI;

use Closure;
use InvalidArgumentException;
use JsonException;
use Pam\Desktop\AnimationEasing;
use Pam\Desktop\AnimationFillMode;
use Pam\Desktop\AnimationKeyframe;
use Pam\Desktop\AnimationPlayState;
use Pam\Desktop\Element;
use Pam\Desktop\EventKind;
use Pam\Desktop\Internal\BinaryValue;
use Pam\Desktop\NodeKind;
use Pam\Desktop\PropKey;
use Pam\Desktop\Renderable;
use Pam\Desktop\Worklets\Worklet;
use Pam\Desktop\Worklets\WorkletTarget;

use function count;

final class Animated extends Element
{
    public static function worklet(
        Renderable $content,
        Worklet $worklet,
        WorkletTarget $target,
        int $durationMs = 300,
    ): self {
        return (new self(NodeKind::View))
            ->withChildren([$content])
            ->withProperty(PropKey::WorkletProgram, new BinaryValue($worklet->bytecode()))
            ->withProperty(PropKey::WorkletTarget, $target->value)
            ->withProperty(PropKey::WorkletDurationMs, max(1, min(60_000, $durationMs)))
            ->withProperty(PropKey::WorkletIterations, 1);
    }

    /** @param list<AnimationKeyframe> $keyframes */
    /**
     * Wraps [content] in a view that plays an Animation program on the UI
     * thread (springs, per-property timing, sequences, repeats).
     */
    public static function program(Renderable $content, \Pam\Desktop\Animation\Animation $animation): self
    {
        return (new self(NodeKind::View))
            ->withChildren([$content])
            ->withProperty(PropKey::AnimationProgram, $animation->encode());
    }

    public static function make(
        Renderable $content,
        array $keyframes,
        int $durationMs = 300,
        AnimationEasing $easing = AnimationEasing::EaseInOut,
    ): self {
        if (
            count($keyframes) < 2
            || count($keyframes) > 64
            || array_filter($keyframes, static fn ($frame): bool => !$frame instanceof AnimationKeyframe)
        ) {
            throw new InvalidArgumentException('Animated requires between two and sixty-four keyframes.');
        }
        $offsets = array_map(static fn (AnimationKeyframe $frame): float => $frame->offset, $keyframes);
        if ($offsets !== array_values(array_unique($offsets, SORT_REGULAR))) {
            throw new InvalidArgumentException('Animation keyframe offsets must be unique.');
        }
        $sorted = $offsets;
        sort($sorted, SORT_NUMERIC);
        if ($sorted !== $offsets) {
            throw new InvalidArgumentException('Animation keyframes must be ordered by offset.');
        }

        try {
            $payload = json_encode($keyframes, JSON_THROW_ON_ERROR | JSON_PRESERVE_ZERO_FRACTION);
        } catch (JsonException $error) {
            throw new InvalidArgumentException('Animation keyframes cannot be encoded.', 0, $error);
        }

        return (new self(NodeKind::View))
            ->withChildren([$content])
            ->withProperty(PropKey::AnimationKeyframes, new BinaryValue($payload))
            ->withProperty(PropKey::AnimationDurationMs, max(1, min(60_000, $durationMs)))
            ->withProperty(PropKey::AnimationEasing, $easing->value)
            ->withProperty(PropKey::AnimationIterations, 1)
            ->withProperty(PropKey::AnimationDelayMs, 0)
            ->withProperty(PropKey::AnimationFillMode, AnimationFillMode::Forwards->value)
            ->withProperty(PropKey::AnimationPlayState, AnimationPlayState::Running->value)
            ->withProperty(PropKey::AnimationAutoReverse, false);
    }

    public function iterations(int $iterations): self
    {
        $bounded = max(0, min(10_000, $iterations));
        return $this
            ->withProperty(PropKey::AnimationIterations, $bounded)
            ->withProperty(PropKey::WorkletIterations, $bounded);
    }

    public function delay(int $milliseconds): self
    {
        return $this->withProperty(PropKey::AnimationDelayMs, max(0, min(60_000, $milliseconds)));
    }

    public function fillMode(AnimationFillMode $mode): self
    {
        return $this->withProperty(PropKey::AnimationFillMode, $mode->value);
    }

    public function playState(AnimationPlayState $state): self
    {
        return $this->withProperty(PropKey::AnimationPlayState, $state->value);
    }

    public function autoReverse(bool $enabled = true): self
    {
        return $this->withProperty(PropKey::AnimationAutoReverse, $enabled);
    }

    public function onComplete(Closure $handler): self
    {
        return $this->withEvent(EventKind::AnimationComplete, $handler);
    }
}
