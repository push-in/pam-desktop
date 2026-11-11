<?php

declare(strict_types=1);

namespace Pam\DesktopSsh;

use Closure;
use DomainException;
use Pam\Desktop\Disposable;
use Pam\Desktop\Modules\NativeModuleException;
use Pam\Desktop\Modules\NativeModuleResult;
use Pam\Desktop\Modules\NativeModules;

/**
 * An open SSH session. While it is open one `next` call waits in the host for
 * its events (shell output, transfer progress, forwards, the end of the
 * session) and routes them to the shells, transfers and forwards they belong
 * to; PHP does not poll.
 */
final class SshSession implements Disposable
{
    private ?string $id = null;
    private bool $closed = false;
    private string $fingerprint = '';
    private string $algorithm = '';
    private ?SftpClient $sftp = null;
    /** @var array<string, Shell> */
    private array $shells = [];
    /** @var array<string, Transfer> */
    private array $transfers = [];
    /** @var array<string, PortForward> */
    private array $forwards = [];
    /** @var list<Closure(SshEvent): void> */
    private array $listeners = [];
    /** @var (Closure(?SshException): void)|null */
    private ?Closure $onClose = null;

    private function __construct()
    {
    }

    /** @internal */
    public static function pending(): self
    {
        return new self();
    }

    /** @internal Called by Ssh::connect(); false when the session was disposed meanwhile. */
    public function opened(string $id, string $fingerprint, string $algorithm): bool
    {
        $this->id = $id;
        $this->fingerprint = $fingerprint;
        $this->algorithm = $algorithm;
        if ($this->closed) {
            $this->release();

            return false;
        }
        $this->read();

        return true;
    }

    /** The host's handle; empty while connecting. */
    public function id(): string
    {
        return $this->id ?? '';
    }

    public function isOpen(): bool
    {
        return $this->id !== null && !$this->closed;
    }

    /** SHA-256 of the server's host key, base64 without padding or `SHA256:`. Store it to pin the server. */
    public function fingerprint(): string
    {
        return $this->fingerprint;
    }

    /** The host key algorithm, such as `ssh-ed25519`. */
    public function algorithm(): string
    {
        return $this->algorithm;
    }

    /**
     * Runs a command and answers with its exit code and output (stdout and
     * stderr together are capped at `$maxBytes`; `truncated` says if more
     * was dropped).
     *
     * @param Closure(ExecResult): void $done
     * @param (Closure(SshException): void)|null $failed
     */
    public function exec(string $command, Closure $done, ?Closure $failed = null, int $timeoutMs = 30_000, int $maxBytes = 983_040): void
    {
        if ($command === '' || strlen($command) > 65_536) {
            throw new DomainException('SSH command must have 1 to 65536 bytes.');
        }
        Native::call('exec', [
            'id' => $this->handle(),
            'command' => $command,
            'timeoutMs' => $timeoutMs,
            'maxBytes' => $maxBytes,
        ], static function (array $values) use ($done): void {
            $done(new ExecResult(
                Native::int($values, 'exitCode', -1),
                Native::string($values, 'stdout'),
                Native::string($values, 'stderr'),
                Native::bool($values, 'truncated'),
            ));
        }, $failed);
    }

    /** The SFTP client of this session; one subsystem channel is shared by every call. */
    public function sftp(): SftpClient
    {
        return $this->sftp ??= new SftpClient($this);
    }

    /**
     * Opens an interactive shell on a pseudo-terminal. Its output arrives in
     * Shell::onData() unless a native terminal element is attached to
     * Shell::channel().
     *
     * @param Closure(Shell): void $opened
     * @param (Closure(SshException): void)|null $failed
     */
    public function shell(Closure $opened, ?Closure $failed = null, int $cols = 80, int $rows = 24, string $term = 'xterm-256color'): void
    {
        Shell::validateSize($cols, $rows);
        Native::call('shellOpen', [
            'id' => $this->handle(),
            'cols' => $cols,
            'rows' => $rows,
            'term' => $term,
        ], function (array $values) use ($opened): void {
            $channel = Native::string($values, 'channel');
            $shell = new Shell($channel);
            if ($this->closed) {
                $shell->close();

                return;
            }
            $this->shells[$channel] = $shell;
            $opened($shell);
        }, $failed);
    }

    /**
     * Listens on this machine and carries each connection to
     * `$remoteHost:$remotePort` as seen from the server (`ssh -L`). Port 0
     * picks a free port: read it from PortForward::port().
     *
     * @param Closure(PortForward): void $opened
     * @param (Closure(SshException): void)|null $failed
     */
    public function forward(int $bindPort, string $remoteHost, int $remotePort, Closure $opened, ?Closure $failed = null, string $bindHost = '127.0.0.1', bool $allowNonLoopback = false): void
    {
        if ($bindPort < 0 || $bindPort > 65535 || $remotePort < 1 || $remotePort > 65535) {
            throw new DomainException('SSH forward ports must be between 1 and 65535 (0 picks a local port).');
        }
        Native::call('forwardLocal', [
            'id' => $this->handle(),
            'bindHost' => $bindHost,
            'bindPort' => $bindPort,
            'remoteHost' => $remoteHost,
            'remotePort' => $remotePort,
            'allowNonLoopback' => $allowNonLoopback,
        ], function (array $values) use ($opened): void {
            $forward = new PortForward(Native::string($values, 'forward'), Native::int($values, 'port'));
            if ($this->closed) {
                $forward->stop();

                return;
            }
            $this->forwards[$forward->id] = $forward;
            $opened($forward);
        }, $failed);
    }

