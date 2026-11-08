<?php

declare(strict_types=1);

namespace Pam\Desktop\Sync\Durable;

use Closure;
use InvalidArgumentException;
use Throwable;

/**
 * Dispatches one bounded batch when an application asks it to. It owns no
 * timer, network client, or visibility policy; callers run it only while the
 * app is active and connected.
 */
final class OutboxDispatcher
{
    private bool $dispatching = false;

    public function __construct(
        private readonly SQLiteOutbox $outbox,
        private readonly int $leaseMillis = 30_000,
        private readonly int $batchSize = SQLiteOutbox::MAX_BATCH,
    ) {
        if ($leaseMillis < 1 || $leaseMillis > 300_000 || $batchSize < 1 || $batchSize > SQLiteOutbox::MAX_BATCH) {
            throw new InvalidArgumentException('Outbox dispatcher limits are invalid.');
        }
    }

    /**
     * The sender must validate the remote response and build SQL only from
     * fixed statements plus bound arguments. A transport failure is retryable;
     * adapters use OutboxResolution::rejected() for permanent domain failures.
     *
     * @param Closure(ClaimedOperation, Closure(OutboxResolution): void, Closure(Throwable): void): void $send
     * @param Closure(OutboxDispatchReport): void $complete
     * @param Closure(Throwable): void $failed
     */
    public function dispatch(
        int $nowMillis,
        Closure $online,
        Closure $send,
        Closure $complete,
        Closure $failed,
    ): void {
        if ($nowMillis < 0) throw new InvalidArgumentException('Dispatch time cannot be negative.');
        if ($this->dispatching) {
            $complete(new OutboxDispatchReport());
            return;
        }
        try {
            if (!$online()) {
                $complete(new OutboxDispatchReport());
                return;
            }
        } catch (Throwable $error) {
            $failed($error);
            return;
        }

        $this->dispatching = true;
        try {
            $this->outbox->claim($nowMillis, $this->leaseMillis, $this->batchSize,
                function (array $operations) use ($nowMillis, $send, $complete, $failed): void {
                    if ($operations === []) {
                        $this->dispatching = false;
                        $complete(new OutboxDispatchReport());
                        return;
                    }
                    $counts = ['applied' => 0, 'retried' => 0, 'rejected' => 0, 'stale' => 0, 'failures' => 0];
                    $remaining = count($operations);
                    $finish = function () use (&$remaining, &$counts, $operations, $complete): void {
                        if (--$remaining !== 0) return;
                        $this->dispatching = false;
                        $complete(new OutboxDispatchReport(
                            claimed: count($operations),
                            applied: $counts['applied'],
                            retried: $counts['retried'],
                            rejected: $counts['rejected'],
                            stale: $counts['stale'],
                            failures: $counts['failures'],
                        ));
                    };
                    foreach ($operations as $operation) {
                        $settled = false;
                        $settle = function (OutboxResolution $resolution) use ($operation, $nowMillis, &$settled, &$counts, $finish): void {
                            if ($settled) return;
                            $settled = true;
                            $after = function () use (&$counts, $finish): void { $finish(); };
                            $onFailure = function (Throwable $error) use (&$counts, $finish): void {
                                ++$counts['failures'];
                                $finish();
                            };
                            try {
                                match ($resolution->disposition) {
                                    OutboxDisposition::Applied => $this->outbox->acknowledgeWithProjection(
                                        $operation,
                                        $resolution->result,
                                        $resolution->projection,
                                        function (bool $applied) use (&$counts, $after): void {
                                            ++$counts[$applied ? 'applied' : 'stale'];
                                            $after();
                                        },
                                        $onFailure,
                                    ),
                                    OutboxDisposition::Retry => $this->outbox->retry(
                                        $operation,
                                        $nowMillis + self::retryDelayMillis($operation->attempts),
                                        $resolution->message,
                                        function () use (&$counts, $after): void { ++$counts['retried']; $after(); },
                                        $onFailure,
                                    ),
                                    OutboxDisposition::Rejected => $this->outbox->reject(
                                        $operation,
                                        $resolution->message,
                                        function () use (&$counts, $after): void { ++$counts['rejected']; $after(); },
                                        $onFailure,
                                    ),
                                };
                            } catch (Throwable $error) {
                                $onFailure($error);
                            }
                        };
                        $onSendFailure = static function (Throwable $error) use ($settle): void {
                            $settle(OutboxResolution::retry($error->getMessage() !== '' ? $error->getMessage() : 'Transport failed.'));
                        };
                        try {
                            $send($operation, $settle, $onSendFailure);
                        } catch (Throwable $error) {
                            $onSendFailure($error);
                        }
                    }
                },
                function (Throwable $error) use ($failed): void {
                    $this->dispatching = false;
                    $failed($error);
                },
            );
        } catch (Throwable $error) {
            $this->dispatching = false;
            $failed($error);
        }
    }

    private static function retryDelayMillis(int $attempt): int
    {
        return 1000 * (2 ** min(max(0, $attempt - 1), 8));
    }
}
