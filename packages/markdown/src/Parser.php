<?php

declare(strict_types=1);

namespace Pam\Desktop\Markdown;

/**
 * Markdown to a small block tree: CommonMark's common subset plus GitHub
 * tables, task lists, strikethrough and bare links, and chat mentions (and
 * Slack's mrkdwn emphasis as a flavor).
 *
 * Blocks are arrays: `['heading', level, inline]`, `['paragraph', inline]`,
 * `['code', language, text]`, `['quote', blocks]`, `['list', ordered, start,
 * items]` (an item is `['task' => ?bool, 'blocks' => blocks]`), `['hr']`,
 * `['table', aligns, header, rows]` and `['image', alt, source]`. Inline
 * content is a list of runs: `['text' => …, 'bold' => …, 'italic' => …,
 * 'strike' => …, 'code' => …, 'link' => ?url, 'mention' => …]`.
 *
 * @internal
 */
final class Parser
{
    private const int MAX_DEPTH = 8;

    /**
     * @param bool $slack Slack's mrkdwn: `*bold*`, `_italic_`, `~strike~`
     * @param list<string> $people names a mention may hold spaces in (`@Ana Souza`)
     */
    public function __construct(
        private readonly bool $mentions = false,
        private readonly bool $slack = false,
        private readonly array $people = [],
    ) {
    }

    /** @return list<array<int, mixed>> */
    public function blocks(string $source, int $depth = 0): array
    {
        $lines = preg_split('/\R/u', str_replace("\t", '    ', $source)) ?: [];
        $blocks = [];
        $count = count($lines);
        $i = 0;
        while ($i < $count) {
            $line = $lines[$i];
            if (trim($line) === '') {
                ++$i;
                continue;
            }
            if (preg_match('/^ {0,3}(`{3,}|~{3,})\s*([^`\s]*)/', $line, $fence) === 1) {
                $code = [];
                ++$i;
                while ($i < $count && preg_match('/^ {0,3}'.preg_quote($fence[1][0], '/').'{'.strlen($fence[1]).',}\s*$/', $lines[$i]) !== 1) {
                    $code[] = $lines[$i];
                    ++$i;
                }
                ++$i;
                $blocks[] = ['code', strtolower($fence[2]), implode("\n", $code)];
                continue;
            }
            if (preg_match('/^ {0,3}(#{1,6})(?:\s+(.*?))?\s*#*\s*$/', $line, $heading) === 1) {
                $blocks[] = ['heading', strlen($heading[1]), $this->inline($heading[2] ?? '')];
                ++$i;
                continue;
            }
            if (self::isRule($line)) {
                $blocks[] = ['hr'];
                ++$i;
                continue;
            }
            if (preg_match('/^ {0,3}>/', $line) === 1) {
                $quoted = [];
                while ($i < $count && trim($lines[$i]) !== '' && (preg_match('/^ {0,3}>/', $lines[$i]) === 1 || !$this->startsBlock($lines[$i]))) {
                    $quoted[] = preg_replace('/^ {0,3}> ?/', '', $lines[$i]);
                    ++$i;
                }
                $blocks[] = ['quote', $depth < self::MAX_DEPTH ? $this->blocks(implode("\n", $quoted), $depth + 1) : []];
                continue;
            }
            if (self::listMarker($line) !== null) {
                [$block, $i] = $this->list($lines, $i, $depth);
                $blocks[] = $block;
                continue;
            }
            if ($i + 1 < $count && str_contains($line, '|') && self::isTableRule($lines[$i + 1])) {
                [$block, $i] = $this->table($lines, $i);
                $blocks[] = $block;
                continue;
            }
            $paragraph = [];
            while ($i < $count && trim($lines[$i]) !== '' && ($paragraph === [] || !$this->startsBlock($lines[$i]))) {
                $paragraph[] = $lines[$i];
                ++$i;
            }
            $text = implode("\n", $paragraph);
            if (preg_match('/^\s*!\[([^\]]*)\]\(\s*<?([^\s)>]+)>?(?:\s+"[^"]*")?\s*\)\s*$/', $text, $image) === 1) {
                $blocks[] = ['image', $image[1], $image[2]];
            } else {
                $blocks[] = ['paragraph', $this->inline($text)];
            }
        }

        return $blocks;
    }

