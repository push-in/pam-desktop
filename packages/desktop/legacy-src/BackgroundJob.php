<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final readonly class BackgroundJob
{
    public const MIN_INTERVAL_MS = 1_000;
    public const MAX_INTERVAL_MS = 86_400_000;

    private function __construct(
        public int $intervalMilliseconds,
        public int $initialDelayMilliseconds,
        public int $timeoutMilliseconds,
        public JobOverlapPolicy $overlapPolicy,
        public bool $persistent,
        public int $maximumAttempts,
        public int $retryBackoffMilliseconds,
    ) {
        if (
            $intervalMilliseconds < self::MIN_INTERVAL_MS
            || $intervalMilliseconds > self::MAX_INTERVAL_MS
        ) {
            throw new InvalidArgumentException(
                sprintf(
                    'Background job intervals must be between %d and %d milliseconds.',
                    self::MIN_INTERVAL_MS,
                    self::MAX_INTERVAL_MS,
                ),
            );
        }
        if ($maximumAttempts < 1 || $maximumAttempts > 10) {
            throw new InvalidArgumentException('Background job attempts must be between 1 and 10.');
        }
        if ($retryBackoffMilliseconds < 100 || $retryBackoffMilliseconds > self::MAX_INTERVAL_MS) {
            throw new InvalidArgumentException('Background job retry backoff must be between 100ms and one day.');
        }
        if (
            $initialDelayMilliseconds < 0
            || $initialDelayMilliseconds > self::MAX_INTERVAL_MS
        ) {
            throw new InvalidArgumentException(
                sprintf(
                    'Background job initial delays must be between 0 and %d milliseconds.',
                    self::MAX_INTERVAL_MS,
                ),
            );
        }
        if (
            $timeoutMilliseconds < Application::MIN_COMMAND_TIMEOUT_MS
            || $timeoutMilliseconds > Application::MAX_COMMAND_TIMEOUT_MS
        ) {
            throw new InvalidArgumentException(
                sprintf(
                    'Background job timeouts must be between %d and %d milliseconds.',
                    Application::MIN_COMMAND_TIMEOUT_MS,
                    Application::MAX_COMMAND_TIMEOUT_MS,
                ),
            );
        }
    }

    public static function every(int $milliseconds): self
    {
        return new self(
            $milliseconds,
            $milliseconds,
            min($milliseconds, 30_000),
            JobOverlapPolicy::Skip,
            false,
            1,
            1_000,
        );
    }

    public function initialDelay(int $milliseconds): self
    {
        return new self(
            $this->intervalMilliseconds,
            $milliseconds,
            $this->timeoutMilliseconds,
            $this->overlapPolicy,
            $this->persistent,
            $this->maximumAttempts,
            $this->retryBackoffMilliseconds,
        );
    }

    public function runOnStart(): self
    {
        return $this->initialDelay(0);
    }

    public function timeout(int $milliseconds): self
    {
        return new self(
            $this->intervalMilliseconds,
            $this->initialDelayMilliseconds,
            $milliseconds,
            $this->overlapPolicy,
            $this->persistent,
            $this->maximumAttempts,
            $this->retryBackoffMilliseconds,
        );
    }

    public function overlap(JobOverlapPolicy $policy): self
    {
        return new self(
            $this->intervalMilliseconds,
            $this->initialDelayMilliseconds,
            $this->timeoutMilliseconds,
            $policy,
            $this->persistent,
            $this->maximumAttempts,
            $this->retryBackoffMilliseconds,
        );
    }

    public function persistent(int $maximumAttempts = 3, int $retryBackoffMilliseconds = 1_000): self
    {
        return new self(
            $this->intervalMilliseconds,
            $this->initialDelayMilliseconds,
            $this->timeoutMilliseconds,
            $this->overlapPolicy,
            true,
            $maximumAttempts,
            $retryBackoffMilliseconds,
        );
    }

    /**
     * @return array{
     *     id: string,
     *     intervalMs: int,
     *     initialDelayMs: int,
     *     timeoutMs: int,
     *     overlap: int,
     *     persistent: bool,
     *     maximumAttempts: int,
     *     retryBackoffMs: int
     * }
     */
    public function toArray(string $id): array
    {
        Identifier::assert($id, 'The background job identifier');

        return [
            'id' => $id,
            'intervalMs' => $this->intervalMilliseconds,
            'initialDelayMs' => $this->initialDelayMilliseconds,
            'timeoutMs' => $this->timeoutMilliseconds,
            'overlap' => $this->overlapPolicy->value,
            'persistent' => $this->persistent,
            'maximumAttempts' => $this->maximumAttempts,
            'retryBackoffMs' => $this->retryBackoffMilliseconds,
        ];
    }
}
