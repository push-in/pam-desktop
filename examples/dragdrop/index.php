<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\Style;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\InteractionRegion;
use Pam\Desktop\UI\Row;
use Pam\Desktop\UI\Text;

require __DIR__.'/../../vendor/autoload.php';

/**
 * Exercises drag and drop: cards move between the columns of a board, and
 * text dragged from another application becomes a new card.
 */
final class Board extends Component
{
    /** @var array<string, list<string>> */
    #[State]
    public array $columns = [
        'To do' => ['Design the icon', 'Write the changelog', 'Record the demo'],
        'Doing' => ['Windows backend'],
        'Done' => [],
    ];

    #[State]
    public string $status = 'Drag a card to another column';

    private function move(string $card, string $target): void
    {
        foreach ($this->columns as $name => $cards) {
            $this->columns[$name] = array_values(array_filter($cards, static fn (string $other): bool => $other !== $card));
        }
        $this->columns[$target][] = $card;
        $this->status = "Moved “{$card}” to {$target}";
    }

    public function render(): Column
    {
        $columns = [];
        foreach ($this->columns as $name => $cards) {
            $items = [];
            foreach ($cards as $card) {
                $items[] = InteractionRegion::make(
                    Text::make($card)->style(new Style(fontSize: 14, textColor: 0xFF12161C)),
                )
                    ->key($card)
                    ->draggable($card)
                    ->cursor('grab')
                    ->style(new Style(padding: 12, borderRadius: 10, backgroundColor: 0xFFFFFFFF, borderWidth: 1, borderColor: 0xFFE4E7EC));
            }
            $columns[] = InteractionRegion::make(
                Text::make(strtoupper($name).' · '.count($cards))->style(new Style(fontSize: 11, fontWeight: 700, textColor: 0xFF6B7485)),
                ...$items,
            )
                ->key($name)
                ->acceptsDrop()
                ->onDrop(fn (string $data) => $this->move($data, $name))
                ->style(new Style(width: 213, padding: 12, gap: 8, borderRadius: 14, backgroundColor: 0xFFEEF1F5));
        }

        return Column::make(
            Text::make('Drag and drop')->style(new Style(fontSize: 24, fontWeight: 700, textColor: 0xFF12161C)),
            Row::make(...$columns)->style(new Style(gap: 12, flexGrow: 1)),
            Text::make($this->status)->style(new Style(fontSize: 13, textColor: 0xFF5B6472)),
        )->style(new Style(flexGrow: 1, padding: 28, gap: 14, backgroundColor: 0xFFF7F8FA));
    }
}

App::run(new Board());
