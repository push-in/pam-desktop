<?php

declare(strict_types=1);

use Pam\Desktop\Align;
use Pam\Desktop\App;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\Style;
use Pam\Desktop\UI\Button;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\Row;
use Pam\Desktop\UI\Text;
use Pam\Desktop\UI\VirtualizedList;

require __DIR__.'/../../vendor/autoload.php';

/**
 * An inverted list, the shape of a conversation: the newest message is row
 * 0 and rests at the bottom; older ones go up.
 */
final class Chat extends Component
{
    /** @var list<string> newest first */
    #[State]
    public array $messages = [];

    #[State]
    public float $offset = 0.0;

    public function mount(): void
    {
        for ($index = 1; $index <= 30; $index++) {
            $this->messages[] = "Message {$index} (older as you go up)";
        }
        $this->messages[0] = 'Message 1 (the newest, at the bottom)';
    }

    public function render(): Column
    {
        return Column::make(
            Row::make(
                Text::make(sprintf('Inverted list · offset %d', (int) $this->offset))->style(new Style(fontSize: 14, fontWeight: 600, textColor: 0xFF12161C, flexGrow: 1)),
                Button::make('New message')->onPress(function (): void {
                    array_unshift($this->messages, 'New message '.(count($this->messages) + 1));
                }),
            )->style(new Style(alignItems: Align::Center, gap: 12)),
            VirtualizedList::of(
                $this->messages,
                static fn (string $message, int $index): Row => Row::make(
                    Text::make($message)->style(new Style(fontSize: 14, textColor: $index % 3 === 0 ? 0xFFFFFFFF : 0xFF12161C)),
                )->style(new Style(
                    marginTop: 4,
                    marginBottom: 4,
                    paddingHorizontal: 14,
                    paddingVertical: 10,
                    borderRadius: 16,
                    alignSelf: $index % 3 === 0 ? Align::End : Align::Start,
                    backgroundColor: $index % 3 === 0 ? 0xFF2F6BFF : 0xFFE9ECF1,
                )),
                static fn (string $message): string => $message,
            )
                ->inverted()
                ->onScroll(function (string $offset): void {
                    $this->offset = (float) $offset;
                })
                ->style(new Style(flexGrow: 1, padding: 12, borderRadius: 14, backgroundColor: 0xFFFFFFFF)),
        )->style(new Style(flexGrow: 1, padding: 20, gap: 12, backgroundColor: 0xFFF7F8FA));
    }
}

App::run(new Chat());
