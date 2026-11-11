<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

/**
 * The text model of a `CodeEditor`: lines, cursors with their selections,
 * and undo history. Positions are 0-based [line, column] in characters
 * (tabs count as one; the editor expands them on screen).
 *
 * A cursor is `[line, column, anchorLine, anchorColumn]`: the anchor is
 * where its selection started (equal to the position when nothing is
 * selected). The first cursor is the primary one.
 */
final class CodeDocument
{
    /** @var list<string> */
    public array $lines;

    /** @var list<array{0: int, 1: int, 2: int, 3: int}> */
    public array $cursors = [[0, 0, 0, 0]];

    /** @var list<array{lines: list<string>, cursors: list<array{0: int, 1: int, 2: int, 3: int}>}> */
    private array $undo = [];

    /** @var list<array{lines: list<string>, cursors: list<array{0: int, 1: int, 2: int, 3: int}>}> */
    private array $redo = [];

    private string $lastKind = '';
    private float $lastEdit = 0.0;

    /**
     * What changed since `takeEdits()`, in order: `['line', n]` (one line's
     * text), `['splice', at, removed, inserted]` (lines replaced) or `['all']`.
     *
     * @var list<array<int, int|string>>
     */
    private array $edits = [['all']];

    public function __construct(string $text = '', public int $tabSize = 4)
    {
        $this->lines = explode("\n", str_replace(["\r\n", "\r"], "\n", $text));
    }

    public function text(): string
    {
        return implode("\n", $this->lines);
    }

    public function lineCount(): int
    {
        return count($this->lines);
    }

    public function length(int $line): int
    {
        return mb_strlen($this->lines[$line] ?? '');
    }

    /** @return list<array<int, int|string>> the edits since the last call (see `$edits`) */
    public function takeEdits(): array
    {
        $edits = $this->edits;
        $this->edits = [];

        return $edits;
    }

    // ---- positions ---------------------------------------------------------------

    /** The on-screen column of character `$column` (tabs to the next stop). */
    public function visual(int $line, int $column): int
    {
        $text = mb_substr($this->lines[$line] ?? '', 0, $column);
        if (!str_contains($text, "\t")) {
            return mb_strlen($text);
        }
        $visual = 0;
        foreach (mb_str_split($text) as $char) {
            $visual = $char === "\t" ? intdiv($visual, $this->tabSize) * $this->tabSize + $this->tabSize : $visual + 1;
        }

        return $visual;
    }

    /** The character column nearest to on-screen column `$visual`. */
    public function column(int $line, int $visual): int
    {
        $text = $this->lines[$line] ?? '';
        if (!str_contains($text, "\t")) {
            return max(0, min($visual, mb_strlen($text)));
        }
        $at = 0;
        foreach (mb_str_split($text) as $index => $char) {
            $next = $char === "\t" ? intdiv($at, $this->tabSize) * $this->tabSize + $this->tabSize : $at + 1;
            if ($visual < ($at + $next) / 2) {
                return $index;
            }
            $at = $next;
        }

        return mb_strlen($text);
    }

    /** The line with tabs expanded to spaces, as shown. */
    public function display(int $line): string
    {
        $text = $this->lines[$line] ?? '';
        if (!str_contains($text, "\t")) {
            return $text;
        }
        $out = '';
        foreach (mb_str_split($text) as $char) {
            $out .= $char === "\t" ? str_repeat(' ', $this->tabSize - mb_strlen($out) % $this->tabSize) : $char;
        }

        return $out;
    }

    // ---- cursors -------------------------------------------------------------------

    public function setCursor(int $line, int $column, bool $extend = false, bool $add = false): void
    {
        $line = max(0, min($line, $this->lineCount() - 1));
        $column = max(0, min($column, $this->length($line)));
        if ($add) {
            $this->cursors[] = [$line, $column, $line, $column];
            $this->merge();

            return;
        }
        $primary = $this->cursors[0];
        $this->cursors = [$extend ? [$line, $column, $primary[2], $primary[3]] : [$line, $column, $line, $column]];
    }

