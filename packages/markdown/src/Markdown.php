<?php

declare(strict_types=1);

namespace Pam\Desktop\Markdown;

use Closure;
use Pam\Desktop\Element;
use Pam\Desktop\EventKind;
use Pam\Desktop\FlexDirection;
use Pam\Desktop\FontStyle;
use Pam\Desktop\ImageFit;
use Pam\Desktop\Style;
use Pam\Desktop\System\Linking;
use Pam\Desktop\TemplateRegistry;
use Pam\Desktop\TextAlignment;
use Pam\Desktop\TextDecoration;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\Icon;
use Pam\Desktop\UI\Image;
use Pam\Desktop\UI\Row;
use Pam\Desktop\UI\Text;
use Pam\Desktop\UI\View;

/**
 * Markdown rendered as native elements: paragraphs and headings are rich
 * text (selectable, with pressable links), lists, quotes, code blocks and
 * tables are boxes. Nothing goes through a web view.
 *
 * ```php
 * Markdown::render("**Deploy** is done, see [the log](https://ci.example.com/42).");
 * ```
 *
 * In templates, after `Markdown::register()`:
 *
 * ```html
 * <Markdown :source="$message->body" theme="dark" mentions="true" />
 * ```
 */
final class Markdown
{
    private static bool $registered = false;

    private function __construct()
    {
    }

    /** Registers the `<Markdown>` tag (`source`, `theme`, `mentions`, `people`, `me`, `flavor`, `@link`). */
    public static function register(): void
    {
        if (self::$registered) {
            return;
        }
        TemplateRegistry::component('Markdown', static function (array $props): Element {
            $theme = $props['theme'] ?? null;
            $link = $props['@link'] ?? $props['onLink'] ?? null;

            return self::render(
                (string) ($props['source'] ?? ''),
                $theme instanceof MarkdownTheme ? $theme : ($theme === 'dark' ? MarkdownTheme::dark() : MarkdownTheme::light()),
                $link instanceof Closure ? $link : null,
                filter_var($props['mentions'] ?? false, FILTER_VALIDATE_BOOL),
                null,
                is_string($props['me'] ?? null) ? $props['me'] : null,
                ($props['flavor'] ?? 'markdown') === 'slack',
                is_array($props['people'] ?? null) ? array_values(array_filter($props['people'], 'is_string')) : [],
            );
        });
        self::$registered = true;
    }

    /**
     * @param (Closure(string): void)|null $onLink a pressed link (the system browser opens it by default)
     * @param (Closure(string, string): list<array{0: string, 1: int|string}>)|null $highlight
     *        a code block's text and language to coloured runs (see `Pam\Desktop\Code\Highlighter`)
     * @param string|null $me the reader's name: `@me` mentions stand out
     * @param bool $slack Slack's mrkdwn emphasis: `*bold*`, `_italic_`, `~strike~`
     * @param list<string> $people names mentions may hold spaces in (`@Ana Souza`)
     */
    public static function render(
        string $source,
        ?MarkdownTheme $theme = null,
        ?Closure $onLink = null,
        bool $mentions = false,
        ?Closure $highlight = null,
        ?string $me = null,
        bool $slack = false,
        array $people = [],
    ): Element {
        $renderer = new self();
        $renderer->theme = $theme ?? MarkdownTheme::light();
        $renderer->onLink = $onLink;
        $renderer->highlight = $highlight;
        $renderer->me = $me === null ? null : '@'.ltrim($me, '@');

        return $renderer->blocks((new Parser($mentions || $people !== [], $slack, $people))->blocks($source));
    }

    /**
     * @param list<string> $people
     * @return list<array<int, mixed>> the block tree (tests, custom renderers)
     */
    public static function parse(string $source, bool $mentions = false, bool $slack = false, array $people = []): array
    {
        return (new Parser($mentions || $people !== [], $slack, $people))->blocks($source);
    }

    /** Forgets the tag registration (tests). */
    public static function reset(): void
    {
        self::$registered = false;
    }

    private MarkdownTheme $theme;
    private ?Closure $onLink = null;
    private ?Closure $highlight = null;
    private ?string $me = null;

