<?php

declare(strict_types=1);

namespace App\Services;

use App\Domain\Conversation;
use App\Domain\ConversationKind;
use App\Domain\Entry;
use App\Domain\Member;
use App\Domain\Message;
use App\Domain\Run;
use App\Domain\SearchHit;
use DateTimeImmutable;

/** The Acme Inc workspace, in memory: members, conversations and their messages. */
final class Workspace
{
    public const string NAME = 'Acme Inc';

    public const string ME = 'u-me';

    /** @var array<string, Member> */
    private array $members = [];

    /** @var array<string, Conversation> */
    private array $conversations = [];

    /** @var array<string, list<Message>> */
    private array $messages = [];

    /** @var array<string, list<Message>> replies by parent message id */
    private array $replies = [];

    private int $sequence = 0;

    public function __construct()
    {
        foreach ([
            ['u-me', 'David Balbino', 'Product engineer', 12],
            ['u-ana', 'Ana Ribeiro', 'Design lead', 47],
            ['u-bruno', 'Bruno Costa', 'Staff engineer', 53],
            ['u-carla', 'Carla Mendes', 'Product manager', 45],
            ['u-diego', 'Diego Alves', 'iOS engineer', 59, false],
            ['u-erica', 'Érica Lima', 'Marketing', 44],
            ['u-felipe', 'Felipe Rocha', 'Data', 60, false],
        ] as $row) {
            [$id, $name, $title, $photo, $active] = $row + [4 => true];
            $this->members[$id] = new Member($id, $name, $title, "asset://assets/avatars/{$photo}.jpg", $active);
        }

        foreach ([
            new Conversation('general', 'general', topic: 'Company-wide announcements and work-based matters', members: 48),
            new Conversation('announcements', 'announcements', topic: 'Only the important stuff', members: 48),
            new Conversation('design', 'design', topic: 'Pixels, prototypes and critique', members: 12, unread: 3),
            new Conversation('engineering', 'engineering', topic: 'Builds, releases and incidents', members: 21, unread: 8, mentions: 1),
            new Conversation('random', 'random', topic: 'Non-work banter and water cooler conversation', members: 46),
            new Conversation('product-launch', 'product-launch', ConversationKind::Private, 'Launch week war room', 7),
            new Conversation('dm-ana', 'Ana Ribeiro', ConversationKind::Direct, memberId: 'u-ana', unread: 1, mentions: 1),
            new Conversation('dm-bruno', 'Bruno Costa', ConversationKind::Direct, memberId: 'u-bruno'),
            new Conversation('dm-carla', 'Carla Mendes', ConversationKind::Direct, memberId: 'u-carla'),
            new Conversation('dm-diego', 'Diego Alves', ConversationKind::Direct, memberId: 'u-diego'),
        ] as $conversation) {
            $this->conversations[$conversation->id] = $conversation;
            $this->messages[$conversation->id] = [];
        }

        (new Seed())->into($this);
    }

    public function me(): Member
    {
        return $this->members[self::ME];
    }

    public function member(string $id): Member
    {
        return $this->members[$id];
    }

    public function conversation(string $id): Conversation
    {
        return $this->conversations[$id];
    }

    /** @return list<Conversation> */
    public function channels(): array
    {
        return array_values(array_filter($this->conversations, static fn (Conversation $c): bool => !$c->isDirect()));
    }

    /** @return list<Conversation> */
    public function directMessages(): array
    {
        return array_values(array_filter($this->conversations, static fn (Conversation $c): bool => $c->isDirect()));
    }

    /** @return list<Message> */
    public function messages(string $conversationId): array
    {
        return $this->messages[$conversationId];
    }

    public function markRead(string $conversationId): void
    {
        $this->conversations[$conversationId] = $this->conversations[$conversationId]->read();
    }

    public function post(string $conversationId, string $authorId, string $text, ?DateTimeImmutable $at = null): Message
    {
        $message = new Message('m'.++$this->sequence, $authorId, $at ?? new DateTimeImmutable(), $text);
        $this->messages[$conversationId][] = $message;

        return $message;
    }

    public function append(string $conversationId, Message $message): void
    {
        $this->messages[$conversationId][] = $message;
    }

    public function message(string $conversationId, string $messageId): ?Message
    {
        foreach ($this->messages[$conversationId] as $message) {
            if ($message->id === $messageId) {
                return $message;
            }
        }

        return null;
    }