    /** @return list<array<string, mixed>> */
    public function inline(string $text, array $style = []): array
    {
        $runs = [];
        $buffer = '';
        $flush = static function () use (&$buffer, &$runs, $style): void {
            if ($buffer !== '') {
                $runs[] = self::run($buffer, $style);
                $buffer = '';
            }
        };
        $length = strlen($text);
        $i = 0;
        while ($i < $length) {
            $char = $text[$i];
            $rest = substr($text, $i);
            if ($char === '\\' && $i + 1 < $length) {
                if ($text[$i + 1] === "\n") {
                    $buffer .= "\n";
                    $i += 2;
                    continue;
                }
                if (ctype_punct($text[$i + 1])) {
                    $buffer .= $text[$i + 1];
                    $i += 2;
                    continue;
                }
            }
            if ($char === "\n") {
                // Two trailing spaces make a hard break; otherwise a space.
                if (str_ends_with($buffer, '  ')) {
                    $buffer = rtrim($buffer, ' ')."\n";
                } else {
                    $buffer = rtrim($buffer, ' ').' ';
                }
                ++$i;
                continue;
            }
            if ($char === '`' && preg_match('/^(`+)(.+?)(?<!`)\1(?!`)/s', $rest, $code) === 1) {
                $flush();
                $inner = $code[2];
                if (strlen($inner) > 2 && $inner[0] === ' ' && str_ends_with($inner, ' ')) {
                    $inner = substr($inner, 1, -1);
                }
                $runs[] = self::run(str_replace("\n", ' ', $inner), $style + ['code' => true]);
                $i += strlen($code[0]);
                continue;
            }
            if (($char === '[' || ($char === '!' && ($text[$i + 1] ?? '') === '['))
                && preg_match('/^!?\[((?:[^\[\]]|\[[^\[\]]*\])*)\]\(\s*<?([^\s)>]*)>?(?:\s+"[^"]*")?\s*\)/', $rest, $link) === 1) {
                $flush();
                $label = $link[1] === '' ? $link[2] : $link[1];
                array_push($runs, ...$this->inline($label, ['link' => $link[2]] + $style));
                $i += strlen($link[0]);
                continue;
            }
            if ($char === '<' && preg_match('/^<((?:https?|mailto):[^\s>]+|[^\s@>]+@[^\s@>]+\.[a-z]{2,})>/i', $rest, $auto) === 1) {
                $flush();
                $url = str_contains($auto[1], ':') ? $auto[1] : 'mailto:'.$auto[1];
                $runs[] = self::run($auto[1], ['link' => $url] + $style);
                $i += strlen($auto[0]);
                continue;
            }
            if (($char === 'h' || $char === 'w') && !isset($style['link'])
                && ($i === 0 || !ctype_alnum($text[$i - 1]))
                && preg_match('/^(?:https?:\/\/|www\.)[^\s<]+/i', $rest, $bare) === 1) {
                $url = rtrim($bare[0], '.,;:!?*_~\'")');
                $flush();
                $runs[] = self::run($url, ['link' => str_starts_with(strtolower($url), 'www.') ? 'https://'.$url : $url] + $style);
                $i += strlen($url);
                continue;
            }
            if ($this->mentions && $char === '@' && ($i === 0 || ctype_space($text[$i - 1]) || $text[$i - 1] === '(')
                && (($name = $this->person($rest)) !== null
                    || preg_match('/^@[\p{L}\p{N}_][\p{L}\p{N}_.\-]*/u', $rest, $mention) === 1)) {
                $name ??= rtrim($mention[0], '.-');
                $flush();
                $runs[] = self::run($name, ['mention' => true] + $style);
                $i += strlen($name);
                continue;
            }
            $delimiters = $this->slack
                ? ['*' => 'bold', '_' => 'italic', '~' => 'strike']
                : ['**' => 'bold', '__' => 'bold', '~~' => 'strike', '*' => 'italic', '_' => 'italic'];
            foreach ($delimiters as $delimiter => $flag) {
                if (!str_starts_with($rest, $delimiter)) {
                    continue;
                }
                $size = strlen($delimiter);
                $after = $text[$i + $size] ?? ' ';
                // Underscores only open at a word boundary (snake_case stays text).
                if (ctype_space($after) || ($delimiter[0] === '_' && $i > 0 && ctype_alnum($text[$i - 1]))) {
                    continue;
                }
                $close = $this->closing($text, $delimiter, $i + $size);
                if ($close === null) {
                    continue;
                }
                $flush();
                array_push($runs, ...$this->inline(substr($text, $i + $size, $close - $i - $size), [$flag => true] + $style));
                $i = $close + $size;
                continue 2;
            }
            $buffer .= $char;
            ++$i;
        }
        $flush();

        return self::merged($runs);
    }

