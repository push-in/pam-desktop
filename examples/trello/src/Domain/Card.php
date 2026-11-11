<?php

declare(strict_types=1);

namespace App\Domain;

use DateTimeImmutable;

final readonly class Card
{
    /**
     * @param list<Label> $labels
     * @param list<string> $memberIds
     * @param array{0: int, 1: int}|null $checklist done and total items
     */
    public function __construct(
        public string $id,
        public string $title,
        public array $labels = [],
        public ?DateTimeImmutable $due = null,
        public bool $complete = false,
        public string $description = '',
        public int $comments = 0,
        public int $attachments = 0,
        public ?array $checklist = null,
        public array $memberIds = [],
        public ?string $cover = null,
        public bool $watching = false,
    ) {
    }

    public function with(mixed ...$changes): self
    {
        return new self(...[...get_object_vars($this), ...$changes]);
    }

    /** "Nov 14" */
    public function dueLabel(): string
    {
        return $this->due?->format('M j') ?? '';
    }

    /** complete, overdue, soon (within a day) or upcoming: the colour of the due badge. */
    public function dueState(DateTimeImmutable $now): string
    {
        return match (true) {
            $this->due === null => '',
            $this->complete => 'complete',
            $this->due < $now => 'overdue',
            $this->due < $now->modify('+1 day') => 'soon',
            default => 'upcoming',
        };
    }

    public function checklistLabel(): string
    {
        return $this->checklist === null ? '' : "{$this->checklist[0]}/{$this->checklist[1]}";
    }

    public function checklistDone(): bool
    {
        return $this->checklist !== null && $this->checklist[0] === $this->checklist[1];
    }

    public function hasBadges(): bool
    {
        return $this->watching || $this->due !== null || $this->description !== '' || $this->comments > 0
            || $this->attachments > 0 || $this->checklist !== null || $this->memberIds !== [];
    }
}
