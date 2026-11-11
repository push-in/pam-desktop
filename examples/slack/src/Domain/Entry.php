<?php

declare(strict_types=1);

namespace App\Domain;

/** A row of the conversation timeline: the intro, a day divider or a message. */
final readonly class Entry
{
    /** @param list<Run> $runs */
    private function __construct(
        public string $key,
        public ?string $day = null,
        public ?Message $message = null,
        public ?Member $author = null,
        public bool $continued = false,
        public array $runs = [],
        public ?Conversation $intro = null,
        public ?Member $peer = null,
    ) {
    }

    /** The top of a conversation: who it is with, or what the channel is for. */
    public static function intro(Conversation $conversation, ?Member $peer): self
    {
        return new self('intro-'.$conversation->id, intro: $conversation, peer: $peer);
    }

    public static function day(string $label): self
    {
        return new self('day-'.$label, day: $label);
    }

    /** @param list<Run> $runs */
    public static function message(Message $message, Member $author, bool $continued, array $runs): self
    {
        return new self($message->id, message: $message, author: $author, continued: $continued, runs: $runs);
    }

    public function isIntro(): bool
    {
        return $this->intro !== null;
    }

    public function isDay(): bool
    {
        return $this->day !== null;
    }
}
