<?php

declare(strict_types=1);

namespace Pam\Desktop\Sync\Durable;

use InvalidArgumentException;

/** A transport adapter's validated response and local reconciliation projection. */
final readonly class OutboxResolution
{
    /** @param array<string, mixed> $result
     *  @param list<array{sql:string,arguments?:list<string|int|float|bool|null>,argumentSets?:list<list<string|int|float|bool|null>>}> $projection
     */
    private function __construct(
        public OutboxDisposition $disposition,
        public array $result,
        public array $projection,
        public string $message,
    ) {
        if ($disposition === OutboxDisposition::Applied && count($projection) > 256) {
            throw new InvalidArgumentException('A reconciliation projection may contain at most 256 statements.');
        }
        if ($disposition !== OutboxDisposition::Applied && $projection !== []) {
            throw new InvalidArgumentException('Only applied responses may change the local projection.');
        }
        if ($disposition !== OutboxDisposition::Applied && trim($message) === '') {
            throw new InvalidArgumentException('Retry and rejection responses need a diagnostic message.');
        }
    }

    /** @param array<string, mixed> $result
     *  @param list<array{sql:string,arguments?:list<string|int|float|bool|null>,argumentSets?:list<list<string|int|float|bool|null>>}> $projection
     */
    public static function applied(array $result, array $projection = []): self
    {
        return new self(OutboxDisposition::Applied, $result, $projection, '');
    }

    public static function retry(string $message): self
    {
        return new self(OutboxDisposition::Retry, [], [], $message);
    }

    public static function rejected(string $message): self
    {
        return new self(OutboxDisposition::Rejected, [], [], $message);
    }
}
