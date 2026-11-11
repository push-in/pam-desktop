<?php

declare(strict_types=1);

namespace App\Services;

use App\Domain\BoardList;
use App\Domain\Card;
use App\Domain\Label;
use App\Domain\LabelColor;
use App\Domain\Member;
use DateTimeImmutable;

/** One board: its lists, in order, and their cards. */
final class Board
{
    public const string NAME = 'Product Launch';

    public const string WORKSPACE = 'Acme Workspace';

    /** @var array<string, BoardList> */
    private array $lists = [];

    /** @var array<string, Card> */
    private array $cards = [];

    /** @var array<string, Member> */
    private array $members = [];

    private int $sequence = 0;

    public function __construct(private readonly DateTimeImmutable $today = new DateTimeImmutable('today'))
    {
        foreach ([['ana', 'Ana Ribeiro', '#1F845A'], ['bruno', 'Bruno Costa', '#C9372C'], ['carla', 'Carla Mendes', '#6E5DC6'], ['david', 'David Balbino', '#0C66E4']] as [$id, $name, $color]) {
            $this->members[$id] = new Member($id, $name, $color);
        }
        $design = new Label(LabelColor::Purple, 'Design');
        $backend = new Label(LabelColor::Blue, 'Backend');
        $urgent = new Label(LabelColor::Red, 'Urgent');
        $marketing = new Label(LabelColor::Orange, 'Marketing');
        $research = new Label(LabelColor::Green, 'Research');
        $qa = new Label(LabelColor::Yellow, 'QA');
        $day = fn (string $offset): DateTimeImmutable => $this->today->modify($offset)->setTime(17, 0);

        $this->list('backlog', 'Backlog', [
            new Card('', 'Competitor pricing teardown', [$research], description: 'Plans and limits of the top five tools.', comments: 2),
            new Card('', 'Localize onboarding emails', [$marketing], memberIds: ['carla']),
            new Card('', 'Dark mode for the public site', [$design]),
            new Card('', 'Usage-based billing spike', [$backend, $research], checklist: [1, 4], memberIds: ['bruno']),
        ]);
        $this->list('todo', 'To Do', [
            new Card('', 'Launch moodboard', [$design], description: 'Tone, photography and type.', attachments: 6, memberIds: ['ana', 'carla'], cover: 'asset://assets/covers/moodboard.jpg'),
            new Card('', 'Write the launch blog post', [$marketing], due: $day('+3 days'), memberIds: ['carla'], watching: true),
            new Card('', 'Pricing page copy review', [$marketing, $urgent], due: $day('+0 days'), comments: 5),
            new Card('', 'Set up status page', [$backend], checklist: [0, 3]),
        ]);
        $this->list('doing', 'In Progress', [
            new Card('', 'New onboarding flow', [$design], due: $day('+2 days'), description: 'Three screens, light and dark.', comments: 8, attachments: 3, checklist: [5, 7], memberIds: ['ana', 'david'], cover: 'asset://assets/covers/design.jpg', watching: true),
            new Card('', 'Webhooks v2', [$backend], due: $day('-1 day'), checklist: [3, 5], memberIds: ['bruno', 'david']),
            new Card('', 'Load test the sync service', [$backend, $qa], memberIds: ['bruno']),
        ]);
        $this->list('review', 'Review', [
            new Card('', 'Release notes 2.4', [$marketing], description: 'Draft in the shared doc.', comments: 3, memberIds: ['carla', 'david']),
            new Card('', 'Accessibility audit fixes', [$qa, $design], checklist: [9, 9], memberIds: ['ana']),
        ]);
        $this->list('done', 'Done', [
            new Card('', 'Launch announcement video', [$marketing], due: $day('-3 days'), complete: true, attachments: 2, memberIds: ['carla'], cover: 'asset://assets/covers/launch.jpg'),
            new Card('', 'Migrate to the new CDN', [$backend], due: $day('-5 days'), complete: true, checklist: [4, 4], memberIds: ['bruno']),
            new Card('', 'Interview 12 beta customers', [$research], comments: 12, memberIds: ['ana', 'carla']),
        ]);
    }

    public function today(): DateTimeImmutable
    {
        return $this->today;
    }

    /** @return list<BoardList> */
    public function lists(): array
    {
        return array_values($this->lists);
    }

    /** @return list<Card> */
    public function cards(string $listId): array
    {
        return array_map(fn (string $id): Card => $this->cards[$id], $this->lists[$listId]->cardIds);
    }

    public function card(string $id): Card
    {
        return $this->cards[$id];
    }

    public function member(string $id): Member
    {
        return $this->members[$id];
    }

    /** @return list<Member> */
    public function members(): array
    {
        return array_values($this->members);
    }

    public function listOf(string $cardId): ?BoardList
    {
        foreach ($this->lists as $list) {
            if (in_array($cardId, $list->cardIds, true)) {
                return $list;
            }
        }

        return null;
    }

    /** Moves a card into a list, before another card or at the end. */
    public function move(string $cardId, string $listId, ?string $beforeId = null): void
    {
        if (!isset($this->cards[$cardId], $this->lists[$listId]) || $cardId === $beforeId) {
            return;
        }
        foreach ($this->lists as $id => $list) {
            $this->lists[$id] = $list->withCards(array_values(array_filter($list->cardIds, static fn (string $id): bool => $id !== $cardId)));
        }
        $ids = $this->lists[$listId]->cardIds;
        $at = $beforeId === null ? false : array_search($beforeId, $ids, true);
        array_splice($ids, $at === false ? count($ids) : $at, 0, [$cardId]);
        $this->lists[$listId] = $this->lists[$listId]->withCards($ids);
    }

    public function add(string $listId, string $title): ?Card
    {
        $title = trim($title);
        if ($title === '' || !isset($this->lists[$listId])) {
            return null;
        }
        $card = new Card('c'.++$this->sequence, $title);
        $this->cards[$card->id] = $card;
        $this->lists[$listId] = $this->lists[$listId]->withCards([...$this->lists[$listId]->cardIds, $card->id]);

        return $card;
    }

    public function addList(string $title): void
    {
        $title = trim($title);
        if ($title !== '') {
            $id = 'l'.++$this->sequence;
            $this->lists[$id] = new BoardList($id, $title);
        }
    }

    public function toggleComplete(string $cardId): void
    {
        $card = $this->cards[$cardId];
        $this->cards[$cardId] = $card->with(complete: !$card->complete);
    }

    public function toggleWatch(string $cardId): void
    {
        $card = $this->cards[$cardId];
        $this->cards[$cardId] = $card->with(watching: !$card->watching);
    }

    public function rename(string $cardId, string $title): void
    {
        if (trim($title) !== '') {
            $this->cards[$cardId] = $this->cards[$cardId]->with(title: trim($title));
        }
    }

    /** @param list<Card> $cards */
    private function list(string $id, string $title, array $cards): void
    {
        $ids = [];
        foreach ($cards as $card) {
            $card = $card->with(id: 'c'.++$this->sequence);
            $this->cards[$card->id] = $card;
            $ids[] = $card->id;
        }
        $this->lists[$id] = new BoardList($id, $title, $ids);
    }
}
