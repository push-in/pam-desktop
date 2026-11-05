<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\Style;
use Pam\Desktop\UI\Button;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\Text;
use Pam\Desktop\UI\Window;

require __DIR__.'/../../vendor/autoload.php';

/**
 * Exercises secondary windows: an element that is a native window while it
 * is rendered, sharing the state of the component that opens it.
 */
final class Main extends Component
{
    #[State]
    public bool $inspector = false;

    #[State]
    public int $count = 0;

    public function render(): Column
    {
        return Column::make(
            Text::make('Windows')->style(new Style(fontSize: 24, fontWeight: 700, textColor: 0xFF12161C)),
            Text::make("Counter: {$this->count}")->style(new Style(fontSize: 16, textColor: 0xFF2A3140)),
            Button::make($this->inspector ? 'Close the inspector' : 'Open the inspector')->onPress(function (): void {
                $this->inspector = !$this->inspector;
            }),
            $this->inspector
                ? Window::make(
                    Column::make(
                        Text::make('Inspector')->style(new Style(fontSize: 20, fontWeight: 700, textColor: 0xFFFFFFFF)),
                        Text::make("This window shares the component's state: the counter is {$this->count}.")
                            ->style(new Style(fontSize: 14, lineHeight: 21, textColor: 0xFFC9D1DD)),
                        Button::make('Add one')->onPress(function (): void {
                            $this->count++;
                        }),
                    )->style(new Style(flexGrow: 1, padding: 24, gap: 12, backgroundColor: 0xFF171B24)),
                )
                    ->title('Inspector')
                    ->size(360, 260)
                    ->onClose(function (): void {
                        $this->inspector = false;
                    })
                : null,
        )->style(new Style(flexGrow: 1, padding: 28, gap: 14, backgroundColor: 0xFFF7F8FA));
    }
}

App::run(new Main());
