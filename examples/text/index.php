<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\Style;
use Pam\Desktop\System\Clipboard;
use Pam\Desktop\UI\Button;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\Text;

require __DIR__.'/../../vendor/autoload.php';

/**
 * Exercises text selection: drag to select, double click for a word, triple
 * click for the paragraph, Ctrl+C to copy.
 */
final class Paragraphs extends Component
{
    #[State]
    public string $clipboard = '(press the button after copying with Ctrl+C)';

    public function render(): Column
    {
        return Column::make(
            Text::make('Selectable text')->style(new Style(fontSize: 24, fontWeight: 700, textColor: 0xFF12161C)),
            Text::make('PAM Desktop draws this paragraph with the text engine of the operating system. Drag across it to select, double click a word, triple click for everything, then copy with Ctrl+C.')
                ->selectable()
                ->style(new Style(fontSize: 16, lineHeight: 25, textColor: 0xFF2A3140)),
            Text::make('This second paragraph is not selectable: pressing it does nothing.')
                ->style(new Style(fontSize: 14, lineHeight: 21, textColor: 0xFF6B7485)),
            Button::make('Show the clipboard')->onPress(function (): void {
                Clipboard::getText(function (?string $text): void {
                    $this->clipboard = $text ?? '(empty)';
                });
            }),
            Text::make($this->clipboard)->style(new Style(fontSize: 14, textColor: 0xFF12B76A)),
        )->style(new Style(flexGrow: 1, padding: 28, gap: 14, backgroundColor: 0xFFF7F8FA));
    }
}

App::run(new Paragraphs());