    public function appendReply(string $parentId, Message $reply): void
    {
        $this->replies[$parentId][] = $reply;
    }

    /** @return list<Message> */
    public function replies(string $parentId): array
    {
        return $this->replies[$parentId] ?? [];
    }

    public function reply(string $conversationId, string $parentId, string $text): void
    {
        $reply = new Message('r'.++$this->sequence, self::ME, new DateTimeImmutable(), $text);
        $this->replies[$parentId][] = $reply;
        $parent = $this->message($conversationId, $parentId);
        if ($parent !== null) {
            $this->update($conversationId, $parent->withReply($reply));
        }
    }

    /**
     * A thread as timeline rows, oldest first: the parent, then its replies.
     *
     * @return list<Entry>
     */
    public function thread(string $conversationId, string $parentId): array
    {
        $parent = $this->message($conversationId, $parentId);
        if ($parent === null) {
            return [];
        }
        $entries = [$this->entry($parent, null)];
        $previous = null;
        foreach ($this->replies($parentId) as $reply) {
            $entries[] = $this->entry($reply, $previous);
            $previous = $reply;
        }

        return $entries;
    }

    public function toggleReaction(string $conversationId, string $messageId, string $emoji): void
    {
        foreach ($this->messages[$conversationId] as $index => $message) {
            if ($message->id === $messageId) {
                $this->messages[$conversationId][$index] = $message->toggleReaction($emoji, self::ME);
            }
        }
    }

    /** Replaces a message by id (reactions and replies return a new value). */
    public function update(string $conversationId, Message $message): void
    {
        foreach ($this->messages[$conversationId] as $index => $current) {
            if ($current->id === $message->id) {
                $this->messages[$conversationId][$index] = $message;
            }
        }
    }

    /**
     * Channels and people whose name matches, then the newest messages that
     * contain the query, as the search popover lists them.
     *
     * @return list<SearchHit>
     */
    public function search(string $query, int $limit = 8): array
    {
        $query = trim($query);
        if (mb_strlen($query) < 2) {
            return [];
        }
        $hits = [];
        foreach ($this->conversations as $conversation) {
            if (mb_stripos($conversation->name, $query) !== false) {
                $peer = $conversation->memberId === null ? null : $this->member($conversation->memberId);
                $hits[] = SearchHit::conversation($conversation, $peer);
            }
        }
        $found = [];
        foreach ($this->messages as $conversationId => $messages) {
            foreach ($messages as $message) {
                if (mb_stripos($message->text, $query) !== false) {
                    $found[] = [$conversationId, $message];
                }
            }
        }
        usort($found, static fn (array $a, array $b): int => $b[1]->at <=> $a[1]->at);
        foreach ($found as [$conversationId, $message]) {
            $hits[] = SearchHit::message($this->conversation($conversationId), $message, $this->member($message->authorId), $query);
        }

        return array_slice($hits, 0, $limit);
    }

    /**
     * The conversation as timeline rows, newest first: the shape of an
     * inverted list, whose row 0 rests at the bottom.
     *
     * @return list<Entry>
     */
    public function timeline(string $conversationId): array
    {
        $conversation = $this->conversation($conversationId);
        $entries = [Entry::intro($conversation, $conversation->memberId === null ? null : $this->member($conversation->memberId))];
        $previous = null;
        $day = null;
        foreach ($this->messages[$conversationId] as $message) {
            $label = self::dayLabel($message->at);
            if ($label !== $day) {
                $entries[] = Entry::day($label);
                $day = $label;
                $previous = null;
            }
            $entries[] = $this->entry($message, $previous);
            $previous = $message;
        }

        return array_reverse($entries);
    }

    private function entry(Message $message, ?Message $previous): Entry
    {
        $names = array_map(static fn (Member $member): string => $member->name, array_values($this->members));

        return Entry::message(
            $message,
            $this->members[$message->authorId],
            $message->continues($previous),
            Run::parse($message->text, $names, $this->me()->name),
        );
    }

    private static function dayLabel(DateTimeImmutable $at): string
    {
        $today = new DateTimeImmutable('today');

        return match ($at->setTime(0, 0)->format('Y-m-d')) {
            $today->format('Y-m-d') => 'Today',
            $today->modify('-1 day')->format('Y-m-d') => 'Yesterday',
            default => $at->format('l, F jS'),
        };
    }
}
