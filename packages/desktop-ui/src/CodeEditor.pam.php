<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

use Pam\Desktop\Attributes\State;
use Pam\Desktop\Code\CodeTheme;
use Pam\Desktop\Code\Highlighter;
use Pam\Desktop\Component;
use Pam\Desktop\GestureEvent;
use Pam\Desktop\KeyEvent;
use Pam\Desktop\LayoutEvent;
use Pam\Desktop\PressEvent;
use Pam\Desktop\System\ComponentTimer;
use Pam\Desktop\System\Clipboard;
use Pam\Desktop\System\ViewRef;

/**
 * A code editor: syntax highlighting, line numbers, the active line,
 * selections, several cursors, undo and the editing keys of VS Code. Only
 * the lines on screen are built.
 *
 * Text arrives through a native text field the editor keeps focused, so
 * dead keys, compose sequences and input methods (accents, CJK) work as in
 * any field. Everything else (cursors, selections, colours) is drawn by the
 * editor.
 *
 * ```html
 * <CodeEditor :text="$source" language="php" on:change="edited" on:cursor="moved" />
 * ```
 *
 * `change` carries the whole text after each edit; `cursor` carries
 * `['line' => …, 'column' => …, 'cursors' => …]` (1-based, like a status bar).
 */
final class CodeEditor extends Component
{
    #[State] public int $revision = 0;
    #[State] public float $charWidth = 0.0;
    #[State] public int $scrollRevision = 0;
    #[State] public float $scrollTarget = 0.0;
    /** The hidden field's value: what was typed since it was last emptied. */
    #[State] public string $buffer = '';

    public CodeDocument $document;
    public readonly CodeTheme $palette;
    public readonly string $key;
    private ?string $loaded = null;
    /** @var array<int, list<array{0: string, 1: int}>> coloured runs by line */
    private array $runs = [];
    private string $consumed = '';
    private float $scrollOffset = 0.0;
    private float $viewport = 0.0;
    private bool $shift = false;
    private bool $alt = false;
    private ?ComponentTimer $relex = null;
    private ?ComponentTimer $clear = null;

    public function __construct(
        #[State] public string $text,
        public ?string $language = null,
        ?CodeTheme $theme = null,
        public float $fontSize = 13.0,
        public float $lineHeight = 19.0,
        public string $fontFamily = 'monospace',
        public bool $lineNumbers = true,
        #[State] public bool $readOnly = false,
        public int $tabSize = 4,
        public ?string $label = null,
        public bool $autoFocus = false,
    ) {
        $this->palette = $theme ?? CodeTheme::darkModern();
        $this->key = 'code-editor-'.spl_object_id($this);
        $this->document = new CodeDocument($text, $tabSize);
        $this->loaded = $text;
    }

    public function mount(): void
    {
        if ($this->autoFocus) {
            $this->focus();
        }
    }

    public function rendering(): void
    {
        // A new text from the owner (another file, a reload) replaces the document.
        if ($this->text !== $this->loaded && $this->text !== $this->document->text()) {
            $this->document = new CodeDocument($this->text, $this->tabSize);
            $this->runs = [];
        }
        $this->loaded = $this->text;
        $this->applyEdits();
    }

    public function focus(): void
    {
        ViewRef::of($this->key.'-input')->focus();
    }

    /** A hundred wide characters: measuring them gives the column width. */
    public function probe(): string
    {
        return str_repeat('M', 100);
    }

    /** @return list<int> the line indexes, for the list */
    public function rows(): array
    {
        return range(0, $this->document->lineCount() - 1);
    }

    /** @return list<array{0: string, 1: int}> */
    public function runs(int $line): array
    {
        return $this->runs[$line] ??= $this->lexLine($this->document->display($line));
    }

    /**
     * One line coloured on its own, until the whole text is coloured again
     * (a pause in typing). PHP needs its open tag to be read as code.
     *
     * @return list<array{0: string, 1: int}>
     */
    private function lexLine(string $text): array
    {
        if (!in_array($this->language, ['php', 'pam'], true) || str_starts_with(ltrim($text), '<')) {
            return Highlighter::lines($text, $this->language, $this->palette)[0] ?? [];
        }
        $runs = Highlighter::lines('<?php '.$text, 'php', $this->palette)[0] ?? [];
        $skip = 6;
        while ($runs !== [] && $skip > 0) {
            $length = mb_strlen($runs[0][0]);
            if ($length <= $skip) {
                array_shift($runs);
                $skip -= $length;
            } else {
                $runs[0][0] = mb_substr($runs[0][0], $skip);
                $skip = 0;
            }
        }

        return $runs;
    }