    /** `@Known Name` at the start of `$rest`, the longest known name first. */
    private function person(string $rest): ?string
    {
        $found = null;
        foreach ($this->people as $person) {
            $candidate = '@'.$person;
            if (str_starts_with($rest, $candidate)
                && !preg_match('/^[\p{L}\p{N}_]/u', substr($rest, strlen($candidate)))
                && ($found === null || strlen($candidate) > strlen($found))) {
                $found = $candidate;
            }
        }

        return $found;
    }

    /** The position of the delimiter that closes one opened before `$from`. */
    private function closing(string $text, string $delimiter, int $from): ?int
    {
        $size = strlen($delimiter);
        $position = $from;
        while (($found = strpos($text, $delimiter, $position)) !== false) {
            $before = $text[$found - 1] ?? ' ';
            $after = $text[$found + $size] ?? ' ';
            $inner = $found > $from && !ctype_space($before);
            // A single `*` must not close on half of a `**`.
            $whole = $size === 2 || ($after !== $delimiter && $before !== $delimiter);
            $boundary = $delimiter[0] !== '_' || !ctype_alnum($after);
            if ($inner && $whole && $boundary) {
                return $found;
            }
            $position = $found + $size;
        }

        return null;
    }

    /** @param list<string> $lines @return array{0: array<int, mixed>, 1: int} */
    private function list(array $lines, int $i, int $depth): array
    {
        $count = count($lines);
        $first = self::listMarker($lines[$i]);
        $ordered = $first['ordered'];
        $start = $first['number'];
        $items = [];
        while ($i < $count) {
            $marker = self::listMarker($lines[$i]);
            if ($marker === null || $marker['ordered'] !== $ordered || $marker['indent'] !== $first['indent']) {
                break;
            }
            $content = [$marker['text']];
            $indent = $marker['content'];
            ++$i;
            while ($i < $count) {
                $line = $lines[$i];
                if (trim($line) === '') {
                    // A blank line continues the item only when indented content follows.
                    $next = $lines[$i + 1] ?? '';
                    if (trim($next) !== '' && strlen($next) - strlen(ltrim($next, ' ')) >= $indent) {
                        $content[] = '';
                        ++$i;
                        continue;
                    }
                    break;
                }
                $spaces = strlen($line) - strlen(ltrim($line, ' '));
                if ($spaces >= $indent) {
                    $content[] = substr($line, $indent);
                } elseif (self::listMarker($line) === null && !$this->startsBlock($line)) {
                    $content[] = ltrim($line); // lazy continuation of the paragraph
                } else {
                    break;
                }
                ++$i;
            }
            $task = null;
            if (preg_match('/^\[([ xX])\]\s+/', $content[0], $box) === 1) {
                $task = $box[1] !== ' ';
                $content[0] = substr($content[0], strlen($box[0]));
            }
            $items[] = [
                'task' => $task,
                'blocks' => $depth < self::MAX_DEPTH ? $this->blocks(implode("\n", $content), $depth + 1) : [],
            ];
            // A blank line between items keeps the list going.
            $next = $i + 1 < $count ? self::listMarker($lines[$i + 1]) : null;
            if ($i < $count && trim($lines[$i]) === '' && $next !== null
                && $next['indent'] === $first['indent'] && $next['ordered'] === $ordered) {
                ++$i;
            }
        }

        return [['list', $ordered, $start, $items], $i];
    }

