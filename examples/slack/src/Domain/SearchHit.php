<?php

declare(strict_types=1);

namespace App\Domain;

/** A search result: a channel, a person or a message, and where it opens. */
final readonly class SearchHit
{
    /** @param list<Run> $runs the message text, matches styled `match` */
    private function __construct(
        public string $key,
        public string $conversationId,
        public string $label,
        public string $detail = '',
        public ?string $icon = null,
        public ?string $avatar = null,
        public array $runs = [],
    ) {
    }

    public static function conversation(Conversation $conversation, ?Member $peer): self
    {
        return $peer === null
            ? new self('c-'.$conversation->id, $conversation->id, $conversation->name, $conversation->topic, icon: $conversation->icon())
            : new self('c-'.$conversation->id, $conversation->id, $peer->name, $peer->title, avatar: $peer->avatar);
    }

    public static function message(Conversation $conversation, Message $message, Member $author, string $query): self
    {
        $where = $conversation->isDirect() ? $conversation->name : '#'.$conversation->name;

        return new self(
            'm-'.$message->id,
            $conversation->id,
            $author->name,
            $where.' · '.$message->time(),
            avatar: $author->avatar,
            runs: self::highlight($message->text, $query),
        );
    }

    public function isMessage(): bool
    {
        return $this->runs !== [];
    }

    /** @return list<Run> */
    private static function highlight(string $text, string $query): array
    {
        // The snippet reads as plain text: mrkdwn markers around words go.
        $text = preg_replace('/(?<![\w*_`])[*_`]+(?=\S)|(?<=\S)[*_`]+(?![\w*_`])/u', '', $text) ?? $text;
        $parts = preg_split('/('.preg_quote($query, '/').')/iu', $text, flags: PREG_SPLIT_DELIM_CAPTURE | PREG_SPLIT_NO_EMPTY) ?: [$text];

        return array_map(
            static fn (string $part): Run => new Run($part, mb_strtolower($part) === mb_strtolower($query) ? 'match' : 'plain'),
            $parts,
        );
    }
}
