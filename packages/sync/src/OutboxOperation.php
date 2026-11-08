<?php

declare(strict_types=1);

namespace Pam\Desktop\Sync\Durable;

use BackedEnum;
use InvalidArgumentException;

final readonly class OutboxOperation
{
    public const int MAX_PAYLOAD_BYTES = 65_536;
    public int $kind;
    public string $payloadJson;

    /** @param array<string, mixed> $payload */
    public function __construct(
        public string $identifier,
        BackedEnum $kind,
        array $payload,
        public int $createdAtMillis,
    ) {
        if (preg_match('/^[\x21-\x7E]{1,128}$/D', $identifier) !== 1
            || !is_int($kind->value) || $kind->value < 1 || $createdAtMillis < 0) {
            throw new InvalidArgumentException('Outbox identifiers, integer operation enums and timestamps must be valid.');
        }
        $this->kind = $kind->value;
        // Detect recursive arrays and unsupported JSON values before walking
        // the payload to normalize object key order.
        json_encode($payload, JSON_THROW_ON_ERROR);
        $this->payloadJson = json_encode(self::canonical($payload), JSON_THROW_ON_ERROR | JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES);
        if (strlen($this->payloadJson) > self::MAX_PAYLOAD_BYTES) {
            throw new InvalidArgumentException('Outbox operation payload exceeds 64 KiB.');
        }
    }

    /**
     * @param array<array-key, mixed> $value
     * @return array<array-key, mixed>
     */
    private static function canonical(array $value): array
    {
        if (!array_is_list($value)) ksort($value);
        foreach ($value as &$entry) {
            if (is_array($entry)) $entry = self::canonical($entry);
            elseif (is_object($entry) || is_resource($entry)) {
                throw new InvalidArgumentException('Outbox payloads contain only JSON scalars and arrays.');
            }
        }
        unset($entry);
        return $value;
    }
}
