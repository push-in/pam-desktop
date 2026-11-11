<?php

declare(strict_types=1);

namespace Pam\Desktop\Code\Lsp;

/** A problem a language server reports in a file (0-based positions). */
final readonly class Diagnostic
{
    public const int ERROR = 1;
    public const int WARNING = 2;
    public const int INFORMATION = 3;
    public const int HINT = 4;

    public function __construct(
        public string $path,
        public int $line,
        public int $character,
        public int $endLine,
        public int $endCharacter,
        public int $severity,
        public string $message,
        public ?string $source = null,
    ) {
    }

    /** @param array<string, mixed> $raw */
    public static function fromLsp(string $path, array $raw): self
    {
        return new self(
            $path,
            (int) ($raw['range']['start']['line'] ?? 0),
            (int) ($raw['range']['start']['character'] ?? 0),
            (int) ($raw['range']['end']['line'] ?? 0),
            (int) ($raw['range']['end']['character'] ?? 0),
            (int) ($raw['severity'] ?? self::ERROR),
            (string) ($raw['message'] ?? ''),
            isset($raw['source']) ? (string) $raw['source'] : null,
        );
    }
}