    public function selectAll(): void
    {
        $last = $this->lineCount() - 1;
        $this->cursors = [[$last, $this->length($last), 0, 0]];
    }

    public function singleCursor(): void
    {
        $this->cursors = [$this->cursors[0]];
    }

    /**
     * Moves every cursor: `left`, `right`, `up`, `down`, `home`, `end`,
     * `wordLeft`, `wordRight`, `pageUp`, `pageDown`, `top`, `bottom`.
     */
    public function move(string $where, bool $extend = false, int $page = 20): void
    {
        foreach ($this->cursors as $index => [$line, $column, $anchorLine, $anchorColumn]) {
            $selected = [$line, $column] !== [$anchorLine, $anchorColumn];
            if (!$extend && $selected && ($where === 'left' || $where === 'right')) {
                // Collapsing a selection goes to its edge, like every editor.
                [$start, $end] = self::ordered([$line, $column, $anchorLine, $anchorColumn]);
                [$line, $column] = $where === 'left' ? $start : $end;
            } else {
                [$line, $column] = match ($where) {
                    'left' => $column > 0 ? [$line, $column - 1] : ($line > 0 ? [$line - 1, $this->length($line - 1)] : [0, 0]),
                    'right' => $column < $this->length($line) ? [$line, $column + 1] : ($line < $this->lineCount() - 1 ? [$line + 1, 0] : [$line, $column]),
                    'up' => $line > 0 ? [$line - 1, $this->column($line - 1, $this->visual($line, $column))] : [0, 0],
                    'down' => $line < $this->lineCount() - 1 ? [$line + 1, $this->column($line + 1, $this->visual($line, $column))] : [$line, $this->length($line)],
                    'pageUp' => [max(0, $line - $page), $this->column(max(0, $line - $page), $this->visual($line, $column))],
                    'pageDown' => [min($this->lineCount() - 1, $line + $page), $this->column(min($this->lineCount() - 1, $line + $page), $this->visual($line, $column))],
                    'home' => [$line, ($indent = strlen($this->lines[$line]) - strlen(ltrim($this->lines[$line]))) !== $column ? $indent : 0],
                    'end' => [$line, $this->length($line)],
                    'top' => [0, 0],
                    'bottom' => [$this->lineCount() - 1, $this->length($this->lineCount() - 1)],
                    'wordLeft' => $this->word($line, $column, -1),
                    'wordRight' => $this->word($line, $column, 1),
                    default => [$line, $column],
                };
            }
            $this->cursors[$index] = $extend ? [$line, $column, $anchorLine, $anchorColumn] : [$line, $column, $line, $column];
        }
        $this->merge();
    }

    /** The selected text of every cursor, joined by newlines (whole lines when nothing is selected). */
    public function selectedText(): string
    {
        $parts = [];
        foreach ($this->cursors as $cursor) {
            [$start, $end] = self::ordered($cursor);
            $parts[] = $start === $end ? $this->lines[$start[0]]."\n" : $this->range($start, $end);
        }

        return implode("\n", $parts);
    }

    /** Selects the word at the primary cursor, or adds a cursor at the next occurrence of the selection (Ctrl+D). */
    public function selectNextOccurrence(): void
    {
        $last = $this->cursors[array_key_last($this->cursors)];
        [$start, $end] = self::ordered($last);
        if ($start === $end) {
            [, $wordStart] = $this->word($start[0], min($start[1] + 1, $this->length($start[0])), -1);
            [, $wordEnd] = $this->word($start[0], $wordStart, 1);
            $this->cursors[array_key_last($this->cursors)] = [$start[0], $wordEnd, $start[0], $wordStart];

            return;
        }
        $needle = $this->range($start, $end);
        if ($needle === '' || str_contains($needle, "\n")) {
            return;
        }
        $count = $this->lineCount();
        for ($step = 0; $step <= $count; ++$step) {
            $line = ($end[0] + $step) % $count;
            $from = $step === 0 ? $end[1] : 0;
            $found = mb_strpos($this->lines[$line], $needle, $from);
            if ($found !== false) {
                $this->cursors[] = [$line, $found + mb_strlen($needle), $line, $found];
                $this->merge();

                return;
            }
        }
    }

