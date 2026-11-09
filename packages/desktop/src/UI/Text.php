<?php

declare(strict_types=1);

namespace Pam\Desktop\UI;

use InvalidArgumentException;
use Pam\Desktop\Element;
use Pam\Desktop\EventKind;
use Pam\Desktop\Internal\BinaryValue;
use Pam\Desktop\NodeKind;
use Pam\Desktop\PropKey;
use Pam\Desktop\TextBreakStrategy;
use Pam\Desktop\TextDataDetectorType;
use Pam\Desktop\TextEllipsizeMode;
use Pam\Desktop\TextHyphenationFrequency;

use function array_map;
use function count;
use function explode;
use function implode;
use function is_bool;
use function is_float;
use function is_int;
use function is_scalar;
use function is_string;
use function sprintf;
use function trim;

final class Text extends Element
{
    /** Style properties a nested Text/Span may override, in wire field order. */
    private const SPAN_FIELDS = [
        PropKey::FontSize->value => 2,
        PropKey::FontWeight->value => 3,
        PropKey::FontStyle->value => 4,
        PropKey::TextColor->value => 5,
        PropKey::BackgroundColor->value => 6,
        PropKey::TextDecoration->value => 7,
        PropKey::LetterSpacing->value => 8,
        PropKey::FontFamily->value => 9,
        PropKey::TextTransform->value => 11,
    ];

    private const MAX_SPANS = 4_096;

    /** Side keys of an inline box, top/right/bottom/left, after the shorthands. */
    private const BOX_PADDING = [
        [PropKey::PaddingTop, PropKey::PaddingVertical],
        [PropKey::PaddingRight, PropKey::PaddingHorizontal],
        [PropKey::PaddingBottom, PropKey::PaddingVertical],
        [PropKey::PaddingLeft, PropKey::PaddingHorizontal],
    ];

    private const BOX_BORDER = [
        PropKey::BorderTopWidth,
        PropKey::BorderRightWidth,
        PropKey::BorderBottomWidth,
        PropKey::BorderLeftWidth,
    ];

    /** @var list<string|Text>|null */
    private ?array $richParts = null;

    public static function make(string $text): self
    {
        return (new self(NodeKind::Text))->withProperty(PropKey::Text, $text);
    }

    /**
     * Shows the position of the player whose `nativeRef` is [ref], updated
     * natively while it plays (no PHP tick, nothing while paused or covered).
     * The text's own content is the placeholder until the player is ready;
     * give it a stable width (e.g. tabular digits) so updates never relayout.
     */
    public function mediaClock(
        string $ref,
        \Pam\Desktop\MediaClockFormat $format = \Pam\Desktop\MediaClockFormat::Elapsed,
    ): self {
        return $this
            ->withProperty(PropKey::MediaClock, \Pam\Desktop\Animation\Drag::ref($ref))
            ->withProperty(PropKey::MediaClockFormat, $format->value);
    }

    /**
     * Inline rich text: one paragraph whose runs may change weight, size,
     * color, decoration, background or font and may be individually
     * pressable (links, mentions), like nested `<Text>` in React Native.
     * Nested runs inherit from their ancestors; the outer node keeps all
     * paragraph-level styling (line height, alignment, number of lines).
     */
    public static function rich(string|self ...$parts): self
    {
        $text = '';
        $length = 0;
        /** @var list<array{int, int, array<int|string, mixed>}|null> $spans */
        $spans = [];
        /** @var list<\Closure> $handlers */
        $handlers = [];
        /** @var list<string> $boxes */
        $boxes = [];
        foreach ($parts as $part) {
            self::appendRun($part, [], $text, $length, $spans, $handlers, $boxes);
        }
        $records = [];
        foreach ($spans as $span) {
            if ($span === null) {
                continue;
            }
            [$start, $end, $fields] = $span;
            $row = array_fill(0, 12, '');
            $row[0] = (string) $start;
            $row[1] = (string) $end;
            foreach ($fields as $index => $value) {
                $row[$index] = self::spanField($value, (int) $index);
            }
            $records[] = rtrim(implode(',', $row), ',');
        }
        $element = self::make($text);
        $element->richParts = array_values($parts);
        if ($records !== []) {
            $element = $element->withProperty(PropKey::TextSpans, implode(';', $records));
        }
        if ($boxes !== []) {
            $element = $element->withProperty(PropKey::TextSpanBoxes, implode(';', $boxes));
        }
        if ($handlers !== []) {
            $element = $element->on(EventKind::SpanPress, static function (mixed $payload = null) use ($handlers): void {
                $handler = $handlers[(int) (is_scalar($payload) ? $payload : -1)] ?? null;
                $handler?->__invoke('');
            });
        }

        return $element;
    }