    public function gutterWidth(): float
    {
        return $this->lineNumbers ? max(3, strlen((string) $this->document->lineCount())) * $this->charWidth + 34.0 : 8.0;
    }

    public function activeLine(int $line): bool
    {
        return $this->document->cursors[0][0] === $line;
    }

    /** @return list<array{0: float, 1: float}> left and width of the selected parts of a line */
    public function selections(int $line): array
    {
        $parts = [];
        foreach ($this->document->cursors as $cursor) {
            [$start, $end] = CodeDocument::ordered($cursor);
            if ($start === $end || $line < $start[0] || $line > $end[0]) {
                continue;
            }
            $from = $line === $start[0] ? $this->document->visual($line, $start[1]) : 0;
            $to = $line === $end[0] ? $this->document->visual($line, $end[1]) : $this->document->visual($line, $this->document->length($line)) + 1;
            $parts[] = [$from * $this->charWidth, max(1, $to - $from) * $this->charWidth];
        }

        return $parts;
    }

    /** @return list<float> the left of each cursor on a line */
    public function carets(int $line): array
    {
        $carets = [];
        foreach ($this->document->cursors as [$cursorLine, $column]) {
            if ($cursorLine === $line) {
                $carets[] = $this->document->visual($line, $column) * $this->charWidth;
            }
        }

        return $carets;
    }

    // ---- input -------------------------------------------------------------------------

    /** The hidden field changed: what was added is typed at every cursor. */
    public function typed(string $value): void
    {
        $added = str_starts_with($value, $this->consumed) ? mb_substr($value, mb_strlen($this->consumed)) : '';
        $this->consumed = $value;
        $this->buffer = $value;
        if ($added !== '' && !$this->readOnly) {
            $this->document->insert($added);
            $this->edited();
        }
        // Empty the field once typing pauses (never in the middle of a word).
        $this->clear?->cancel();
        $this->clear = $this->timers()->timeout(600, function (): void {
            $this->clear = null;
            $this->consumed = '';
            $this->buffer = '';
        });
    }

    public function key(KeyEvent $event): void
    {
        if ($event->key === 'Shift' || $event->key === 'Alt') {
            $event->key === 'Shift' ? $this->shift = true : $this->alt = true;

            return;
        }
        $document = $this->document;
        $command = $event->control || $event->meta;
        $editable = !$this->readOnly;
        $handled = true;
        $edit = false;
        match (true) {
            $command && $event->key === 'z' && !$event->shift => $edit = $editable && $document->undo(),
            $command && ($event->key === 'y' || ($event->key === 'Z' || ($event->key === 'z' && $event->shift))) => $edit = $editable && $document->redo(),
            $command && $event->key === 'a' => $document->selectAll(),
            $command && $event->key === 'c' => Clipboard::setText($document->selectedText()),
            $command && $event->key === 'x' => $edit = $this->cut(),
            $command && $event->key === 'd' => $document->selectNextOccurrence(),
            $command && $event->key === '/' => $edit = $editable && ($document->toggleComment($this->commentMarker()) ?? true),
            $command && $event->key === 'Home' => $document->move('top', $event->shift),
            $command && $event->key === 'End' => $document->move('bottom', $event->shift),
            $command && $event->key === 'ArrowLeft' => $document->move('wordLeft', $event->shift),
            $command && $event->key === 'ArrowRight' => $document->move('wordRight', $event->shift),
            $command && $event->key === 'Backspace' => $edit = $editable && ($document->delete(-1, true) ?? true),
            $command && $event->key === 'Delete' => $edit = $editable && ($document->delete(1, true) ?? true),
            $command => $handled = false,
            $event->key === 'ArrowLeft' => $document->move('left', $event->shift),
            $event->key === 'ArrowRight' => $document->move('right', $event->shift),
            $event->key === 'ArrowUp' => $document->move('up', $event->shift),
            $event->key === 'ArrowDown' => $document->move('down', $event->shift),
            $event->key === 'Home' => $document->move('home', $event->shift),
            $event->key === 'End' => $document->move('end', $event->shift),
            $event->key === 'PageUp' => $document->move('pageUp', $event->shift, $this->page()),
            $event->key === 'PageDown' => $document->move('pageDown', $event->shift, $this->page()),
            $event->key === 'Escape' => $document->singleCursor(),
            $event->key === 'Enter' => $edit = $editable && ($document->newline() ?? true),
            $event->key === 'Backspace' => $edit = $editable && ($document->delete(-1) ?? true),
            $event->key === 'Delete' => $edit = $editable && ($document->delete(1) ?? true),
            $event->key === 'Tab' => $edit = $editable && ($document->indent($event->shift) ?? true),
            default => $handled = false,
        };
        if (!$handled) {
            return;
        }
        $edit ? $this->edited() : $this->moved();
    }

