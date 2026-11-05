<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\DrawingMode;
use Pam\Desktop\Style;
use Pam\Desktop\UI\Button;
use Pam\Desktop\UI\Canvas;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\DrawingCanvas;
use Pam\Desktop\UI\Row;
use Pam\Desktop\UI\Text;

require __DIR__.'/../../vendor/autoload.php';

/**
 * Exercises the vector Canvas (a bar chart drawn from state) and the
 * freehand DrawingCanvas (strokes over a photo, with undo and clear).
 */
final class Surfaces extends Component
{
    /** @var list<int> */
    #[State]
    public array $bars = [42, 68, 30, 86, 55, 74, 38];

    #[State]
    public string $drawing = '';

    #[State]
    public int $undo = 0;

    #[State]
    public int $clear = 0;

    #[State]
    public bool $eraser = false;

    #[State]
    public int $color = 0xFFFF3B6B;

    public function render(): Column
    {
        $chart = Canvas::make()->roundedRectangle(0, 0, 380, 200, 16, 0xFF12161C);
        foreach ([50, 100, 150] as $y) {
            $chart = $chart->line(20, $y, 360, $y, 1, 0xFF2A3140);
        }
        foreach ($this->bars as $index => $value) {
            $height = $value * 1.6;
            $x = 32 + $index * 48;
            $chart = $chart
                ->roundedRectangle($x, 180 - $height, 28, $height, 8, $index % 2 === 0 ? 0xFF2F6BFF : 0xFF12B76A)
                ->circle($x + 14, 180 - $height, 5, 0xFFFFFFFF);
        }
        $chart = $chart->line(20, 180, 360, 180, 2, 0xFF8A93A3);
        $strokes = count(json_decode($this->drawing === '' ? '{"strokes":[]}' : $this->drawing, true)['strokes'] ?? []);
        $label = new Style(fontSize: 12, textColor: 0xFF6B7485);

        return Column::make(
            Text::make('Canvas and DrawingCanvas')->style(new Style(fontSize: 24, fontWeight: 700, textColor: 0xFF12161C)),
            Row::make(
                Column::make(
                    $chart->style(new Style(width: 380, height: 200)),
                    Text::make('Canvas: rectangles, circles and lines from state')->style($label),
                    Button::make('Shuffle')->onPress(function (): void {
                        $this->bars = array_map(static fn (): int => random_int(15, 100), $this->bars);
                    }),
                )->style(new Style(gap: 10)),
                Column::make(
                    DrawingCanvas::make('asset://photo.jpg', $this->drawing)
                        ->brush($this->color, 6, $this->eraser ? DrawingMode::Eraser : DrawingMode::Brush)
                        ->undoRequest($this->undo)
                        ->clearRequest($this->clear)
                        ->onChange(function (string $drawing): void {
                            $this->drawing = $drawing;
                        })
                        ->cursor('crosshair')
                        ->style(new Style(width: 400, height: 300, borderRadius: 16, backgroundColor: 0xFF12161C)),
                    Text::make("DrawingCanvas: {$strokes} stroke(s) · ".($this->eraser ? 'eraser' : 'brush'))->style($label),
                    Row::make(
                        Button::make('Red')->onPress(function (): void { $this->color = 0xFFFF3B6B; $this->eraser = false; }),
                        Button::make('Yellow')->onPress(function (): void { $this->color = 0xFFFFD21E; $this->eraser = false; }),
                        Button::make('Eraser')->onPress(function (): void { $this->eraser = true; }),
                        Button::make('Undo')->onPress(function (): void { $this->undo++; }),
                        Button::make('Clear')->onPress(function (): void { $this->clear++; }),
                    )->style(new Style(gap: 8)),
                )->style(new Style(gap: 10)),
            )->style(new Style(gap: 24)),
        )->style(new Style(flexGrow: 1, padding: 28, gap: 18, backgroundColor: 0xFFF7F8FA));
    }
}

App::run(new Surfaces());
