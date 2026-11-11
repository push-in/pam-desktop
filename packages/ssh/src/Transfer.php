<?php

declare(strict_types=1);

namespace Pam\DesktopSsh;

use Closure;
use Pam\Desktop\Modules\NativeModuleException;
use Pam\Desktop\Modules\NativeModuleResult;
use Pam\Desktop\Modules\NativeModules;

/**
 * An upload or a download running in the host. Progress arrives at most ten
 * times a second; exactly one of done or failed ends it. A download is
 * written to `<localPath>.part` and renamed when complete.
 */
final class Transfer
{
    private ?string $id = null;
    private bool $finished = false;
    private bool $cancelled = false;
    private int $bytes = 0;
    private int $total = 0;

    /**
     * @internal
     * @param (Closure(int, int): void)|null $progress
     * @param (Closure(int): void)|null $done
     * @param (Closure(SshException): void)|null $failed
     */
    public function __construct(
        private readonly ?Closure $progress,
        private readonly ?Closure $done,
        private readonly ?Closure $failed,
    ) {
    }

    /** The host's handle; empty until the host accepted the transfer. */
    public function id(): string
    {
        return $this->id ?? '';
    }

    public function bytes(): int
    {
        return $this->bytes;
    }

    public function total(): int
    {
        return $this->total;
    }

    public function isFinished(): bool
    {
        return $this->finished;
    }

    /** Stops the transfer; it ends with a failure whose message is `cancelled`. */
    public function cancel(): void
    {
        if ($this->finished || $this->cancelled) {
            return;
        }
        $this->cancelled = true;
        if ($this->id !== null) {
            NativeModules::call('ssh', 'sftpCancel', ['transfer' => $this->id], static function (NativeModuleResult $result): void {
            }, static function (NativeModuleException $error): void {
            });
        }
    }

    /** @internal */
    public function started(string $id): void
    {
        $this->id = $id;
        if ($this->cancelled) {
            $this->cancelled = false;
            $this->cancel();
        }
    }

    /** @internal */
    public function progressed(int $bytes, int $total): void
    {
        if ($this->finished) {
            return;
        }
        $this->bytes = $bytes;
        $this->total = $total;
        ($this->progress)?->__invoke($bytes, $total);
    }

    /** @internal */
    public function finished(int $bytes): void
    {
        if ($this->finished) {
            return;
        }
        $this->finished = true;
        $this->bytes = $bytes;
        ($this->done)?->__invoke($bytes);
    }

    /** @internal Without a failure closure the error is thrown, unless the whole session ended. */
    public function failed(SshException $error, bool $throw = true): void
    {
        if ($this->finished) {
            return;
        }
        $this->finished = true;
        if ($this->failed !== null) {
            ($this->failed)($error);
        } elseif ($throw) {
            throw $error;
        }
    }
}