    public function keyUp(KeyEvent $event): void
    {
        if ($event->key === 'Shift') {
            $this->shift = false;
        } elseif ($event->key === 'Alt') {
            $this->alt = false;
        }
    }

    /** A click on a line: the cursor goes there (Shift extends, Alt adds a cursor). */
    public function press(int $line, PressEvent|string $event): void
    {
        $event = $event instanceof PressEvent ? $event : PressEvent::fromPayload($event);
        $this->focus();
        $column = $this->columnAt($line, $event->x);
        $this->document->setCursor($line, $column, extend: $this->shift, add: $this->alt);
        $this->moved(reveal: false);
    }

    public function dragBegin(GestureEvent $event): void
    {
        $this->focus();
        [$line, $column] = $this->pointAt($event->x, $event->y);
        $this->document->setCursor($line, $column, extend: $this->shift);
        $this->moved(reveal: false);
    }

    public function drag(GestureEvent $event): void
    {
        [$line, $column] = $this->pointAt($event->x, $event->y);
        $this->document->setCursor($line, $column, extend: true);
        $this->moved(reveal: false);
    }

    public function measureChar(LayoutEvent $layout): void
    {
        $width = $layout->width / 100;
        if (abs($width - $this->charWidth) > 0.001) {
            $this->charWidth = $width;
        }
    }

    public function measured(LayoutEvent $layout): void
    {
        $this->viewport = $layout->height;
    }

    public function scrolled(string $offset): void
    {
        $this->scrollOffset = (float) $offset;
    }

    public function unmount(): void
    {
        $this->relex?->cancel();
        $this->clear?->cancel();
    }

    // ---- internals ---------------------------------------------------------------------

    private function cut(): bool
    {
        Clipboard::setText($this->document->selectedText());
        if ($this->readOnly) {
            return false;
        }
        foreach ($this->document->cursors as $index => [$line, $column, $anchorLine, $anchorColumn]) {
            if ([$line, $column] === [$anchorLine, $anchorColumn]) {
                // Nothing selected: Ctrl+X cuts the whole line.
                $last = $line === $this->document->lineCount() - 1;
                $this->document->cursors[$index] = $last && $line > 0
                    ? [$line, $this->document->length($line), $line - 1, $this->document->length($line - 1)]
                    : [$last ? $line : $line + 1, 0, $line, 0];
            }
        }
        $this->document->delete(1);

        return true;
    }

    private function edited(): void
    {
        $this->applyEdits();
        $this->text = $this->loaded = $this->document->text();
        $this->emit('change', $this->text);
        $this->moved();
        // Multi-line constructs (block comments, strings) settle once typing pauses.
        $this->relex?->cancel();
        $this->relex = $this->timers()->timeout(300, function (): void {
            $this->relex = null;
            $this->highlightAll();
            ++$this->revision;
        });
    }

    private function moved(bool $reveal = true): void
    {
        ++$this->revision;
        [$line, $column] = $this->document->cursors[0];
        $this->emit('cursor', ['line' => $line + 1, 'column' => $column + 1, 'cursors' => count($this->document->cursors)]);
        if ($reveal) {
            $this->reveal($line);
        }
    }

    /** Applies the document's edits to the colour cache. */
    private function applyEdits(): void
    {
        foreach ($this->document->takeEdits() as $edit) {
            match ($edit[0]) {
                'all' => $this->highlightAll(),
                'line' => $this->forget((int) $edit[1]),
                'splice' => $this->splice((int) $edit[1], (int) $edit[2], (int) $edit[3]),
                default => null,
            };
        }
    }

    private function forget(int $line): void
    {
        unset($this->runs[$line]);
    }

    private function splice(int $at, int $removed, int $inserted): void
    {
        $runs = [];
        foreach ($this->runs as $line => $value) {
            if ($line < $at) {
                $runs[$line] = $value;
            } elseif ($line >= $at + $removed) {
                $runs[$line - $removed + $inserted] = $value;
            }
        }
        $this->runs = $runs;
    }

    private function highlightAll(): void
    {
        $display = [];
        for ($line = 0, $count = $this->document->lineCount(); $line < $count; ++$line) {
            $display[] = $this->document->display($line);
        }
        $this->runs = Highlighter::lines(implode("\n", $display), $this->language, $this->palette);
    }

    private function commentMarker(): string
    {
        return match ($this->language) {
            'python', 'shell', 'yaml', 'toml', 'ini', 'dockerfile' => '#',
            'sql' => '--',
            default => '//',
        };
    }

