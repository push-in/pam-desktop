<?php

declare(strict_types=1);

namespace Pam\Desktop\Charts;

use Closure;
use InvalidArgumentException;
use Pam\Desktop\Canvas\Canvas;
use Pam\Desktop\Canvas\CanvasScene;
use Pam\Desktop\Canvas\CanvasView;
use Pam\Desktop\Element;
use Pam\Desktop\PropKey;
use Pam\Desktop\Renderable;

/**
 * Base of every chart: an immutable builder that paints a dp canvas scene.
 *
 * Charts are `Renderable`, so a chart can be returned straight from a screen or
 * component. `view()` exposes the underlying `CanvasView` and `scene()` the raw
 * display list for tests and composition.
 */
abstract class Chart implements Renderable
{
    public const float MIN_SIZE = 8.0;
    public const float MAX_SIZE = 4096.0;

    protected ChartTheme $theme;

    protected function __construct(
        protected float $width,
        protected float $height,
    ) {
        if ($width < self::MIN_SIZE || $width > self::MAX_SIZE || $height < self::MIN_SIZE || $height > self::MAX_SIZE) {
            throw new InvalidArgumentException('Chart sizes must be between 8 and 4096 dp.');
        }

        $this->theme = ChartTheme::dark();
    }

    /** Colors used for ink, grid, surface and series. Defaults to `ChartTheme::dark()`. */
    public function theme(ChartTheme $theme): static
    {
        $copy = clone $this;
        $copy->theme = $theme;

        return $copy;
    }

    public function width(): float
    {
        return $this->width;
    }

    public function height(): float
    {
        return $this->height;
    }

    /** Display list of the chart, in dp. */
    public function scene(): CanvasScene
    {
        return $this->draw(new Canvas())->scene();
    }

    /**
     * Revision derived from the painted scene: it changes whenever any input
     * changes the drawing, which tells the native view to redraw.
     */
    public function revision(): int
    {
        return self::revisionOf($this->scene());
    }

    /** The native canvas view in dp units, with the pointer handler wired when one is set. */
    public function view(): CanvasView
    {
        $scene = $this->scene();
        $view = CanvasView::make($scene, self::revisionOf($scene))->dp();
        $handler = $this->pointerHandler();

        return $handler === null ? $view : $view->onPointer($handler);
    }

    public function toElement(): Element
    {
        return $this->view()
            ->toElement()
            ->property(PropKey::Width, $this->width)
            ->property(PropKey::Height, $this->height);
    }

    /** Paints the chart. */
    abstract protected function draw(Canvas $canvas): Canvas;

    /**
     * Pointer handler for the canvas view, or null when the chart is static.
     *
     * @return (Closure(\Pam\Desktop\Canvas\CanvasEventKind, float, float): void)|null
     */
    protected function pointerHandler(): ?Closure
    {
        return null;
    }

    /** Fills the background with the theme surface when it is not transparent. */
    protected function paintSurface(Canvas $canvas): Canvas
    {
        return Internal\Color::opacity($this->theme->surface) === 0.0
            ? $canvas
            : $canvas->fillRect(0, 0, $this->width, $this->height, $this->theme->surface);
    }

    private static function revisionOf(CanvasScene $scene): int
    {
        return max(1, crc32($scene->toJson()) & 0x7fffffff);
    }
}
