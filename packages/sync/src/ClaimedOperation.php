<?php

declare(strict_types=1);

namespace Pam\Desktop\Sync\Durable;

use UnexpectedValueException;

/** A lease is an opaque ownership token, never a replacement idempotency key. */
final readonly class ClaimedOperation
{
    /** @param array<string, mixed> $payload */
    public function __construct(
        public string $identifier,
        public int $kind,
        public array $payload,
        public int $createdAtMillis,
        public int $attempts,
        public string $lease,
        public int $leaseUntilMillis,
    ) {}

    /** @param array<string, string|int|float|bool|null> $row */
    public static function fromRow(array $row): self
    {
        foreach (['id', 'payload_json', 'lease'] as $key) {
            if (!is_string($row[$key] ?? null)) throw new UnexpectedValueException('Invalid persisted outbox text field.');
        }
        foreach (['operation_kind', 'created_at', 'attempts', 'lease_until'] as $key) {
            if (!is_int($row[$key] ?? null)) throw new UnexpectedValueException('Invalid persisted outbox integer field.');
        }
        $json = $row['payload_json'];
        $id = $row['id'];
        $lease = $row['lease'];
        $kind = $row['operation_kind'];
        $created = $row['created_at'];
        $attempts = $row['attempts'];
        $until = $row['lease_until'];
        if ($kind < 1 || $created < 0 || $attempts < 1 || $until < 1
            || preg_match('/^[a-f0-9]{32}$/D', $lease) !== 1) {
            throw new UnexpectedValueException('Invalid persisted outbox row.');
        }
        $decoded = json_decode($json, true, flags: JSON_THROW_ON_ERROR);
        if (!is_array($decoded)) throw new UnexpectedValueException('Outbox payload must be an object.');
        $payload = [];
        foreach ($decoded as $key => $value) {
            if (!is_string($key)) throw new UnexpectedValueException('Outbox payload keys must be strings.');
            $payload[$key] = $value;
        }
        return new self($id, $kind, $payload, $created, $attempts, $lease, $until);
    }
}
