<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts;

use InvalidArgumentException;
use Pam\Desktop\Canvas\Canvas;
use Pam\Desktop\Canvas\FontWeight;
use Pam\Desktop\Canvas\LineCap;
use Pam\Desktop\Canvas\TextAlign;
use Pam\Desktop\Charts\Internal\Color;
use Pam\Desktop\Charts\Internal\Text;

/**
 * Circular progress indicator with a track, a round-capped arc and a center label.
 *
 * ```php
 * ProgressRing::make(size: 56)->progress(0.42)->thickness(6)->label('42%');
 * ```
 */
final class ProgressRing extends Chart
{
    private float $size;
    private float $progress = 0.0;
    private float $thickness = 6.0;
    private ?string $color = null;
    private ?string $track = null;
    private ?string $label = null;

    protected function __construct(float $size)
    {
        parent::__construct($size, $size);
        $this->size = $size;
    }

    public static function make(float|int $size): self
    {
        return new self((float) $size);
    }

    /** Completed share, 0..1 (clamped). */
    public function progress(float|int $progress): self
    {
        $progress = (float) $progress;

        if (!is_finite($progress)) {
            throw new InvalidArgumentException('Progress must be finite.');
        }

        $copy = clone $this;
        $copy->progress = max(0.0, min(1.0, $progress));

        return $copy;
    }

    public function thickness(float|int $thickness): self
    {
        if ($thickness <= 0 || $thickness > $this->size / 2) {
            throw new InvalidArgumentException('Ring thickness must be positive and fit inside the ring.');
        }

        $copy = clone $this;
        $copy->thickness = (float) $thickness;

        return $copy;
    }

    /** Arc color; defaults to the theme primary. */
    public function color(string $color): self
    {
        $copy = clone $this;
        $copy->color = Color::normalize($color);

        return $copy;
    }

    /** Track color; defaults to the theme grid. */
    public function track(string $color): self
    {
        $copy = clone $this;
        $copy->track = Color::normalize($color);

        return $copy;
    }

    /** Text drawn in the center. */
    public function label(?string $label): self
    {
        $copy = clone $this;
        $copy->label = $label === null ? null : Series::validLabel($label);

        return $copy;
    }

    protected function draw(Canvas $canvas): Canvas
    {
        $canvas = $this->paintSurface($canvas);
        $center = $this->size / 2;
        $radius = $center - $this->thickness / 2;
        $canvas = $canvas->arc($center, $center, $radius, 0, 360, $this->track ?? $this->theme->grid, $this->thickness, LineCap::Butt);

        if ($this->progress > 0.0) {
            $canvas = $canvas->arc(
                $center,
                $center,
                $radius,
                0,
                round($this->progress * 360, 3),
                $this->color ?? $this->theme->primary,
                $this->thickness,
                LineCap::Round,
            );
        }

        if ($this->label !== null) {
            $textSize = max(8.0, min(($this->size - 2 * $this->thickness) * 0.32, $this->size * 0.26));
            $canvas = $canvas->label(
                $this->label,
                $center,
                $center + Text::baselineOffset($textSize),
                $textSize,
                $this->theme->ink,
                TextAlign::Center,
                FontWeight::SemiBold,
            );
        }

        return $canvas;
    }
}
