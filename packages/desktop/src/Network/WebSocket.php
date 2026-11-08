<?php

declare(strict_types=1);

namespace Pam\Desktop\Network;

use Closure;
use DomainException;
use Pam\Desktop\Component;
use Pam\Desktop\Disposable;
use Pam\Desktop\Modules\{NativeModules,NativeModuleResult,NativeModuleException};

/** Native bounded text WebSocket. Lifecycle/reconnection policies belong to its owner. */
final class WebSocket implements Disposable
{
    private ?string $id = null;
    private bool $closed = false;
    private bool $ready = false;
    /** @param Closure(WebSocketEvent,string):void $event @param Closure(NativeModuleException):void $failed */
    private function __construct(private readonly Closure $event, private readonly Closure $failed)
    {
    }
    /** Creates at most one native reader, parked until a message arrives; no PHP polling.
     * @param Closure(WebSocketEvent,string):void $event @param Closure(NativeModuleException):void $failed */
    public static function connect(string $url, Closure $event, Closure $failed, ?Component $owner = null): self
    {
        if (strlen($url) > 2048 || !preg_match('~^wss?://~D', $url)) {
            throw new DomainException('A WebSocket URL must use ws:// or wss://.');
        }
        $socket = new self($event, $failed);
        $owner?->__pamOwn($socket);
        NativeModules::call('websocket', 'open', ['url' => $url], function (NativeModuleResult $result) use ($socket): void {
            $id = $result->values()['id'] ?? null;
            if (!is_string($id) || $id === '') {
                $socket->fail(new NativeModuleException('websocket', 'open', 'Invalid WebSocket handle'));
                return;
            }
            $socket->id = $id;
            if ($socket->closed) {
                $socket->release();
                return;
            }$socket->read();
        }, $socket->fail(...));
        return $socket;
    }
    /** Acceptance means queued, not acknowledged by the remote application.
     * @param Closure():void|null $queued @param Closure(NativeModuleException):void|null $failed */
    public function send(string $text, ?Closure $queued = null, ?Closure $failed = null): void
    {
        if ($this->closed || !$this->ready || $this->id === null) {
            throw new DomainException('WebSocket is not open.');
        }
        if (strlen($text) > 65536 || preg_match('//u', $text) !== 1) {
            throw new DomainException('WebSocket text must be valid UTF-8 and at most 64 KiB.');
        }
        NativeModules::call('websocket', 'send', ['id' => $this->id,'text' => $text], static function (NativeModuleResult $result) use ($queued): void {
            $queued?->__invoke();
        }, $failed ?? $this->fail(...));
    }
    public function isOpen(): bool
    {
        return $this->ready && !$this->closed;
    }
    public function close(): void
    {
        if ($this->closed) {
            return;
        }$this->closed = true;
        $this->ready = false;
        $this->release();
    }
    public function dispose(): void
    {
        $this->close();
    }
    public function disposed(): bool
    {
        return $this->closed;
    }
    private function release(): void
    {
        if ($this->id === null) {
            return;
        }$id = $this->id;
        $this->id = null;
        NativeModules::call('websocket', 'close', ['id' => $id], static function (NativeModuleResult $result): void {
        }, static function (NativeModuleException $error): void {
        });
    }
    private function read(): void
    {
        if ($this->closed || $this->id === null) {
            return;
        }
        NativeModules::call('websocket', 'next', ['id' => $this->id], function (NativeModuleResult $result): void {
            if ($this->closed) {
                return;
            }$data = $result->values();
            $kind = $data['kind'] ?? null;
            $text = $data['text'] ?? '';
            $event = is_int($kind) ? WebSocketEvent::tryFrom($kind) : null;
            if ($event === null || !is_string($text)) {
                $this->fail(new NativeModuleException('websocket', 'next', 'Invalid WebSocket event'));
                return;
            }
            if ($event === WebSocketEvent::Open) {
                $this->ready = true;
            }
            if ($event === WebSocketEvent::Closed || $event === WebSocketEvent::Failed) {
                $this->close();
            }
            try {
                ($this->event)($event, $text);
            } catch (\Throwable $error) {
                $this->close();
                throw $error;
            }
            if (!$this->closed) {
                $this->read();
            }
        }, $this->fail(...));
    }
    private function fail(NativeModuleException $error): void
    {
        if ($this->closed) {
            return;
        }$this->close();
        ($this->failed)($error);
    }
}