    /**
     * Appends one rich text run (a named method instead of a recursive
     * closure, which would be a reference cycle left to the cycle collector).
     *
     * @param array<int|string, mixed> $inherited
     * @param array<int, array{int, int, array<int|string, mixed>}|null> $spans
     * @param list<\Closure> $handlers
     * @param list<string> $boxes
     */
    private static function appendRun(
        string|self $part,
        array $inherited,
        string &$text,
        int &$length,
        array &$spans,
        array &$handlers,
        array &$boxes,
    ): void {
        if (is_string($part)) {
            $text .= $part;
            $length += (int) preg_match_all('/./su', $part);
            return;
        }
        if ($part->kind() !== NodeKind::Text) {
            throw new InvalidArgumentException('Rich text can only contain strings and Text/Span runs.');
        }
        $merged = $inherited;
        foreach ($part->properties() as $key => $value) {
            if (isset(self::SPAN_FIELDS[$key]) && !$value instanceof BinaryValue) {
                $merged[self::SPAN_FIELDS[$key]] = $value;
            }
        }
        $box = self::inlineBox($part->properties());
        if ($box !== null) {
            // The box paints its own background under its padding too; like
            // CSS, the runs inside do not repeat it.
            if (isset($part->properties()[PropKey::BackgroundColor->value])) {
                unset($merged[self::SPAN_FIELDS[PropKey::BackgroundColor->value]]);
            }
            $boxSlot = count($boxes);
            $boxes[] = '';
        }
        $press = $part->events()[EventKind::Press->value] ?? null;
        if ($press !== null) {
            $merged[10] = count($handlers);
            $handlers[] = $press;
        }
        $start = $length;
        $slot = count($spans);
        $spans[] = null;
        if ($part->richParts !== null) {
            foreach ($part->richParts as $child) {
                self::appendRun($child, $merged, $text, $length, $spans, $handlers, $boxes);
            }
        } else {
            $content = $part->properties()[PropKey::Text->value] ?? '';
            self::appendRun(is_string($content) ? $content : '', $merged, $text, $length, $spans, $handlers, $boxes);
        }
        if ($box !== null) {
            if ($length > $start) {
                $boxes[$boxSlot] = rtrim($start.','.$length.','.$box, ',');
            } else {
                array_splice($boxes, $boxSlot, 1);
            }
        }
        if ($merged !== [] && $length > $start) {
            $spans[$slot] = [$start, $length, $merged];
        }
        if (count($spans) > self::MAX_SPANS) {
            throw new InvalidArgumentException('Rich text supports at most 4096 runs.');
        }
    }

