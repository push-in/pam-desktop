<?php

declare(strict_types=1);

namespace Pam\Desktop\Realtime;

use Closure;
use Pam\Desktop\{App,AppState,Component,Disposable};
use Pam\Desktop\Http\{HttpClient,PendingResponse,Response};
use Pam\Desktop\Network\{WebSocket,WebSocketEvent};
use Pam\Desktop\System\{Timers,TimerHandle};
use Throwable;

/** One private channel, exact event allowlist, no payload logging and no hidden polling. */
final class ReverbSubscription implements Disposable
{
    private ?WebSocket $socket = null;
    private ?PendingResponse $authorization = null;
    private ?TimerHandle $retry = null;
    private ?TimerHandle $heartbeat = null;
    private ?TimerHandle $deadline = null;
    private bool $authorized = false;
    private bool $closed = false;
    private bool $suspended = false;
    private bool $rejected = false;
    private int $generation = 0;
    private int $attempt = 0;
    private int $stateListener;
    private int $activityTimeout = 120000;
    private ConnectionState $state = ConnectionState::Connecting;
    /** @param list<string> $events @param Closure(string,array<string,mixed>):void $message @param Closure(ConnectionState):void $changed */
    private function __construct(private readonly ReverbConfig $config, private readonly HttpClient $http, private readonly string $channel, private readonly array $events, private readonly Closure $message, private readonly Closure $changed)
    {
        $this->suspended = App::state() === AppState::Background;
        $this->stateListener = App::onStateChange(function (AppState $state): void {
            if ($state === AppState::Background) {
                $this->suspend();
            } elseif ($state === AppState::Active) {
                $this->resume();
            }
        });
    }
    /** Never put a bearer token in the WebSocket URL or callbacks. The provided client carries the selected session header.
     * @param list<string> $events @param Closure(string,array<string,mixed>):void $message @param Closure(ConnectionState):void $changed */
    public static function privateChannel(ReverbConfig $config, HttpClient $authenticatedHttp, string $channel, array $events, Closure $message, Closure $changed, ?Component $owner = null): self
    {
        if (preg_match('/^[A-Za-z0-9_.-]{1,180}$/D', $channel) !== 1 || str_starts_with($channel, 'private-') || count($events) > 32 || $events === []) {
            throw new \InvalidArgumentException('Invalid private channel.');
        }
        foreach ($events as $event) {
            if (preg_match('/^[A-Za-z0-9_.-]{1,180}$/D', $event) !== 1) {
                throw new \InvalidArgumentException('Invalid event name.');
            }
        }
        $subscription = new self($config, $authenticatedHttp->withTimeout(10000)->withRetry(0), 'private-'.$channel, $events, $message, $changed);
        $owner?->__pamOwn($subscription);
        if ($subscription->suspended) {
            $subscription->setState(ConnectionState::Suspended);
        } else {
            $subscription->connect();
        }return $subscription;
    }
    public function state(): ConnectionState
    {
        return $this->state;
    }
    public function suspend(): void
    {
        if ($this->closed || $this->suspended) {
            return;
        }$this->suspended = true;
        $this->stopTransport();
        $this->setState(ConnectionState::Suspended);
    }
    public function resume(): void
    {
        if ($this->closed || !$this->suspended) {
            return;
        }$this->suspended = false;
        if ($this->rejected) {
            $this->setState(ConnectionState::Rejected);
            return;
        }$this->connect();
    }
    public function cancel(): void
    {
        if ($this->closed) {
            return;
        }$this->closed = true;
        $this->stopTransport();
        App::offStateChange($this->stateListener);
        $this->setState(ConnectionState::Closed);
    }
    public function dispose(): void
    {
        $this->cancel();
    }
    public function disposed(): bool
    {
        return $this->closed;
    }
    private function setState(ConnectionState $state): void
    {
        $this->state = $state;
        ($this->changed)($state);
    }
    private function stopTransport(): void
    {
        $this->authorized = false;
        ++$this->generation;
        $this->retry?->cancel();
        $this->retry = null;
        $this->heartbeat?->cancel();
        $this->heartbeat = null;
        $this->deadline?->cancel();
        $this->deadline = null;
        $this->authorization?->cancel();
        $this->authorization = null;
        $this->socket?->close();
        $this->socket = null;
    }
    private function connect(): void
    {
        if ($this->closed || $this->suspended || $this->rejected) {
            return;
        }$this->stopTransport();
        $generation = $this->generation;
        $this->setState(ConnectionState::Connecting);
        $this->deadline = Timers::timeout(15000, function () use ($generation): void {
            if ($generation === $this->generation) {
                $this->lost();
            }
        });
        $this->socket = WebSocket::connect($this->config->socketUrl, function (WebSocketEvent $event, string $text) use ($generation): void {
            if ($generation !== $this->generation || $this->closed || $this->suspended) {
                return;
            }
            if ($event === WebSocketEvent::Text) {
                try {
                    $this->received($text);
                } catch (Throwable) {
                    $this->lost();
                }
            } elseif ($event === WebSocketEvent::Closed || $event === WebSocketEvent::Failed) {
                $this->lost();
            }
        }, function () use ($generation): void {
            if ($generation === $this->generation) {
                $this->lost();
            }
        });
    }
    private function lost(): void
    {
        if ($this->closed || $this->suspended || $this->rejected) {
            return;
        }$this->stopTransport();
        $this->setState(ConnectionState::Reconnecting);
        $delay = min(30000, 1000 * (2 ** min($this->attempt++, 5))) + random_int(0, 250);
        $this->retry = Timers::timeout($delay, fn () => $this->connect());
    }
    private function reject(): void
    {
        $this->rejected = true;
        $this->stopTransport();
        $this->setState(ConnectionState::Rejected);
    }
    private function received(string $text): void
    {
        $frame = json_decode($text, true, 16, JSON_THROW_ON_ERROR);
        if (!is_array($frame) || !is_string($frame['event'] ?? null)) {
            throw new \UnexpectedValueException('Invalid realtime frame.');
        }
        $event = $frame['event'];
        $data = $frame['data'] ?? [];
        if (is_string($data)) {
            $data = json_decode($data, true, 16, JSON_THROW_ON_ERROR);
        }if (!is_array($data)) {
            throw new \UnexpectedValueException('Invalid realtime data.');
        }
        if ($event === 'pusher:connection_established') {
            $socketId = $data['socket_id'] ?? null;
            if (!is_string($socketId) || preg_match('/^[0-9]+\.[0-9]+$/D', $socketId) !== 1) {
                throw new \UnexpectedValueException('Invalid realtime connection.');
            }
            $timeout = $data['activity_timeout'] ?? 120;
            if (!is_int($timeout) || $timeout < 1 || $timeout > 300) {
                throw new \UnexpectedValueException('Invalid realtime timeout.');
            }$this->activityTimeout = $timeout * 1000;
            $this->authorize($socketId);
            return;
        }
        if ($event === 'pusher:ping') {
            $this->send(['event' => 'pusher:pong','data' => []]);
            $this->armHeartbeat();
            return;
        }
        if ($event === 'pusher:pong') {
            $this->deadline?->cancel();
            $this->deadline = null;
            $this->armHeartbeat();
            return;
        }
        if ($event === 'pusher:error') {
            $this->reject();
            return;
        }
        if (($frame['channel'] ?? null) !== $this->channel) {
            return;
        }
        if ($event === 'pusher_internal:subscription_succeeded') {
            if (!$this->authorized) {
                $this->reject();
                return;
            }$this->deadline?->cancel();
            $this->deadline = null;
            $this->attempt = 0;
            $this->setState(ConnectionState::Subscribed);
            $this->armHeartbeat();
            return;
        }
        if ($this->state !== ConnectionState::Subscribed || !in_array($event, $this->events, true)) {
            return;
        }
        $object = [];
        foreach ($data as $key => $value) {
            if (!is_string($key)) {
                throw new \UnexpectedValueException('Invalid event data.');
            }$object[$key] = $value;
        }
        $this->armHeartbeat();
        ($this->message)($event, $object);
    }
    private function authorize(string $socketId): void
    {
        if ($this->state !== ConnectionState::Connecting) {
            throw new \UnexpectedValueException('Unexpected realtime handshake.');
        }$this->setState(ConnectionState::Authorizing);
        $generation = $this->generation;
        $pending = $this->http->post($this->config->authorizationUrl, ['socket_id' => $socketId,'channel_name' => $this->channel]);
        $this->authorization = $pending;
        $pending->then(function (mixed $response) use ($generation): void {
            if ($generation !== $this->generation) {
                return;
            }if (!$response instanceof Response) {
                $this->reject();
                return;
            }$data = $response->json();
            $auth = is_array($data) ? ($data['auth'] ?? null) : null;
            if (!is_string($auth) || strlen($auth) > 512 || preg_match('/^[A-Za-z0-9_-]+:[a-fA-F0-9]{64}$/D', $auth) !== 1) {
                $this->reject();
                return;
            }$this->authorized = true;
            $this->send(['event' => 'pusher:subscribe','data' => ['auth' => $auth,'channel' => $this->channel]]);
        })->catch(function () use ($generation): void {
            if ($generation === $this->generation) {
                $this->reject();
            }
        });
    }
    /** @param array<string,mixed> $message */
    private function send(array $message): void
    {
        $this->socket?->send(json_encode($message, JSON_THROW_ON_ERROR), failed:fn () => $this->lost());
    }
    private function armHeartbeat(): void
    {
        $this->heartbeat?->cancel();
        $this->heartbeat = Timers::timeout($this->activityTimeout, function (): void {
            if ($this->closed || $this->suspended) {
                return;
            }$this->send(['event' => 'pusher:ping','data' => []]);
            $this->deadline?->cancel();
            $this->deadline = Timers::timeout(10000,fn () => $this->lost());
        });
    }
}
