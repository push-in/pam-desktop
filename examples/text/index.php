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

    #[State]
    public string $lines = '';

    public function render(): Column
    {
        return Column::make(
            Text::make('Selectable text')->style(new Style(fontSize: 24, fontWeight: 700, textColor: 0xFF12161C)),
            Text::make('PAM Desktop draws this paragraph with the text engine of the operating system. Drag across it to select, double click a word, triple click for everything, then copy with Ctrl+C.')
                ->selectable()
                ->style(new Style(fontSize: 16, lineHeight: 25, textColor: 0xFF2A3140)),
            Text::make('This second paragraph is not selectable: pressing it does nothing.')
                ->style(new Style(fontSize: 14, lineHeight: 21, textColor: 0xFF6B7485)),
            Text::rich(
                'Runs inside a paragraph can be pressed: ',
                Text::make('open the terms')
                    ->style(new Style(textColor: 0xFF2F6BFF, fontWeight: 600))
                    ->on(\Pam\Desktop\EventKind::Press, function (): void {
                        $this->clipboard = 'Link pressed: terms';
                    }),
                ' or ',
                Text::make('the privacy policy')
                    ->style(new Style(textColor: 0xFF2F6BFF, fontWeight: 600))
                    ->on(\Pam\Desktop\EventKind::Press, function (): void {
                        $this->clipboard = 'Link pressed: privacy';
                    }),
                '. The host also reports how this paragraph wrapped.',
            )
                ->on(\Pam\Desktop\EventKind::TextLayout, function (string $payload): void {
                    $layout = \Pam\Desktop\TextLayoutEvent::fromPayload($payload);
                    $this->lines = "{$layout->lines} line(s), ".($layout->truncated ? 'truncated' : 'complete');
                })
                ->style(new Style(fontSize: 14, lineHeight: 21, textColor: 0xFF2A3140)),
            Text::make($this->lines)->style(new Style(fontSize: 12, textColor: 0xFF6B7485)),
            // Nested texts with padding, a border or a background are inline
            // boxes, like a CSS <kbd>: the padding moves the words around them.
            Text::rich(
                'Press ',
                Text::make('Ctrl')->style($kbd = new Style(
                    fontSize: 12,
                    fontWeight: 600,
                    paddingHorizontal: 6,
                    paddingVertical: 1,
                    borderWidth: 1,
                    borderBottomWidth: 2,
                    borderColor: 0xFFC9CED8,
                    borderRadius: 5,
                    backgroundColor: 0xFFFFFFFF,
                )),
                ' + ',
                Text::make('C')->style($kbd),
                ' to copy what you selected, ',
                Text::make('Esc')->style($kbd),
                ' to clear the selection.',
            )
                ->selectable()
                ->style(new Style(fontSize: 14, lineHeight: 24, textColor: 0xFF2A3140)),
            Button::make('Show the clipboard')->onPress(function (): void {
                Clipboard::getText(function (?string $text): void {
                    $this->clipboard = $text ?? '(empty)';
                });
            }),
            Text::make($this->clipboard)->style(new Style(fontSize: 14, textColor: 0xFF12B76A)),
            // One line that does not fit: it slides to its end and back.
            Text::make('Now playing · A line too long for its box slides to its end and back, resting on each side · PAM Desktop')
                ->numberOfLines(1)
                ->ellipsize(\Pam\Desktop\TextEllipsizeMode::Marquee)
                ->style(new Style(width: 340, fontSize: 14, textColor: 0xFFFFFFFF, backgroundColor: 0xFF12161C, paddingHorizontal: 14, paddingVertical: 9, borderRadius: 10)),
        )->style(new Style(flexGrow: 1, padding: 28, gap: 14, backgroundColor: 0xFFF7F8FA));
    }
}

App::run(new Paragraphs());
