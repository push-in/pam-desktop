<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\Style;
use Pam\Desktop\UI\Button;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\Text;

require __DIR__.'/../../vendor/autoload.php';

/**
 * Exercises the error overlay: an exception in an event handler (a toast in
 * development) and one while rendering (the full report).
 */
final class Faulty extends Component
{
    #[State]
    public bool $broken = false;

    private function total(array $prices): float
    {
        return array_sum($prices) / count($prices);
    }

    public function render(): Column
    {
        if ($this->broken) {
            throw new RuntimeException('The invoice has no lines to render.');
        }

        return Column::make(
            Text::make('Error overlay')->style(new Style(fontSize: 24, fontWeight: 700, textColor: 0xFF12161C)),
            Text::make('Errors never leave a blank window: in development the host shows what failed and where.')
                ->style(new Style(fontSize: 14, lineHeight: 21, textColor: 0xFF5B6472)),
            Button::make('Fail in an event handler')->onPress(function (): void {
                $this->total([]);
            }),
            Button::make('Fail while rendering')->onPress(function (): void {
                $this->broken = true;
            }),
        )->style(new Style(flexGrow: 1, padding: 28, gap: 14, backgroundColor: 0xFFF7F8FA));
    }
}

App::run(new Faulty());