    /** @param list<array<int, mixed>> $blocks */
    private function blocks(array $blocks, ?int $color = null): Element
    {
        return Column::make(...array_map(fn (array $block): Element => $this->block($block, $color), $blocks))
            ->style(new Style(gap: $this->theme->blockGap));
    }

    /** @param array<int, mixed> $block */
    private function block(array $block, ?int $color): Element
    {
        $theme = $this->theme;

        return match ($block[0]) {
            'heading' => $this->text($block[2], $color, $theme->headings[$block[1] - 1], 700)
                ->style(new Style(marginTop: $block[1] <= 2 ? 6.0 : 2.0, lineHeight: $theme->headings[$block[1] - 1] * 1.3)),
            'paragraph' => $this->text($block[1], $color),
            'code' => $this->code($block[2], $block[1]),
            'quote' => Row::make(
                View::make()->style(new Style(width: 3.0, borderRadius: 2.0, backgroundColor: $theme->quoteBar)),
                $this->blocks($block[1], $theme->muted)->style(new Style(flexGrow: 1.0, flexShrink: 1.0)),
            )->style(new Style(gap: 10.0)),
            'list' => $this->list($block[1], $block[2], $block[3], $color),
            'hr' => View::make()->style(new Style(height: 1.0, backgroundColor: $theme->border, marginVertical: 4.0)),
            'table' => $this->table($block[1], $block[2], $block[3], $color),
            'image' => Image::make($block[2])
                ->accessibilityLabel($block[1])
                ->style(new Style(maxWidth: 480.0, height: 240.0, borderRadius: 6.0, imageFit: ImageFit::Contain)),
            default => View::make(),
        };
    }

    /** @param list<array<string, mixed>> $runs */
    private function text(array $runs, ?int $color, ?float $size = null, ?int $weight = null): Text
    {
        $theme = $this->theme;
        $parts = [];
        foreach ($runs as $run) {
            $parts[] = $this->run($run);
        }

        return Text::rich(...$parts)
            ->selectable()
            ->style(new Style(
                fontSize: $size ?? $theme->fontSize,
                lineHeight: $size === null ? $theme->lineHeight : null,
                textColor: $color ?? $theme->text,
                fontWeight: $weight,
                fontFamily: $theme->fontFamily,
            ));
    }

    /** @param array<string, mixed> $run */
    private function run(array $run): Text
    {
        $theme = $this->theme;
        $link = $run['link'];
        $decoration = match (true) {
            $run['strike'] && $link !== null => TextDecoration::UnderlineLineThrough,
            $run['strike'] => TextDecoration::LineThrough,
            default => null,
        };
        $self = $run['mention'] && $this->me !== null && strcasecmp($run['text'], $this->me) === 0;
        $text = Text::make($run['code'] ? "\u{2009}{$run['text']}\u{2009}" : $run['text'])->style(new Style(
            textColor: match (true) {
                $link !== null => $theme->link,
                $self => $theme->mentionSelf,
                $run['mention'] => $theme->mention,
                $run['code'] => $theme->codeText,
                default => null,
            },
            backgroundColor: match (true) {
                $run['code'] => $theme->codeBackground,
                $self => $theme->mentionSelfBackground,
                $run['mention'] => $theme->mentionBackground,
                default => null,
            },
            fontWeight: $run['bold'] || $run['mention'] ? 700 : null,
            fontStyle: $run['italic'] ? FontStyle::Italic : null,
            textDecoration: $decoration,
            fontFamily: $run['code'] ? $theme->monospace : null,
            fontSize: $run['code'] ? $theme->fontSize * 0.87 : null,
        ));
        if ($link !== null && $link !== '') {
            $onLink = $this->onLink;
            $text = $text->on(EventKind::Press, static function () use ($link, $onLink): void {
                $onLink !== null ? $onLink($link) : Linking::open($link);
            });
        }

        return $text;
    }

