<?php

declare(strict_types=1);

namespace App\Domain;

/** One styled piece of a message body. */
final readonly class Run
{
    public function __construct(
        public string $text,
        public string $style = 'plain',
    ) {
    }

    /**
     * Slack's mrkdwn subset: *bold*, _italic_, `code`, @mentions and links.
     *
     * @param list<string> $people names that turn into mention chips
     * @param string|null $me the reader, whose mentions are highlighted
     * @return list<self>
     */
    public static function parse(string $text, array $people, ?string $me = null): array
    {
        $names = implode('|', array_map(static fn (string $name): string => preg_quote($name, '/'), $people));
        $pattern = '/(\*[^*\n]+\*|_[^_\n]+_|`[^`\n]+`|https?:\/\/\S+'.($names === '' ? '' : "|@(?:{$names})").')/u';
        $runs = [];
        foreach (preg_split($pattern, $text, flags: PREG_SPLIT_DELIM_CAPTURE | PREG_SPLIT_NO_EMPTY) ?: [] as $part) {
            $runs[] = match (true) {
                str_starts_with($part, '*') && str_ends_with($part, '*') && strlen($part) > 2 => new self(substr($part, 1, -1), 'bold'),
                str_starts_with($part, '_') && str_ends_with($part, '_') && strlen($part) > 2 => new self(substr($part, 1, -1), 'italic'),
                str_starts_with($part, '`') && str_ends_with($part, '`') && strlen($part) > 2 => new self(substr($part, 1, -1), 'code'),
                str_starts_with($part, 'http') => new self($part, 'link'),
                $me !== null && $part === "@{$me}" => new self($part, 'mention-me'),
                str_starts_with($part, '@') && $names !== '' && preg_match("/^@(?:{$names})$/u", $part) === 1 => new self($part, 'mention'),
                default => new self($part),
            };
        }

        return $runs;
    }
}