    private function page(): int
    {
        return max(1, (int) floor($this->viewport / $this->lineHeight) - 1);
    }

    private function columnAt(int $line, float $x): int
    {
        if ($this->charWidth <= 0.0) {
            return 0;
        }

        return $this->document->column($line, (int) round(($x - $this->gutterWidth() - 4.0) / $this->charWidth));
    }

    /** @return array{0: int, 1: int} the line and column under a point of the list */
    private function pointAt(float $x, float $y): array
    {
        $line = max(0, min($this->document->lineCount() - 1, (int) floor(($y + $this->scrollOffset) / $this->lineHeight)));

        return [$line, $this->columnAt($line, $x)];
    }

    /** Scrolls just enough to show line `$line`. */
    private function reveal(int $line): void
    {
        $top = $line * $this->lineHeight;
        $bottom = $top + $this->lineHeight;
        if ($top < $this->scrollOffset) {
            $this->scrollTarget = $top;
        } elseif ($this->viewport > 0 && $bottom > $this->scrollOffset + $this->viewport) {
            $this->scrollTarget = $bottom - $this->viewport;
        } else {
            return;
        }
        $this->scrollOffset = $this->scrollTarget;
        ++$this->scrollRevision;
    }
}
?>

<template>
    <View class="editor" :backgroundColor="$palette->background" :accessibilityLabel="$label ?? 'Editor'">
        <Text class="probe" :fontFamily="$fontFamily" :fontSize="$fontSize" on:layout="measureChar">{{ probe() }}</Text>
        <Input :nativeRef="$key . '-input'" class="sink" :value="$buffer" on:change="typed" on:keyDown="key" on:keyUp="keyUp"
               captureKeys="ArrowLeft,ArrowRight,ArrowUp,ArrowDown,Home,End,PageUp,PageDown,Enter,Backspace,Delete,Tab,Escape"
               :autoFocus="$autoFocus" accessibilityLabel="Editor input" />
        <GestureDetector class="surface" gestureType="pan" gestureMinDistance="3" on:gestureBegin="dragBegin" on:gestureUpdate="drag">
            <VirtualizedList class="lines" :estimatedRowHeight="$lineHeight" :scrollRequest="$scrollRevision" :scrollTargetOffset="$scrollTarget"
                             on:scroll="scrolled" on:layout="measured">
                <Pressable p-for="$line in rows()" p-key="$line" class="line" :height="$lineHeight" cursor="text"
                           :backgroundColor="activeLine($line) ? $palette->activeLine : $palette->background"
                           on:press="press($line, $event)">
                    <View class="gutter" :width="gutterWidth()">
                        <Text p-if="$lineNumbers" class="number" :fontFamily="$fontFamily" :fontSize="$fontSize" :lineHeight="$lineHeight"
                              :textColor="activeLine($line) ? $palette->activeLineNumber : $palette->lineNumber">{{ $line + 1 }}</Text>
                    </View>
                    <View class="content">
                        <View p-for="$part in selections($line)" p-key="$partIndex" class="selection" :left="$part[0]" :width="$part[1]"
                              :height="$lineHeight" :backgroundColor="$palette->selection" />
                        <Text class="code" :fontFamily="$fontFamily" :fontSize="$fontSize" :lineHeight="$lineHeight" :textColor="$palette->text"
                              numberOfLines="1"><Span p-for="$run in runs($line)" p-key="$runIndex" :textColor="$run[1]">{{ $run[0] }}</Span></Text>
                        <View p-for="$left in carets($line)" p-key="$leftIndex" class="caret" :left="$left" :height="$lineHeight"
                              :backgroundColor="$palette->cursor" />
                    </View>
                </Pressable>
            </VirtualizedList>
        </GestureDetector>
    </View>
</template>

<style scoped>
    .editor { flex: 1; min-height: 0; overflow: hidden; }
    .probe { position: absolute; left: 0; top: 0; opacity: 0; white-space: pre; }
    .sink { position: absolute; left: 0; top: 0; width: 2px; height: 2px; opacity: 0; border-width: 0; padding: 0; }
    .surface { flex: 1; min-height: 0; }
    .lines { flex: 1; scrollbar-width: thin; }
    .line { flex-direction: row; align-items: stretch; }
    .gutter { flex-direction: row; justify-content: flex-end; padding-right: 20px; }
    .number { white-space: pre; }
    .content { flex: 1; min-width: 0; padding-left: 4px; overflow: hidden; }
    .selection { position: absolute; top: 0; margin-left: 4px; }
    .code { white-space: pre; }
    .caret { position: absolute; top: 0; width: 2px; margin-left: 3px; }
</style>