    private function code(string $source, string $language): Element
    {
        $theme = $this->theme;
        $runs = $this->highlight !== null && $language !== '' ? ($this->highlight)($source, $language) : [[$source, $theme->codeBlockText]];
        $parts = [];
        foreach ($runs as [$piece, $color]) {
            $parts[] = Text::make($piece)->style(new Style(textColor: is_int($color) ? $color : self::hex($color)));
        }

        return View::make(
            Text::rich(...$parts)->selectable()->style(new Style(
                fontFamily: $theme->monospace,
                fontSize: $theme->fontSize * 0.87,
                lineHeight: $theme->fontSize * 1.35,
                textColor: $theme->codeBlockText,
            )),
        )->style(new Style(
            padding: 10.0,
            borderRadius: 6.0,
            borderWidth: 1.0,
            borderColor: $theme->border,
            backgroundColor: $theme->codeBlockBackground,
        ));
    }

    /** @param list<array{task: ?bool, blocks: list<array<int, mixed>>}> $items */
    private function list(bool $ordered, int $start, array $items, ?int $color): Element
    {
        $theme = $this->theme;
        $rows = [];
        foreach ($items as $index => $item) {
            $marker = match (true) {
                $item['task'] !== null => Icon::make($item['task'] ? 'square-check' : 'square', 'Lucide', $theme->fontSize, $item['task'] ? $theme->link : $theme->muted),
                $ordered => Text::make(($start + $index).'.')->style(new Style(fontSize: $theme->fontSize, lineHeight: $theme->lineHeight, textColor: $color ?? $theme->text, textAlign: TextAlignment::End)),
                default => Text::make('•')->style(new Style(fontSize: $theme->fontSize, lineHeight: $theme->lineHeight, textColor: $color ?? $theme->text, textAlign: TextAlignment::Center)),
            };
            $rows[] = Row::make(
                View::make($marker)->style(new Style(width: $ordered ? 24.0 : 18.0, paddingTop: $item['task'] !== null ? 3.0 : 0.0)),
                $this->blocks($item['blocks'], $item['task'] === true ? $theme->muted : $color)
                    ->style(new Style(flexGrow: 1.0, flexShrink: 1.0, gap: 4.0)),
            )->style(new Style(gap: 4.0));
        }

        return Column::make(...$rows)->style(new Style(gap: 2.0));
    }

    /**
     * @param list<string> $aligns
     * @param list<list<array<string, mixed>>> $header
     * @param list<list<list<array<string, mixed>>>> $rows
     */
    private function table(array $aligns, array $header, array $rows, ?int $color): Element
    {
        $theme = $this->theme;
        $line = function (array $cells, bool $head) use ($aligns, $theme, $color): Element {
            $views = [];
            foreach ($cells as $index => $cell) {
                $align = match ($aligns[$index] ?? 'left') {
                    'center' => TextAlignment::Center,
                    'right' => TextAlignment::End,
                    default => TextAlignment::Start,
                };
                $views[] = View::make($this->text($cell, $color, null, $head ? 700 : null)->style(new Style(textAlign: $align)))
                    ->style(new Style(flexGrow: 1.0, flexShrink: 1.0, paddingHorizontal: 10.0, paddingVertical: 6.0,
                        borderRightWidth: $index < count($cells) - 1 ? 1.0 : 0.0, borderColor: $theme->border));
            }

            return Row::make(...$views)->style(new Style(
                flexDirection: FlexDirection::Row,
                backgroundColor: $head ? $theme->tableHeader : null,
                borderBottomWidth: 1.0,
                borderColor: $theme->border,
            ));
        };

        return Column::make($line($header, true), ...array_map(static fn (array $row): Element => $line($row, false), $rows))
            ->style(new Style(borderWidth: 1.0, borderColor: $theme->border, borderRadius: 6.0, overflow: \Pam\Desktop\Overflow::Hidden));
    }

    private static function hex(string $color): int
    {
        $hex = ltrim($color, '#');
        if (strlen($hex) === 6) {
            $hex = 'FF'.$hex;
        } elseif (strlen($hex) === 8) {
            $hex = substr($hex, 6, 2).substr($hex, 0, 6); // #RRGGBBAA → AARRGGBB
        }

        return (int) hexdec($hex);
    }
}
