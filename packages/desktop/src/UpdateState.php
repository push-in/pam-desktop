<?php

declare(strict_types=1);

namespace Pam\Desktop;

/**
 * Where the signed update of the application stands.
 *
 * ```php
 * Updates::onChange(function (UpdateState $state): void {
 *     if ($state->isAvailable()) {
 *         $this->banner = "Version {$state->availableVersion} is available";
 *     }
 * });
 * ```
 */
final readonly class UpdateState
{
    /** `pam-desktop.json` declares no `updates`. */
    public const string DISABLED = 'disabled';
    public const string IDLE = 'idle';
    public const string CHECKING = 'checking';
    /** A newer release exists: `Updates::download()` fetches it. */
    public const string AVAILABLE = 'available';
    public const string DOWNLOADING = 'downloading';
    /** Downloaded and verified: `Updates::install()` applies it. */
    public const string READY = 'ready';
    /** The application is about to close and start again updated. */
    public const string APPLYING = 'applying';
    public const string UP_TO_DATE = 'up-to-date';
    public const string FAILED = 'failed';

    public function __construct(
        public string $status = self::DISABLED,
        public string $currentVersion = '',
        public ?string $availableVersion = null,
        public int $downloadedBytes = 0,
        public int $totalBytes = 0,
        public ?string $notesUrl = null,
        public ?string $error = null,
        /** `manual`, `notify` or `automatic`; `null` when updates are disabled. */
        public ?string $policy = null,
    ) {
    }

    public function isEnabled(): bool
    {
        return $this->status !== self::DISABLED;
    }

    public function isAvailable(): bool
    {
        return $this->status === self::AVAILABLE;
    }

    public function isReady(): bool
    {
        return $this->status === self::READY;
    }

    public function isFailed(): bool
    {
        return $this->status === self::FAILED;
    }

    /** A check, a download or an installation is running. */
    public function isBusy(): bool
    {
        return in_array($this->status, [self::CHECKING, self::DOWNLOADING, self::APPLYING], true);
    }

    /** Download progress from 0 to 1. */
    public function progress(): float
    {
        return $this->totalBytes > 0 ? max(0.0, min(1.0, $this->downloadedBytes / $this->totalBytes)) : 0.0;
    }

    /**
     * @internal
     * @param array<array-key, mixed> $values
     */
    public static function fromArray(array $values): self
    {
        $text = static fn (string $key): ?string => is_string($values[$key] ?? null) && $values[$key] !== '' ? $values[$key] : null;
        $bytes = static fn (string $key): int => is_int($values[$key] ?? null) ? max(0, $values[$key]) : 0;

        return new self(
            $text('state') ?? self::DISABLED,
            $text('currentVersion') ?? '',
            $text('availableVersion'),
            $bytes('downloadedBytes'),
            $bytes('totalBytes'),
            $text('notesUrl'),
            $text('error'),
            $text('policy'),
        );
    }

    /** @internal */
    public static function failed(string $error): self
    {
        return new self(self::FAILED, error: $error);
    }
}