    // ---- editing ---------------------------------------------------------------------

    /** Types `$text` at every cursor (replacing selections). */
    public function insert(string $text): void
    {
        $text = str_replace(["\r\n", "\r"], "\n", $text);
        $this->remember($text === "\n" ? 'newline' : (strlen($text) === 1 && $text !== ' ' ? 'type' : 'insert'));
        // With one line per cursor, a multi-line paste goes line by line.
        $pieces = explode("\n", $text);
        $spread = count($this->cursors) > 1 && count($pieces) === count($this->cursors);
        foreach ($this->sortedIndexes(true) as $order => $index) {
            $piece = $spread ? $pieces[count($pieces) - 1 - $order] : $text;
            $this->replace($index, $piece);
        }
    }

    /** Enter: a new line keeping the indentation, one level deeper after an opening bracket or a colon. */
    public function newline(): void
    {
        $this->remember('newline');
        foreach ($this->sortedIndexes(true) as $index) {
            [$start] = self::ordered($this->cursors[$index]);
            $current = $this->lines[$start[0]];
            $indent = substr($current, 0, strlen($current) - strlen(ltrim($current, " \t")));
            $before = rtrim(mb_substr($current, 0, $start[1]));
            if ($before !== '' && in_array(mb_substr($before, -1), ['{', '[', '(', ':'], true)) {
                $indent .= str_contains($indent, "\t") ? "\t" : str_repeat(' ', $this->tabSize);
            }
            $this->replace($index, "\n".$indent);
        }
    }

    /** Backspace (`-1`) or Delete (`1`): the selection, or one character (a whole indent level before the text). */
    public function delete(int $direction, bool $word = false): void
    {
        $this->remember('delete');
        foreach ($this->sortedIndexes(true) as $index) {
            [$line, $column, $anchorLine, $anchorColumn] = $this->cursors[$index];
            if ([$line, $column] === [$anchorLine, $anchorColumn]) {
                if ($word) {
                    [$line, $column] = $this->word($line, $column, $direction);
                } elseif ($direction < 0) {
                    $prefix = mb_substr($this->lines[$line], 0, $column);
                    $spaces = $column > 0 && trim($prefix) === '' && !str_contains($prefix, "\t") ? (($column - 1) % $this->tabSize) + 1 : 1;
                    [$line, $column] = $column >= $spaces ? [$line, $column - $spaces] : ($line > 0 ? [$line - 1, $this->length($line - 1)] : [0, 0]);
                } else {
                    [$line, $column] = $column < $this->length($line) ? [$line, $column + 1] : ($line < $this->lineCount() - 1 ? [$line + 1, 0] : [$line, $column]);
                }
                $this->cursors[$index] = [$line, $column, $anchorLine, $anchorColumn];
            }
            $this->replace($index, '');
        }
    }

    /** Tab and Shift+Tab: indents or outdents the selected lines, or types an indent. */
    public function indent(bool $outdent = false): void
    {
        $unit = str_repeat(' ', $this->tabSize);
        $multiline = array_filter($this->cursors, static fn (array $cursor): bool => $cursor[0] !== $cursor[2]) !== [];
        if (!$outdent && !$multiline) {
            $this->remember('type');
            foreach ($this->sortedIndexes(true) as $index) {
                $column = $this->cursors[$index][1];
                $this->replace($index, str_repeat(' ', $this->tabSize - $column % $this->tabSize));
            }

            return;
        }
        $this->remember('indent');
        foreach ($this->cursors as $index => [$line, $column, $anchorLine, $anchorColumn]) {
            for ($row = min($line, $anchorLine); $row <= max($line, $anchorLine); ++$row) {
                $text = $this->lines[$row];
                if ($outdent) {
                    $remove = str_starts_with($text, "\t") ? 1 : min(strlen($text) - strlen(ltrim($text, ' ')), $this->tabSize);
                    $this->lines[$row] = substr($text, $remove);
                    $shift = -$remove;
                } else {
                    $this->lines[$row] = $unit.$text;
                    $shift = $this->tabSize;
                }
                $this->touched($row);
                if ($row === $line) {
                    $column = max(0, $column + $shift);
                }
                if ($row === $anchorLine) {
                    $anchorColumn = max(0, $anchorColumn + $shift);
                }
            }
            $this->cursors[$index] = [$line, $column, $anchorLine, $anchorColumn];
        }
    }

