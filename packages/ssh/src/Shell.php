<?php

declare(strict_types=1);

namespace Pam\DesktopSsh;

use Closure;
use DomainException;
use Pam\Desktop\Modules\NativeModuleException;
use Pam\Desktop\Modules\NativeModuleResult;
use Pam\Desktop\Modules\NativeModules;
use Pam\Desktop\Modules\WireBytes;

/**
 * An interactive shell on a pseudo-terminal.
 *
 * channel() is also the id of the shell in the host's terminal hub: a native
 * terminal element bound to it receives the output directly and sends
 * keystrokes without PHP. While one is attached, onData() is not called.
 */
final class Shell
{
    private bool $open = true;
    /** @var (Closure(string): void)|null */
    private ?Closure $onData = null;
    /** @var (Closure(?int): void)|null */
    private ?Closure $onExit = null;

    /** @internal */
    public function __construct(private readonly string $channel)
    {
    }

    public function channel(): string
    {
        return $this->channel;
    }

    public function isOpen(): bool
    {
        return $this->open;
    }

    /** @param Closure(string): void $listener Raw output bytes, in order. */
    public function onData(Closure $listener): self
    {
        $this->onData = $listener;

        return $this;
    }

    /** @param Closure(?int): void $listener The exit status (-1 if the server sent none); null when the session ended. */
    public function onExit(Closure $listener): self
    {
        $this->onExit = $listener;

        return $this;
    }

    /**
     * Sends keystrokes or pasted text (at most 64 KiB per call).
     *
     * @param (Closure(SshException): void)|null $failed
     */
    public function write(string $bytes, ?Closure $failed = null): void
    {
        if (strlen($bytes) > 65_536) {
            throw new DomainException('A shell write is at most 64 KiB.');
        }
        if (!$this->open || $bytes === '') {
            return;
        }
        Native::call('shellWrite', ['channel' => $this->channel, 'bytes' => new WireBytes($bytes)], static function (array $values): void {
        }, $failed ?? static function (SshException $error): void {
        });
    }

    /** @param (Closure(SshException): void)|null $failed */
    public function resize(int $cols, int $rows, ?Closure $failed = null): void
    {
        self::validateSize($cols, $rows);
        if (!$this->open) {
            return;
        }
        Native::call('shellResize', ['channel' => $this->channel, 'cols' => $cols, 'rows' => $rows], static function (array $values): void {
        }, $failed ?? static function (SshException $error): void {
        });
    }

    public function close(): void
    {
        if (!$this->open) {
            return;
        }
        NativeModules::call('ssh', 'shellClose', ['channel' => $this->channel], static function (NativeModuleResult $result): void {
        }, static function (NativeModuleException $error): void {
        });
    }

    /** @internal */
    public static function validateSize(int $cols, int $rows): void
    {
        if ($cols < 1 || $cols > 1000 || $rows < 1 || $rows > 1000) {
            throw new DomainException('Terminal size must be 1 to 1000 columns and rows.');
        }
    }

    /** @internal */
    public function received(string $bytes): void
    {
        if ($this->open) {
            ($this->onData)?->__invoke($bytes);
        }
    }

    /** @internal */
    public function ended(?int $status): void
    {
        if (!$this->open) {
            return;
        }
        $this->open = false;
        ($this->onExit)?->__invoke($status);
    }
}