    /** @param list<string> $lines @return array{0: array<int, mixed>, 1: int} */
    private function table(array $lines, int $i): array
    {
        $header = self::cells($lines[$i]);
        $aligns = array_map(static function (string $cell): string {
            $cell = trim($cell);

            return match (true) {
                str_starts_with($cell, ':') && str_ends_with($cell, ':') => 'center',
                str_ends_with($cell, ':') => 'right',
                default => 'left',
            };
        }, self::cells($lines[$i + 1]));
        $i += 2;
        $rows = [];
        while ($i < count($lines) && trim($lines[$i]) !== '' && str_contains($lines[$i], '|')) {
            $rows[] = array_map(fn (string $cell): array => $this->inline(trim($cell)), array_pad(array_slice(self::cells($lines[$i]), 0, count($header)), count($header), ''));
            ++$i;
        }

        return [['table', $aligns, array_map(fn (string $cell): array => $this->inline(trim($cell)), $header), $rows], $i];
    }

    /** @return list<string> */
    private static function cells(string $line): array
    {
        $line = trim($line);
        $line = preg_replace('/^\||\|$/', '', $line) ?? $line;

        return preg_split('/(?<!\\\\)\|/', $line) ?: [];
    }

    private static function isRule(string $line): bool
    {
        return preg_match('/^ {0,3}([-*_])(?:\s*\1){2,}\s*$/', $line) === 1;
    }

    private static function isTableRule(string $line): bool
    {
        return preg_match('/^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/', $line) === 1 && str_contains($line, '-');
    }

    /** @return array{ordered: bool, number: int, indent: int, content: int, text: string}|null */
    private static function listMarker(string $line): ?array
    {
        if (self::isRule($line) || preg_match('/^( {0,3})([-*+]|(\d{1,9})[.)])( +|$)(.*)$/', $line, $match) !== 1) {
            return null;
        }

        return [
            'ordered' => $match[3] !== '',
            'number' => $match[3] === '' ? 1 : (int) $match[3],
            'indent' => strlen($match[1]),
            'content' => strlen($match[1]) + strlen($match[2]) + max(1, min(4, strlen($match[4]))),
            'text' => $match[5],
        ];
    }

    private function startsBlock(string $line): bool
    {
        return preg_match('/^ {0,3}(#{1,6}(\s|$)|>|`{3,}|~{3,})/', $line) === 1
            || self::isRule($line)
            || self::listMarker($line) !== null;
    }

    /** @param array<string, mixed> $style @return array<string, mixed> */
    private static function run(string $text, array $style): array
    {
        return ['text' => $text] + $style + [
            'bold' => false, 'italic' => false, 'strike' => false, 'code' => false, 'link' => null, 'mention' => false,
        ];
    }

    /** @param list<array<string, mixed>> $runs @return list<array<string, mixed>> */
    private static function merged(array $runs): array
    {
        $merged = [];
        foreach ($runs as $run) {
            $last = array_key_last($merged);
            if ($last !== null && array_diff_key($merged[$last], ['text' => 1]) == array_diff_key($run, ['text' => 1])) {
                $merged[$last]['text'] .= $run['text'];
            } else {
                $merged[] = $run;
            }
        }

        return $merged;
    }
}
