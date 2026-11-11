<?php

declare(strict_types=1);

namespace App\Domain;

/** A channel or a direct message: everything the sidebar and the header show. */
final readonly class Conversation
{
    public function __construct(
        public string $id,
        public string $name,
        public ConversationKind $kind = ConversationKind::Channel,
        public string $topic = '',
        public int $members = 0,
        public int $unread = 0,
        public int $mentions = 0,
        public ?string $memberId = null,
    ) {
    }

    public function isDirect(): bool
    {
        return $this->kind === ConversationKind::Direct;
    }

    public function icon(): string
    {
        return $this->kind === ConversationKind::Private ? 'lock' : 'hash';
    }

    public function placeholder(): string
    {
        return $this->isDirect() ? "Message {$this->name}" : "Message #{$this->name}";
    }

    public function read(): self
    {
        return new self($this->id, $this->name, $this->kind, $this->topic, $this->members, 0, 0, $this->memberId);
    }
}