    /** Ctrl+/: comments the selected lines out, or back in when they all are. */
    public function toggleComment(string $marker = '//'): void
    {
        $this->remember('comment');
        $rows = [];
        foreach ($this->cursors as [$line, , $anchorLine]) {
            for ($row = min($line, $anchorLine); $row <= max($line, $anchorLine); ++$row) {
                $rows[$row] = true;
            }
        }
        $all = true;
        foreach (array_keys($rows) as $row) {
            if (trim($this->lines[$row]) !== '' && !str_starts_with(ltrim($this->lines[$row]), $marker)) {
                $all = false;
            }
        }
        foreach (array_keys($rows) as $row) {
            $text = $this->lines[$row];
            if (trim($text) === '') {
                continue;
            }
            $indent = strlen($text) - strlen(ltrim($text));
            $this->lines[$row] = $all
                ? substr($text, 0, $indent).preg_replace('/^'.preg_quote($marker, '/').' ?/', '', substr($text, $indent))
                : substr($text, 0, $indent).$marker.' '.substr($text, $indent);
            $this->touched($row);
        }
        foreach ($this->cursors as $index => [$line, $column, $anchorLine, $anchorColumn]) {
            $this->cursors[$index] = [$line, min($column, $this->length($line)), $anchorLine, min($anchorColumn, $this->length($anchorLine))];
        }
    }

    public function undo(): bool
    {
        return $this->travel($this->undo, $this->redo);
    }

    public function redo(): bool
    {
        return $this->travel($this->redo, $this->undo);
    }

    // ---- internals -------------------------------------------------------------------

    /** Replaces cursor `$index`'s selection with `$text`, moving the other cursors along. */
    private function replace(int $index, string $text): void
    {
        [$start, $end] = self::ordered($this->cursors[$index]);
        $head = mb_substr($this->lines[$start[0]], 0, $start[1]);
        $tail = mb_substr($this->lines[$end[0]], $end[1]);
        $pieces = explode("\n", $text);
        $pieces[0] = $head.$pieces[0];
        $lastPiece = array_key_last($pieces);
        $caretColumn = mb_strlen($pieces[$lastPiece]);
        $pieces[$lastPiece] .= $tail;
        array_splice($this->lines, $start[0], $end[0] - $start[0] + 1, $pieces);
        $caretLine = $start[0] + $lastPiece;
        $lineShift = $lastPiece - ($end[0] - $start[0]);
        if ($lineShift !== 0 || $end[0] !== $start[0]) {
            $this->edits[] = ['splice', $start[0], $end[0] - $start[0] + 1, count($pieces)];
        } else {
            $this->touched($start[0]);
        }
        $this->cursors[$index] = [$caretLine, $caretColumn, $caretLine, $caretColumn];
        // Cursors after this one move by the lines and characters it added.
        foreach ($this->cursors as $other => $cursor) {
            if ($other === $index) {
                continue;
            }
            foreach ([[0, 1], [2, 3]] as [$l, $c]) {
                if ($cursor[$l] > $end[0] || ($cursor[$l] === $end[0] && $cursor[$c] >= $end[1])) {
                    if ($cursor[$l] === $end[0]) {
                        $cursor[$c] = $caretColumn + ($cursor[$c] - $end[1]);
                    }
                    $cursor[$l] += $lineShift;
                }
            }
            $this->cursors[$other] = $cursor;
        }
    }

    private function touched(int $line): void
    {
        $this->edits[] = ['line', $line];
    }

    /** Saves the state before an edit; consecutive typing joins one undo step. */
    private function remember(string $kind): void
    {
        $now = microtime(true);
        $joins = $kind === 'type' && $this->lastKind === 'type' && $now - $this->lastEdit < 1.0;
        if (!$joins) {
            $this->undo[] = ['lines' => $this->lines, 'cursors' => $this->cursors];
            if (count($this->undo) > 500) {
                array_shift($this->undo);
            }
        }
        $this->redo = [];
        $this->lastKind = $kind;
        $this->lastEdit = $now;
    }

