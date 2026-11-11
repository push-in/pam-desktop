<?php

declare(strict_types=1);

use App\Screens\Board\BoardScreen;
use App\Services\Board;
use Pam\Desktop\App;

use function Pam\Desktop\Testing\mount;

require dirname(__DIR__).'/vendor/autoload.php';

App::components(dirname(__DIR__).'/src', dirname(__DIR__).'/.pam-desktop/components');

$check = static function (bool $condition, string $message): void {
    if (!$condition) {
        fwrite(STDERR, "FAIL {$message}\n");
        exit(1);
    }
};
$titles = static fn (Board $board, string $list): array => array_map(static fn ($card): string => $card->title, $board->cards($list));

$board = new Board(new DateTimeImmutable('2026-11-10'));
$check(array_map(static fn ($list): string => $list->title, $board->lists()) === ['Backlog', 'To Do', 'In Progress', 'Review', 'Done'], 'the board opens with its five lists');

// Moving: before a card of another list, to the end, and within a list.
$dark = $board->cards('backlog')[2];
$webhooks = $board->cards('doing')[1];
$board->move($dark->id, 'doing', $webhooks->id);
$check($titles($board, 'doing')[1] === 'Dark mode for the public site' && !in_array($dark->title, $titles($board, 'backlog'), true), 'a card lands before the card it was dropped on');
$board->move($dark->id, 'done');
$check(array_last($titles($board, 'done')) === 'Dark mode for the public site' && count($board->cards('doing')) === 3, 'a drop on a list puts the card at its end');
$board->move($dark->id, 'done', $dark->id);
$check(array_last($titles($board, 'done')) === 'Dark mode for the public site', 'a card dropped on itself stays where it is');
$check($board->listOf($dark->id)?->title === 'Done', 'a card knows its list');

// Adding, completing, due states.
$check($board->add('todo', '   ') === null, 'a blank title adds nothing');
$card = $board->add('todo', 'Record a demo video');
$check($card !== null && array_last($titles($board, 'todo')) === 'Record a demo video', 'a card is added at the end of its list');
$pricing = $board->cards('todo')[2];
$check($pricing->dueState($board->today()) === 'soon', 'a card due today is due soon');
$check($board->cards('doing')[1]->dueState($board->today()) === 'overdue', 'yesterday is overdue');
$board->toggleComplete($pricing->id);
$check($board->card($pricing->id)->dueState($board->today()) === 'complete', 'a completed card is complete whatever its date');

// The screen, mounted without a host: a drop and the detail dialog.
$screen = mount(BoardScreen::class);
$check(str_contains($screen->text(), 'Product Launch') && str_contains($screen->text(), 'Competitor pricing teardown'), 'the board renders');
$first = $screen->instance->board->cards('backlog')[0];
$screen->instance->dragStart($first->id);
$screen->instance->drop(['card' => $first->id, 'list' => 'review']);
$screen->render();
$check($screen->instance->dragging === null && $screen->instance->board->listOf($first->id)?->id === 'review', 'a drop moves the card and ends the drag');
$screen->instance->open($first->id);
$screen->render();
$check(str_contains($screen->text(), 'in list') && str_contains($screen->text(), 'Add to card'), 'a card opens its detail');
$screen->instance->close();
$screen->unmount();

echo "Trello example tests passed.\n";
