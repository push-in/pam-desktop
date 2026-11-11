<?php

declare(strict_types=1);

namespace App\Domain;

use DateTimeImmutable;

final readonly class Message
{
    /**
     * @param array<string, list<string>> $reactions emoji => ids of the members who reacted
     * @param list<string> $repliedBy ids of the members in the thread, newest first
     */
    public function __construct(
        public string $id,
        public string $authorId,
        public DateTimeImmutable $at,
        public string $text,
        public array $reactions = [],
        public int $replies = 0,
        public array $repliedBy = [],
        public ?DateTimeImmutable $lastReplyAt = null,
    ) {
    }

    public function time(): string
    {
        return $this->at->format('g:i A');
    }

    /** @return list<array{emoji: string, count: int, mine: bool}> */
    public function reactionChips(string $memberId): array
    {
        $chips = [];
        foreach ($this->reactions as $emoji => $who) {
            $chips[] = ['emoji' => (string) $emoji, 'count' => count($who), 'mine' => in_array($memberId, $who, true)];
        }

        return $chips;
    }

    public function repliesLabel(): string
    {
        return $this->replies === 1 ? '1 reply' : "{$this->replies} replies";
    }

    public function lastReplyLabel(): string
    {
        return $this->lastReplyAt === null ? '' : 'Last reply today at '.$this->lastReplyAt->format('g:i A');
    }

    /** Slack folds a message into the previous one by the same author within five minutes. */
    public function continues(?self $previous): bool
    {
        return $previous !== null
            && $previous->authorId === $this->authorId
            && $this->at->getTimestamp() - $previous->at->getTimestamp() < 300;
    }

    public function withReply(self $reply): self
    {
        return new self(
            $this->id, $this->authorId, $this->at, $this->text, $this->reactions,
            $this->replies + 1,
            array_values(array_unique([$reply->authorId, ...$this->repliedBy])),
            $reply->at,
        );
    }

    public function toggleReaction(string $emoji, string $memberId): self
    {
        $reactions = $this->reactions;
        $who = $reactions[$emoji] ?? [];
        $who = in_array($memberId, $who, true)
            ? array_values(array_diff($who, [$memberId]))
            : [...$who, $memberId];
        if ($who === []) {
            unset($reactions[$emoji]);
        } else {
            $reactions[$emoji] = $who;
        }

        return new self($this->id, $this->authorId, $this->at, $this->text, $reactions, $this->replies, $this->repliedBy, $this->lastReplyAt);
    }
}