    /**
     * Every event of the session, after it was routed to its shell, transfer
     * or forward.
     *
     * @param Closure(SshEvent): void $listener
     */
    public function onEvent(Closure $listener): self
    {
        $this->listeners[] = $listener;

        return $this;
    }

    /**
     * The session ended on its own: the server closed it (null) or the
     * connection failed. Not called after disconnect(). Without a listener a
     * failure is thrown, like an unhandled module failure.
     *
     * @param Closure(?SshException): void $listener
     */
    public function onClose(Closure $listener): self
    {
        $this->onClose = $listener;

        return $this;
    }

    public function disconnect(): void
    {
        if ($this->closed) {
            return;
        }
        $this->closed = true;
        $this->release();
        $this->forget();
    }

    public function dispose(): void
    {
        $this->disconnect();
    }

    public function disposed(): bool
    {
        return $this->closed;
    }

    /** @internal */
    public function handle(): string
    {
        if ($this->closed || $this->id === null) {
            throw new DomainException('SSH session is not open.');
        }

        return $this->id;
    }

    /** @internal */
    public function track(Transfer $transfer): void
    {
        if ($this->closed) {
            $transfer->cancel();

            return;
        }
        $this->transfers[$transfer->id()] = $transfer;
    }

    private function release(): void
    {
        if ($this->id === null) {
            return;
        }
        NativeModules::call('ssh', 'disconnect', ['id' => $this->id], static function (NativeModuleResult $result): void {
        }, static function (NativeModuleException $error): void {
        });
    }

    private function forget(): void
    {
        foreach ($this->shells as $shell) {
            $shell->ended(null);
        }
        foreach ($this->transfers as $transfer) {
            $transfer->failed(new SshException('ssh: session closed'), false);
        }
        foreach ($this->forwards as $forward) {
            $forward->ended();
        }
        $this->shells = $this->transfers = $this->forwards = [];
    }

    private function read(): void
    {
        if ($this->closed || $this->id === null) {
            return;
        }
        NativeModules::call('ssh', 'next', ['id' => $this->id], function (NativeModuleResult $result): void {
            if ($this->closed) {
                return;
            }
            $values = $result->values();
            $kind = $values['kind'] ?? null;
            $kind = is_int($kind) ? SshEventKind::tryFrom($kind) : null;
            if ($kind === null) {
                $this->end(new SshException('ssh: invalid event'));

                return;
            }
            $event = new SshEvent($kind, $values);
            try {
                $this->dispatch($event);
                foreach ($this->listeners as $listener) {
                    $listener($event);
                }
            } finally {
                $this->read();
            }
        }, function (NativeModuleException $error): void {
            if (!$this->closed) {
                $this->end(SshException::fromNative($error));
            }
        });
    }

    private function dispatch(SshEvent $event): void
    {
        switch ($event->kind) {
            case SshEventKind::ShellData:
                ($this->shells[$event->string('channel')] ?? null)?->received($event->string('bytes'));
                break;
            case SshEventKind::ShellExit:
                $channel = $event->string('channel');
                $shell = $this->shells[$channel] ?? null;
                unset($this->shells[$channel]);
                $shell?->ended($event->int('exitCode', -1));
                break;
            case SshEventKind::TransferProgress:
                ($this->transfers[$event->string('transfer')] ?? null)?->progressed($event->int('bytes'), $event->int('total'));
                break;
            case SshEventKind::TransferDone:
            case SshEventKind::TransferFailed:
                $id = $event->string('transfer');
                $transfer = $this->transfers[$id] ?? null;
                unset($this->transfers[$id]);
                if ($event->kind === SshEventKind::TransferDone) {
                    $transfer?->finished($event->int('bytes'));
                } else {
                    $transfer?->failed(new SshException($event->string('message')));
                }
                break;
            case SshEventKind::ForwardOpened:
                break;
            case SshEventKind::ForwardConnections:
                ($this->forwards[$event->string('forward')] ?? null)?->counted($event->int('connections'), $event->int('failures'));
                break;
            case SshEventKind::ForwardFailed:
                $id = $event->string('forward');
                $forward = $this->forwards[$id] ?? null;
                unset($this->forwards[$id]);
                $forward?->failed(new SshException($event->string('message')));
                break;
            case SshEventKind::SessionClosed:
                $this->end(null);
                break;
            case SshEventKind::SessionFailed:
                $this->end(new SshException($event->string('message')));
                break;
        }
    }

    private function end(?SshException $error): void
    {
        if ($this->closed) {
            return;
        }
        $this->closed = true;
        $this->release();
        $this->forget();
        if ($this->onClose === null && $error !== null) {
            throw $error;
        }
        ($this->onClose)?->__invoke($error);
    }
}
