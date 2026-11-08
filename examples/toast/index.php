<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Component;
use Pam\Desktop\Style;
use Pam\Desktop\System\Toast;
use Pam\Desktop\System\ToastPosition;
use Pam\Desktop\UI\Button;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\Text;

require __DIR__.'/../../vendor/autoload.php';

/** Exercises toasts: the plain capsule and the success, error and info cards. */
final class Toasts extends Component
{
    public function render(): Column
    {
        return Column::make(
            Text::make('Toasts')->style(new Style(fontSize: 24, fontWeight: 700, textColor: 0xFF12161C)),
            Button::make('Plain')->onPress(fn () => Toast::show('Copied to the clipboard')),
            Button::make('Success (top)')->onPress(fn () => Toast::success('Saved', 'Your changes are safe.')),
            Button::make('Error (bottom)')->onPress(fn () => Toast::error('No connection', 'Trying again in 5 seconds.', ToastPosition::Bottom)),
            Button::make('Info, sized to its content')->onPress(fn () => Toast::message('3 new messages', fitContent: true)),
        )->style(new Style(flexGrow: 1, padding: 28, gap: 12, backgroundColor: 0xFFF7F8FA));
    }
}

App::run(new Toasts());
