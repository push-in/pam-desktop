<?php

declare(strict_types=1);

namespace Pam\Desktop\Code;

/** The colour (ARGB) of each token type, plus the editor's own colours. */
final readonly class CodeTheme
{
    /** @param array<string, int> $tokens token type value => colour */
    public function __construct(
        public array $tokens,
        public int $background,
        public int $text,
        public int $lineNumber,
        public int $activeLineNumber,
        public int $activeLine,
        public int $selection,
        public int $cursor,
        public int $guide,
    ) {
    }

    public function color(TokenType $type): int
    {
        return $this->tokens[$type->value] ?? $this->text;
    }

    /** VS Code's Dark Modern. */
    public static function darkModern(): self
    {
        return new self([
            'text' => 0xFFCCCCCC, 'comment' => 0xFF6A9955, 'string' => 0xFFCE9178, 'number' => 0xFFB5CEA8,
            'keyword' => 0xFF569CD6, 'control' => 0xFFC586C0, 'type' => 0xFF4EC9B0, 'function' => 0xFFDCDCAA,
            'variable' => 0xFF9CDCFE, 'constant' => 0xFF4FC1FF, 'operator' => 0xFFD4D4D4, 'punctuation' => 0xFF808080,
            'tag' => 0xFF569CD6, 'attribute' => 0xFF9CDCFE, 'property' => 0xFF9CDCFE, 'heading' => 0xFF569CD6,
            'emphasis' => 0xFFCE9178, 'link' => 0xFF3794FF, 'inserted' => 0xFF81B88B, 'deleted' => 0xFFC74E39,
        ], 0xFF1F1F1F, 0xFFCCCCCC, 0xFF6E7681, 0xFFCCCCCC, 0xFF282828, 0x80264F78, 0xFFAEAFAD, 0xFF404040);
    }

    /** VS Code's Light Modern. */
    public static function lightModern(): self
    {
        return new self([
            'text' => 0xFF3B3B3B, 'comment' => 0xFF008000, 'string' => 0xFFA31515, 'number' => 0xFF098658,
            'keyword' => 0xFF0000FF, 'control' => 0xFFAF00DB, 'type' => 0xFF267F99, 'function' => 0xFF795E26,
            'variable' => 0xFF001080, 'constant' => 0xFF0070C1, 'operator' => 0xFF000000, 'punctuation' => 0xFF3B3B3B,
            'tag' => 0xFF800000, 'attribute' => 0xFFE50000, 'property' => 0xFF001080, 'heading' => 0xFF800000,
            'emphasis' => 0xFFA31515, 'link' => 0xFF006AB1, 'inserted' => 0xFF098658, 'deleted' => 0xFFA31515,
        ], 0xFFFFFFFF, 0xFF3B3B3B, 0xFF6E7681, 0xFF171184, 0xFFF3F3F3, 0x66ADD6FF, 0xFF000000, 0xFFD3D3D3);
    }

    /** `#RRGGBB` of a token type, for APIs that take CSS colours. */
    public function hex(TokenType $type): string
    {
        return sprintf('#%06X', $this->color($type) & 0xFFFFFF);
    }
}
