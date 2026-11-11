<?php

declare(strict_types=1);

namespace Pam\DesktopSsh;

use Closure;
use Pam\Desktop\Modules\NativeModuleException;
use Pam\Desktop\Modules\NativeModuleResult;
use Pam\Desktop\Modules\NativeModules;

/** A local port forward (`ssh -L`). */
final class PortForward
{
    private bool $active = true;
    private int $connections = 0;
    private int $failures = 0;
    /** @var (Closure(int, int): void)|null */
    private ?Closure $onConnections = null;
    /** @var (Closure(SshException): void)|null */
    private ?Closure $onFailed = null;

    /** @internal */
    public function __construct(public readonly string $id, private readonly int $port)
    {
    }

    /** The local port that accepts connections. */
    public function port(): int
    {
        return $this->port;
    }

    public function isActive(): bool
    {
        return $this->active;
    }

    public function connections(): int
    {
        return $this->connections;
    }

    /** Connections refused at the limit (64) or by the server. */
    public function failures(): int
    {
        return $this->failures;
    }

    /** @param Closure(int $connections, int $failures): void $listener */
    public function onConnections(Closure $listener): self
    {
        $this->onConnections = $listener;

        return $this;
    }

    /** @param Closure(SshException): void $listener The listener stopped on its own. */
    public function onFailed(Closure $listener): self
    {
        $this->onFailed = $listener;

        return $this;
    }

    public function stop(): void
    {
        if (!$this->active) {
            return;
        }
        $this->active = false;
        NativeModules::call('ssh', 'forwardStop', ['forward' => $this->id], static function (NativeModuleResult $result): void {
        }, static function (NativeModuleException $error): void {
        });
    }

    /** @internal */
    public function counted(int $connections, int $failures): void
    {
        $this->connections = $connections;
        $this->failures = $failures;
        ($this->onConnections)?->__invoke($connections, $failures);
    }

    /** @internal */
    public function failed(SshException $error): void
    {
        if (!$this->active) {
            return;
        }
        $this->active = false;
        ($this->onFailed)?->__invoke($error);
    }

    /** @internal */
    public function ended(): void
    {
        $this->active = false;
    }
}
