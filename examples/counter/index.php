<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Component;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Style;
use Pam\Desktop\UI\Button;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\Text;

require __DIR__.'/../../vendor/autoload.php';

final class Counter extends Component
{
    #[State]
    public int $count = 0;

    public function render(): Column
    {
        return Column::make(
            Text::make('PAM Desktop')->style(new Style(fontSize: 28, fontWeight: 700)),
            Text::make("Count: {$this->count}")->style(new Style(fontSize: 18)),
            Button::make('Increment')->onPress(function (): void {
                $this->count++;
            }),
        )->style(new Style(padding: 24, gap: 16));
    }
}

App::run(new Counter());