    /**
     * @param list<array{lines: list<string>, cursors: list<array{0: int, 1: int, 2: int, 3: int}>}> $from
     * @param list<array{lines: list<string>, cursors: list<array{0: int, 1: int, 2: int, 3: int}>}> $to
     */
    private function travel(array &$from, array &$to): bool
    {
        $state = array_pop($from);
        if ($state === null) {
            return false;
        }
        $to[] = ['lines' => $this->lines, 'cursors' => $this->cursors];
        $this->lines = $state['lines'];
        $this->cursors = $state['cursors'];
        $this->edits[] = ['all'];
        $this->lastKind = '';

        return true;
    }

    /** @return array{0: int, 1: int} the next word boundary in `$direction` */
    private function word(int $line, int $column, int $direction): array
    {
        $text = mb_str_split($this->lines[$line] ?? '');
        if ($direction < 0 && $column === 0) {
            return $line > 0 ? [$line - 1, $this->length($line - 1)] : [0, 0];
        }
        if ($direction > 0 && $column >= count($text)) {
            return $line < $this->lineCount() - 1 ? [$line + 1, 0] : [$line, $column];
        }
        $isWord = static fn (?string $char): bool => $char !== null && preg_match('/[\p{L}\p{N}_$]/u', $char) === 1;
        if ($direction < 0) {
            while ($column > 0 && trim($text[$column - 1]) === '') {
                --$column;
            }
            $word = $isWord($text[$column - 1] ?? null);
            while ($column > 0 && trim($text[$column - 1]) !== '' && $isWord($text[$column - 1]) === $word) {
                --$column;
            }

            return [$line, $column];
        }
        while ($column < count($text) && trim($text[$column]) === '') {
            ++$column;
        }
        $word = $isWord($text[$column] ?? null);
        while ($column < count($text) && trim($text[$column]) !== '' && $isWord($text[$column]) === $word) {
            ++$column;
        }

        return [$line, $column];
    }

    /** @param array{0: int, 1: int} $start @param array{0: int, 1: int} $end */
    private function range(array $start, array $end): string
    {
        if ($start[0] === $end[0]) {
            return mb_substr($this->lines[$start[0]], $start[1], $end[1] - $start[1]);
        }
        $parts = [mb_substr($this->lines[$start[0]], $start[1])];
        for ($line = $start[0] + 1; $line < $end[0]; ++$line) {
            $parts[] = $this->lines[$line];
        }
        $parts[] = mb_substr($this->lines[$end[0]], 0, $end[1]);

        return implode("\n", $parts);
    }

    /** @return list<int> cursor indexes by position, last first when `$reverse` */
    private function sortedIndexes(bool $reverse = false): array
    {
        $indexes = array_keys($this->cursors);
        usort($indexes, function (int $a, int $b): int {
            [$startA] = self::ordered($this->cursors[$a]);
            [$startB] = self::ordered($this->cursors[$b]);

            return $startA <=> $startB;
        });

        return $reverse ? array_reverse($indexes) : $indexes;
    }

    /** Joins cursors whose selections touch. */
    private function merge(): void
    {
        $kept = [];
        foreach ($this->cursors as $cursor) {
            [$start, $end] = self::ordered($cursor);
            foreach ($kept as $other) {
                [$otherStart, $otherEnd] = self::ordered($other);
                if ($start <= $otherEnd && $otherStart <= $end) {
                    continue 2;
                }
            }
            $kept[] = $cursor;
        }
        $this->cursors = $kept;
    }

    /**
     * @param array{0: int, 1: int, 2: int, 3: int} $cursor
     * @return array{0: array{0: int, 1: int}, 1: array{0: int, 1: int}} start and end
     */
    public static function ordered(array $cursor): array
    {
        $head = [$cursor[0], $cursor[1]];
        $anchor = [$cursor[2], $cursor[3]];

        return $head <= $anchor ? [$head, $anchor] : [$anchor, $head];
    }
}
