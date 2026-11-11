<?php

declare(strict_types=1);

namespace Pam\Desktop\Markdown;

/** Colours (ARGB) and sizes of rendered Markdown. */
final readonly class MarkdownTheme
{
    public function __construct(
        public float $fontSize = 15.0,
        public float $lineHeight = 22.0,
        public int $text = 0xFF1D1C1D,
        public int $muted = 0xFF616061,
        public int $link = 0xFF1264A3,
        public int $codeText = 0xFFC01343,
        public int $codeBackground = 0x0F1D1C1D,
        public int $codeBlockText = 0xFF1D1C1D,
        public int $codeBlockBackground = 0xFFF8F8F8,
        public int $border = 0x261D1C1D,
        public int $quoteBar = 0xFFDDDDDD,
        public int $mention = 0xFF1264A3,
        public int $mentionBackground = 0x1A1D9BD1,
        /** A mention of the reader (`me`). */
        public int $mentionSelf = 0xFF1D1C1D,
        public int $mentionSelfBackground = 0x66F2C744,
        public int $tableHeader = 0xFFF8F8F8,
        public string $monospace = 'monospace',
        public ?string $fontFamily = null,
        public float $blockGap = 8.0,
        /** @var list<float> font sizes of headings 1 to 6 */
        public array $headings = [26.0, 22.0, 18.0, 16.0, 15.0, 14.0],
    ) {
    }

    public static function light(): self
    {
        return new self();
    }

    public static function dark(): self
    {
        return new self(
            text: 0xFFD1D2D3,
            muted: 0xFFABABAD,
            link: 0xFF1D9BD1,
            codeText: 0xFFE8912D,
            codeBackground: 0x1AFFFFFF,
            codeBlockText: 0xFFD1D2D3,
            codeBlockBackground: 0xFF222529,
            border: 0x33FFFFFF,
            quoteBar: 0xFF4F5155,
            mention: 0xFF1D9BD1,
            mentionBackground: 0x331D9BD1,
            mentionSelf: 0xFFF2C744,
            mentionSelfBackground: 0x33F2C744,
            tableHeader: 0xFF222529,
        );
    }
}