    /**
     * The CSS inline box of a nested run (`<span>` with padding, border,
     * radius or background): its record fields after `start,end`, or null
     * when the run draws no box.
     *
     * @param array<int, mixed> $properties
     */
    private static function inlineBox(array $properties): ?string
    {
        $length = static function (mixed $value): ?float {
            if (is_int($value) || is_float($value)) {
                return is_finite((float) $value) ? max(0.0, (float) $value) : null;
            }
            return is_string($value) && preg_match('/^\s*(\d+(?:\.\d+)?)(?:px)?\s*$/D', $value, $match) === 1
                ? (float) $match[1]
                : null;
        };
        // `padding` may be a number or a CSS shorthand of one to four lengths.
        $all = $properties[PropKey::Padding->value] ?? null;
        $shorthand = is_string($all) ? preg_split('/\s+/', trim($all)) : [$all];
        $shorthand = $shorthand === false ? [] : $shorthand;
        $sides = match (count($shorthand)) {
            1 => [$shorthand[0], $shorthand[0], $shorthand[0], $shorthand[0]],
            2 => [$shorthand[0], $shorthand[1], $shorthand[0], $shorthand[1]],
            3 => [$shorthand[0], $shorthand[1], $shorthand[2], $shorthand[1]],
            default => [$shorthand[0] ?? null, $shorthand[1] ?? null, $shorthand[2] ?? null, $shorthand[3] ?? null],
        };
        $padding = [];
        $border = [];
        foreach (self::BOX_PADDING as $side => [$own, $axis]) {
            $padding[] = $length($properties[$own->value] ?? $properties[$axis->value] ?? $sides[$side]) ?? 0.0;
            $border[] = $length($properties[self::BOX_BORDER[$side]->value] ?? $properties[PropKey::BorderWidth->value] ?? null) ?? 0.0;
        }
        $radius = $length($properties[PropKey::BorderRadius->value] ?? null) ?? 0.0;
        $background = $properties[PropKey::BackgroundColor->value] ?? null;
        $background = is_int($background) ? $background : null;
        $inlineBlock = ($properties[PropKey::TextInlineBlock->value] ?? false) === true;
        if (!$inlineBlock && max([...$padding, ...$border]) <= 0.0 && ($radius <= 0.0 || $background === null)) {
            return null;
        }
        $color = $properties[PropKey::BorderColor->value] ?? $properties[PropKey::BorderTopColor->value] ?? null;
        $fields = [
            ...$padding,
            ...$border,
            is_int($color) ? $color : '',
            $radius,
            $background ?? '',
            $length($properties[PropKey::LineHeight->value] ?? null) ?? '',
            $inlineBlock ? 1 : '',
        ];

        return implode(',', array_map(
            static fn (mixed $value): string => $value === '' ? '' : self::spanField($value, 0),
            $fields,
        ));
    }

    private static function spanField(mixed $value, int $index): string
    {
        if ($index === 9) {
            // A CSS family list travels with `|` between its families (the
            // record separates fields with commas); quotes only group names.
            $family = is_string($value) ? implode('|', array_map(
                static fn (string $entry): string => trim($entry, " \t\"'"),
                explode(',', $value),
            )) : '';
            return preg_match('/^[A-Za-z0-9 _.\/:|-]{1,256}$/D', $family) === 1 ? $family : '';
        }
        if (is_bool($value)) {
            return $value ? '1' : '0';
        }
        if (is_int($value)) {
            return (string) $value;
        }
        if (is_float($value) && is_finite($value)) {
            $formatted = rtrim(rtrim(sprintf('%.4F', $value), '0'), '.');
            return $formatted === '-0' ? '0' : $formatted;
        }
        if (is_string($value) && is_numeric($value)) {
            return self::spanField((float) $value, $index);
        }

        return '';
    }

    public function includeFontPadding(bool $include = true): self
    {
        return $this->withProperty(PropKey::IncludeFontPadding, $include);
    }

    public function numberOfLines(int $lines): self
    {
        return $this->withProperty(PropKey::NumberOfLines, max(0, $lines));
    }

    public function selectable(bool $selectable = true): self
    {
        return $this->withProperty(PropKey::TextSelectable, $selectable);
    }

    public function selectionColor(int $color): self
    {
        return $this->withProperty(PropKey::SelectionColor, $color);
    }

    public function ellipsize(TextEllipsizeMode $mode): self
    {
        return $this->withProperty(PropKey::TextEllipsizeMode, $mode->value);
    }

    public function allowFontScaling(bool $allow = true): self
    {
        return $this->withProperty(PropKey::TextAllowFontScaling, $allow);
    }

    public function maxFontSizeMultiplier(float $multiplier): self
    {
        return $this->withProperty(
            PropKey::TextMaxFontSizeMultiplier,
            $multiplier <= 0.0 ? 0.0 : max(1.0, $multiplier),
        );
    }

    public function adjustsFontSizeToFit(
        bool $adjust = true,
        float $minimumScale = 0.01,
    ): self {
        return $this
            ->withProperty(PropKey::TextAdjustsFontSizeToFit, $adjust)
            ->withProperty(
                PropKey::TextMinimumFontScale,
                min(1.0, max(0.01, $minimumScale)),
            );
    }

    public function breakStrategy(TextBreakStrategy $strategy): self
    {
        return $this->withProperty(PropKey::TextBreakStrategy, $strategy->value);
    }

    public function hyphenation(TextHyphenationFrequency $frequency): self
    {
        return $this->withProperty(
            PropKey::TextHyphenationFrequency,
            $frequency->value,
        );
    }

    public function dataDetector(TextDataDetectorType $type): self
    {
        return $this->withProperty(PropKey::TextDataDetectorType, $type->value);
    }
}
