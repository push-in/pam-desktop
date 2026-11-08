<?php

declare(strict_types=1);

namespace Pam\Desktop\Sync\Durable;

use Closure;
use InvalidArgumentException;
use Pam\Desktop\Database\SQLite;
use Throwable;

/**
 * No snapshots and no eviction: every intent lives in SQLite until explicitly
 * settled. Callers own transport/lifecycle; this store starts no timer or poll.
 * Database names must identify an immutable account/store scope.
 */
final readonly class SQLiteOutbox
{
    // Four maximum-sized JSON payloads stay comfortably below the bridge's
    // 1 MiB limit, including JSON string escaping and row metadata.
    public const int MAX_BATCH = 4;

    private string $table;

    public function __construct(public string $database, public string $namespace = 'default')
    {
        if (preg_match('/^[a-z][a-z0-9_]{0,31}$/D', $namespace) !== 1) {
            throw new InvalidArgumentException('Outbox namespace must be a safe identifier.');
        }
        $this->table = 'pam_durable_outbox'.($namespace === 'default' ? '' : '_'.$namespace);
        if (preg_match('/^[A-Za-z0-9_.-]{1,128}$/D', $database) !== 1) {
            throw new InvalidArgumentException('Outbox database must be a safe scoped name.');
        }
    }

    /** Validated SQL identifier for application projections in the same SQLite transaction. */
    public function tableName(): string { return $this->table; }

    /** @param Closure(): void $complete @param Closure(Throwable): void $failed */
    public function initialize(Closure $complete, Closure $failed): void
    {
        SQLite::transaction($this->database, [
            ['sql' => 'CREATE TABLE IF NOT EXISTS ' . $this->table . ' (
                id TEXT PRIMARY KEY, operation_kind INTEGER NOT NULL CHECK(operation_kind > 0),
                payload_json TEXT NOT NULL, created_at INTEGER NOT NULL,
                operation_state INTEGER NOT NULL CHECK(operation_state BETWEEN 1 AND 4),
                attempts INTEGER NOT NULL DEFAULT 0, available_at INTEGER NOT NULL,
                lease TEXT, lease_until INTEGER NOT NULL DEFAULT 0,
                last_error TEXT NOT NULL DEFAULT \'\', result_json TEXT
            )'],
            ['sql' => 'CREATE INDEX IF NOT EXISTS ' . $this->table . '_ready ON ' . $this->table . '(operation_state, available_at, created_at, id)'],
            ['sql' => 'CREATE INDEX IF NOT EXISTS ' . $this->table . '_lease ON ' . $this->table . '(lease)'],
        ], $complete, $failed);
    }

    /**
     * Projection SQL and intent commit together. A repeated id with identical
     * content acknowledges the original commit without running projection SQL
     * again. Reusing an id for different content is an error.
     *
     * @param list<array{sql:string, arguments?:list<string|int|float|bool|null>, argumentSets?:list<list<string|int|float|bool|null>>}> $projection
     * @param Closure(): void $complete
     * @param Closure(Throwable): void $failed
     */
    public function commit(OutboxOperation $operation, array $projection, Closure $complete, Closure $failed): void
    {
        if (count($projection) > 256) throw new InvalidArgumentException('A projection may contain at most 256 statements.');
        SQLite::transaction($this->database, [
            ['sql' => 'INSERT INTO ' . $this->table . '(id, operation_kind, payload_json, created_at, operation_state, available_at) VALUES (?, ?, ?, ?, ?, ?)',
                'arguments' => [$operation->identifier, $operation->kind, $operation->payloadJson, $operation->createdAtMillis, OutboxState::Pending->value, $operation->createdAtMillis]],
            ...$projection,
        ], $complete, function (Throwable $original) use ($operation, $complete, $failed): void {
            SQLite::query($this->database, 'SELECT operation_kind, payload_json FROM ' . $this->table . ' WHERE id = ?', [$operation->identifier],
                static function (array $rows) use ($operation, $complete, $failed, $original): void {
                    if ($rows === []) { $failed($original); return; }
                    if ($rows[0]['operation_kind'] !== $operation->kind || $rows[0]['payload_json'] !== $operation->payloadJson) {
                        $failed(new InvalidArgumentException('An outbox idempotency key cannot be reused for different content.'));
                        return;
                    }
                    $complete();
                }, $failed);
        });
    }

    /**
     * A dependency group and its local entities commit together. Retrying the
     * identical complete group is idempotent; partial/key-conflicting groups fail.
     * @param list<OutboxOperation> $operations
     * @param list<array{sql:string,arguments?:list<string|int|float|bool|null>,argumentSets?:list<list<string|int|float|bool|null>>}> $projection
     * @param Closure():void $complete
     * @param Closure(Throwable):void $failed
     */
    public function commitBatch(array $operations,array $projection,Closure $complete,Closure $failed):void
    {
        if($operations===[]||count($operations)>128||count($projection)>256)throw new InvalidArgumentException('Atomic outbox group exceeds operation/statement limit.');
        $statements=[];$clauses=[];$arguments=[];$ids=[];
        foreach($operations as $operation){
            if(isset($ids[$operation->identifier]))throw new InvalidArgumentException('Atomic outbox keys must be unique.');
            $ids[$operation->identifier]=true;
            $statements[]=['sql'=>'INSERT INTO '.$this->table.'(id,operation_kind,payload_json,created_at,operation_state,available_at) VALUES(?,?,?,?,?,?)','arguments'=>[$operation->identifier,$operation->kind,$operation->payloadJson,$operation->createdAtMillis,OutboxState::Pending->value,$operation->createdAtMillis]];
            $clauses[]='(id=? AND operation_kind=? AND payload_json=?)';
            array_push($arguments,$operation->identifier,$operation->kind,$operation->payloadJson);
        }
        $statements=[...$statements,...$projection];
        if(strlen(json_encode($statements,JSON_THROW_ON_ERROR))>524288)throw new InvalidArgumentException('Atomic outbox group exceeds 512 KiB.');
        SQLite::transaction($this->database,$statements,$complete,function(Throwable $original)use($operations,$clauses,$arguments,$complete,$failed):void{
            SQLite::query($this->database,'SELECT count(*) AS matches FROM '.$this->table.' WHERE '.implode(' OR ',$clauses),$arguments,static function(array $rows)use($operations,$original,$complete,$failed):void{
                if(($rows[0]['matches']??null)!==count($operations)){$failed($original);return;}
                $complete();
            },$failed);
        });
    }

    /**
     * Atomically leases ready intents; a process crash recovers naturally when
     * its lease expires. Claim size is bounded before crossing the bridge.
     *
     * @param Closure(list<ClaimedOperation>): void $complete
     * @param Closure(Throwable): void $failed
     */
    public function claim(int $nowMillis, int $leaseMillis, int $limit, Closure $complete, Closure $failed): void
    {
        if ($nowMillis < 0 || $leaseMillis < 1 || $leaseMillis > 300_000 || $limit < 1 || $limit > self::MAX_BATCH || $nowMillis > PHP_INT_MAX - $leaseMillis) {
            throw new InvalidArgumentException('Invalid outbox claim limits.');
        }
        $lease = bin2hex(random_bytes(16));
        SQLite::execute($this->database,
            'UPDATE ' . $this->table . ' SET operation_state = ?, lease = ?, lease_until = ?, attempts = attempts + 1
             WHERE id IN (SELECT id FROM ' . $this->table . ' WHERE available_at <= ?
               AND (operation_state = ? OR (operation_state = ? AND lease_until <= ?))
               ORDER BY created_at, id LIMIT ?)',
            [OutboxState::InFlight->value, $lease, $nowMillis + $leaseMillis, $nowMillis, OutboxState::Pending->value, OutboxState::InFlight->value, $nowMillis, $limit],
            function () use ($lease, $complete, $failed): void {
                SQLite::query($this->database,
                    'SELECT id, operation_kind, payload_json, created_at, attempts, lease, lease_until FROM ' . $this->table . ' WHERE lease = ? ORDER BY created_at, id LIMIT ?',
                    [$lease, self::MAX_BATCH],
                    static function (array $rows) use ($complete, $failed): void {
                        try {
                            $operations = array_map(ClaimedOperation::fromRow(...), $rows);
                        } catch (Throwable $error) {
                            $failed($error);
                            return;
                        }
                        $complete($operations);
                    }, $failed);
            }, $failed);
    }

    /**
     * Applied tombstones retain their original payload for idempotent replay.
     * Lease checks make late responses from an expired sender harmless.
     *
     * @param array<string, mixed> $result
     * @param Closure(): void $complete @param Closure(Throwable): void $failed
     */
    public function acknowledge(ClaimedOperation $operation, array $result, Closure $complete, Closure $failed): void
    {
        $json = json_encode($result, JSON_THROW_ON_ERROR | JSON_UNESCAPED_UNICODE);
        if (strlen($json) > OutboxOperation::MAX_PAYLOAD_BYTES) throw new InvalidArgumentException('Outbox result exceeds 64 KiB.');
        SQLite::execute($this->database,
            'UPDATE ' . $this->table . ' SET operation_state = ?, lease = NULL, lease_until = 0, last_error = \'\', result_json = ? WHERE id = ? AND lease = ? AND operation_state = ?',
            [OutboxState::Applied->value, $json, $operation->identifier, $operation->lease, OutboxState::InFlight->value], $complete, $failed);
    }

    /**
     * Atomically reconciles the local projection and acknowledges this exact
     * lease. A stale/already-settled response returns false without any writes.
     * @param array<string, mixed> $result
     * @param list<array{sql:string,arguments?:list<string|int|float|bool|null>,argumentSets?:list<list<string|int|float|bool|null>>}> $projection
     * @param Closure(bool): void $complete
     * @param Closure(Throwable): void $failed
     */
    public function acknowledgeWithProjection(ClaimedOperation $operation, array $result, array $projection, Closure $complete, Closure $failed): void
    {
        if (count($projection) > 256) throw new InvalidArgumentException('A projection may contain at most 256 statements.');
        $json = json_encode($result, JSON_THROW_ON_ERROR | JSON_UNESCAPED_UNICODE);
        if (strlen($json) > OutboxOperation::MAX_PAYLOAD_BYTES) throw new InvalidArgumentException('Outbox result exceeds 64 KiB.');
        SQLite::transactionIf($this->database,
            'SELECT EXISTS(SELECT 1 FROM ' . $this->table . ' WHERE id = ? AND lease = ? AND operation_state = ?)',
            [$operation->identifier, $operation->lease, OutboxState::InFlight->value],
            [...$projection, ['sql' => "UPDATE {$this->table} SET operation_state = ?, lease = NULL, lease_until = 0, last_error = '', result_json = ? WHERE id = ? AND lease = ? AND operation_state = ?",
                'arguments' => [OutboxState::Applied->value, $json, $operation->identifier, $operation->lease, OutboxState::InFlight->value]]],
            $complete, $failed);
    }

    /** @param Closure(): void $complete @param Closure(Throwable): void $failed */
    public function retry(ClaimedOperation $operation, int $availableAtMillis, string $error, Closure $complete, Closure $failed): void
    {
        if ($availableAtMillis < 0) throw new InvalidArgumentException('Retry time cannot be negative.');
        $this->settle($operation, OutboxState::Pending, $availableAtMillis, $error, $complete, $failed);
    }

    /** Permanent errors remain durable and inspectable, never silently dropped. */
    public function reject(ClaimedOperation $operation, string $error, Closure $complete, Closure $failed): void
    {
        $this->settle($operation, OutboxState::Failed, 0, $error, $complete, $failed);
    }

    private function settle(ClaimedOperation $operation, OutboxState $state, int $availableAt, string $error, Closure $complete, Closure $failed): void
    {
        SQLite::execute($this->database,
            'UPDATE ' . $this->table . ' SET operation_state = ?, available_at = ?, last_error = ?, lease = NULL, lease_until = 0 WHERE id = ? AND lease = ? AND operation_state = ?',
            [$state->value, $availableAt, mb_strcut($error, 0, 4096, 'UTF-8'), $operation->identifier, $operation->lease, OutboxState::InFlight->value], $complete, $failed);
    }
}
