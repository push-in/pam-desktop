<?php

declare(strict_types=1);

namespace Pam\Desktop\Code\Lsp;

/** A completion a language server offers at a position. */
final readonly class CompletionItem
{
    public function __construct(
        public string $label,
        public string $insertText,
        public ?string $detail = null,
        /** LSP CompletionItemKind (3 function, 6 variable, 7 class, 14 keyword…). */
        public int $kind = 1,
    ) {
    }

    /** @param array<string, mixed> $raw */
    public static function fromLsp(array $raw): self
    {
        $label = (string) ($raw['label'] ?? '');

        return new self(
            $label,
            (string) ($raw['textEdit']['newText'] ?? $raw['insertText'] ?? $label),
            isset($raw['detail']) ? (string) $raw['detail'] : null,
            (int) ($raw['kind'] ?? 1),
        );
    }
}
