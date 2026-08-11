<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Closure;
use JsonException;
use RuntimeException;
use Throwable;

final class Application
{
    public const API_VERSION = 1;
    public const PROTOCOL_VERSION = 6;
    public const BOOT_COMMAND = '@pam/boot';
    public const EVENT_COMMAND = '@pam/event';
    public const JOB_COMMAND = '@pam/job';
    public const MAX_MESSAGE_BYTES = 1_048_576;
    public const MIN_COMMAND_TIMEOUT_MS = 100;
    public const MAX_COMMAND_TIMEOUT_MS = 120_000;

    /** @var array<string, Window> */
    private array $windows;

    /** @var array<string, Closure(CommandContext): mixed> */
    private array $commands = [];

    /** @var array<string, CommandExecution> */
    private array $commandExecutions = [];

    /** @var array<string, Closure(EventContext): mixed> */
    private array $eventHandlers = [];

    /** @var array<string, BackgroundJob> */
    private array $backgroundJobs = [];

    /** @var array<string, Closure(JobContext): mixed> */
    private array $jobHandlers = [];

    /** @var array<string, RustPlugin> */
    private array $rustPlugins = [];

    /** @var array<string, true> */
    private array $phpPlugins = [];

    private int $commandTimeoutMilliseconds = 30_000;

    private int $parallelWorkerCount = 2;

    private Capabilities $capabilities;

    private Shell $shell;

    private function __construct(
        private Manifest $manifest,
        Window $window,
    ) {
        $this->windows = ['main' => $window];
        $this->capabilities = Capabilities::none();
        $this->shell = Shell::none();
    }

    public static function create(Window $window, Manifest $manifest): self
    {
        return new self($manifest, $window);
    }

    public static function make(
        string $id,
        string $name,
        string $version = '1.0.0',
        ?Window $window = null,
    ): self {
        return new self(
            manifest: Manifest::create($id, $name, $version),
            window: $window ?? Window::create($name),
        );
    }

    public function description(string $description): self
    {
        $this->manifest = $this->manifest->description($description);

        return $this;
    }

    public function publisher(string $publisher): self
    {
        $this->manifest = $this->manifest->publisher($publisher);

        return $this;
    }

    public function category(ApplicationCategory $category): self
    {
        $this->manifest = $this->manifest->category($category);

        return $this;
    }

    public function icon(string $path): self
    {
        $this->manifest = $this->manifest->icon($path);

        return $this;
    }

    public function excludeFromBundle(string ...$paths): self
    {
        $this->manifest = $this->manifest->excludeFromBundle(...$paths);

        return $this;
    }

    public function updates(Updates $updates): self
    {
        $this->manifest = $this->manifest->updates($updates);

        return $this;
    }

    public function withoutUpdates(): self
    {
        $this->manifest = $this->manifest->withoutUpdates();

        return $this;
    }

    public function lifecycle(Lifecycle $lifecycle): self
    {
        $this->manifest = $this->manifest->lifecycle($lifecycle);

        return $this;
    }

    public function window(string $id, Window $window): self
    {
        Identifier::assert($id, 'The window identifier');
        if ($id === 'main') {
            throw new RuntimeException('The main window is configured through Application::create().');
        }
        if (isset($this->windows[$id])) {
            throw new RuntimeException("Window {$id} is already registered.");
        }

        $this->windows[$id] = $window;

        return $this;
    }

    public function commandTimeout(int $milliseconds): self
    {
        if (
            $milliseconds < self::MIN_COMMAND_TIMEOUT_MS
            || $milliseconds > self::MAX_COMMAND_TIMEOUT_MS
        ) {
            throw new RuntimeException(
                sprintf(
                    'The command timeout must be between %d and %d milliseconds.',
                    self::MIN_COMMAND_TIMEOUT_MS,
                    self::MAX_COMMAND_TIMEOUT_MS,
                ),
            );
        }

        $this->commandTimeoutMilliseconds = $milliseconds;

        return $this;
    }

    public function parallelWorkers(int $workers): self
    {
        if ($workers < 1 || $workers > 16) {
            throw new RuntimeException('The parallel worker count must be between 1 and 16.');
        }
        $this->parallelWorkerCount = $workers;

        return $this;
    }

    public function capabilities(Capabilities $capabilities): self
    {
        $this->capabilities = $capabilities;

        return $this;
    }

    public function shell(Shell $shell): self
    {
        $this->shell = $shell;

        return $this;
    }

    /**
     * @param Closure(JobContext): mixed $handler
     */
    public function job(string $id, BackgroundJob $job, Closure $handler): self
    {
        Identifier::assert($id, 'The background job identifier');
        if (isset($this->backgroundJobs[$id])) {
            throw new RuntimeException("Background job {$id} is already registered.");
        }
        $this->backgroundJobs[$id] = $job;
        $this->jobHandlers[$id] = $handler;

        return $this;
    }

    public function rustPlugin(RustPlugin $plugin): self
    {
        if (isset($this->rustPlugins[$plugin->id])) {
            throw new RuntimeException("Rust plugin {$plugin->id} is already registered.");
        }
        $this->rustPlugins[$plugin->id] = $plugin;

        return $this;
    }

    public function plugin(Plugin $plugin): self
    {
        $identifier = $plugin->identifier();
        Identifier::assert($identifier, 'The PHP plugin identifier');
        if (isset($this->phpPlugins[$identifier])) {
            throw new RuntimeException("PHP plugin {$identifier} is already registered.");
        }
        $this->phpPlugins[$identifier] = true;
        try {
            $plugin->register($this);
        } catch (Throwable $error) {
            unset($this->phpPlugins[$identifier]);
            throw $error;
        }

        return $this;
    }

    /**
     * @param Closure(CommandContext): mixed $handler
     */
    public function command(
        string $name,
        Closure $handler,
        CommandExecution $execution = CommandExecution::Stateful,
    ): self
    {
        Identifier::assert($name, 'The command name');
        if (isset($this->commands[$name])) {
            throw new RuntimeException("Command {$name} is already registered.");
        }

        $this->commands[$name] = $handler;
        $this->commandExecutions[$name] = $execution;

        return $this;
    }

    /**
     * @param Closure(EventContext): mixed $handler
     */
    public function on(string $name, Closure $handler): self
    {
        Identifier::assert($name, 'The event name');
        if (isset($this->eventHandlers[$name])) {
            throw new RuntimeException("Event {$name} already has a handler.");
        }

        $this->eventHandlers[$name] = $handler;

        return $this;
    }

    public function run(): never
    {
        while (($line = fgets(STDIN, self::MAX_MESSAGE_BYTES + 2)) !== false) {
            $response = $this->handleLine($line);
            fwrite(
                STDOUT,
                json_encode(
                    $response,
                    JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE,
                )."\n",
            );
            fflush(STDOUT);
        }

        exit(0);
    }

    /**
     * @return array<string, mixed>
     */
    public function handleLine(string $line): array
    {
        if (strlen($line) > self::MAX_MESSAGE_BYTES) {
            return $this->failure(
                id: 0,
                code: ErrorCode::InvalidMessage,
                message: 'The IPC message exceeds the one-megabyte limit.',
            );
        }

        try {
            $message = json_decode($line, true, flags: JSON_THROW_ON_ERROR);
        } catch (JsonException) {
            return $this->failure(0, ErrorCode::InvalidMessage, 'The IPC message is not valid JSON.');
        }

        if (!is_array($message)) {
            return $this->failure(0, ErrorCode::InvalidMessage, 'The IPC message must be an object.');
        }

        $object = [];
        foreach ($message as $key => $value) {
            if (!is_string($key)) {
                return $this->failure(
                    0,
                    ErrorCode::InvalidMessage,
                    'The IPC message must be a JSON object.',
                );
            }
            $object[$key] = $value;
        }

        return $this->dispatch($object);
    }

    /**
     * @param array<string, mixed> $message
     * @return array<string, mixed>
     */
    public function dispatch(array $message): array
    {
        $id = is_int($message['id'] ?? null) && $message['id'] > 0
            ? $message['id']
            : 0;

        if (($message['version'] ?? null) !== self::PROTOCOL_VERSION) {
            return $this->failure(
                $id,
                ErrorCode::UnsupportedProtocol,
                'The host and PHP worker use incompatible protocol versions.',
            );
        }

        if (($message['kind'] ?? null) !== MessageKind::Request->value || $id === 0) {
            return $this->failure($id, ErrorCode::InvalidMessage, 'The IPC request envelope is invalid.');
        }

        $windowId = $message['windowId'] ?? null;
        if (!is_string($windowId) || !isset($this->windows[$windowId])) {
            return $this->failure($id, ErrorCode::InvalidMessage, 'The source window is invalid.');
        }

        $command = $message['command'] ?? null;
        if (!is_string($command) || $command === '') {
            return $this->failure($id, ErrorCode::InvalidMessage, 'The IPC command is missing.');
        }

        if ($command === self::BOOT_COMMAND) {
            return $this->success($id, [
                'manifest' => $this->manifest->toArray(),
                'windows' => array_map(
                    static fn (string $windowId, Window $window): array => $window->toArray($windowId),
                    array_keys($this->windows),
                    array_values($this->windows),
                ),
                'commandTimeoutMs' => $this->commandTimeoutMilliseconds,
                'parallelWorkerCount' => $this->parallelWorkerCount,
                'commands' => array_map(
                    static fn (string $name, CommandExecution $execution): array => [
                        'name' => $name,
                        'execution' => $execution->value,
                    ],
                    array_keys($this->commandExecutions),
                    array_values($this->commandExecutions),
                ),
                'capabilities' => $this->capabilities->toArray(),
                'shell' => $this->shell->toArray(),
                'backgroundJobs' => array_map(
                    static fn (string $id, BackgroundJob $job): array => $job->toArray($id),
                    array_keys($this->backgroundJobs),
                    array_values($this->backgroundJobs),
                ),
                'rustPlugins' => array_map(
                    static fn (RustPlugin $plugin): array => $plugin->toArray(),
                    array_values($this->rustPlugins),
                ),
                'phpPlugins' => array_keys($this->phpPlugins),
            ]);
        }

        if ($command === self::EVENT_COMMAND) {
            return $this->dispatchEvent($id, $windowId, $message['payload'] ?? null);
        }

        if ($command === self::JOB_COMMAND) {
            return $this->dispatchJob($id, $message['payload'] ?? null);
        }

        $handler = $this->commands[$command] ?? null;
        if ($handler === null) {
            return $this->failure(
                $id,
                ErrorCode::UnknownCommand,
                "The command {$command} is not registered.",
            );
        }

        return $this->invoke(
            id: $id,
            handler: $handler,
            context: new CommandContext(
                id: $id,
                name: $command,
                windowId: $windowId,
                payload: $message['payload'] ?? null,
            ),
        );
    }

    /**
     * @return array<string, mixed>
     */
    private function dispatchEvent(int $id, string $windowId, mixed $payload): array
    {
        if (!is_array($payload) || !is_string($payload['name'] ?? null)) {
            return $this->failure($id, ErrorCode::InvalidPayload, 'The event envelope is invalid.');
        }

        $name = $payload['name'];
        $handler = $this->eventHandlers[$name] ?? null;
        if ($handler === null) {
            return $this->failure(
                $id,
                ErrorCode::UnknownEvent,
                "The event {$name} has no registered handler.",
            );
        }

        return $this->invoke(
            id: $id,
            handler: $handler,
            context: new EventContext(
                id: $id,
                name: $name,
                windowId: $windowId,
                payload: $payload['payload'] ?? null,
            ),
        );
    }

    /**
     * @return array<string, mixed>
     */
    private function dispatchJob(int $id, mixed $payload): array
    {
        if (
            !is_array($payload)
            || !is_string($payload['id'] ?? null)
            || !is_int($payload['runId'] ?? null)
            || !is_int($payload['startedAtMs'] ?? null)
        ) {
            return $this->failure(
                $id,
                ErrorCode::InvalidPayload,
                'The background job envelope is invalid.',
            );
        }

        $jobId = $payload['id'];
        $handler = $this->jobHandlers[$jobId] ?? null;
        if ($handler === null) {
            return $this->failure(
                $id,
                ErrorCode::BackgroundJobFailed,
                "Background job {$jobId} has no registered handler.",
            );
        }

        return $this->invoke(
            id: $id,
            handler: $handler,
            context: new JobContext(
                requestId: $id,
                id: $jobId,
                runId: $payload['runId'],
                startedAtMilliseconds: $payload['startedAtMs'],
            ),
        );
    }

    /**
     * @param Closure(CommandContext): mixed|Closure(EventContext): mixed|Closure(JobContext): mixed $handler
     * @return array<string, mixed>
     */
    private function invoke(
        int $id,
        Closure $handler,
        CommandContext|EventContext|JobContext $context,
    ): array
    {
        try {
            $result = $handler($context);
        } catch (Throwable $error) {
            return $this->failure($id, ErrorCode::HandlerFailed, $error->getMessage());
        }

        if (!$result instanceof CommandResult) {
            $result = CommandResult::success($result);
        }

        return $this->success(
            id: $id,
            payload: $result->payload,
            effects: $result->effects,
            events: $result->events,
        );
    }

    /**
     * @param list<Effect> $effects
     * @param list<ClientEvent> $events
     * @return array<string, mixed>
     */
    private function success(
        int $id,
        mixed $payload,
        array $effects = [],
        array $events = [],
    ): array {
        return [
            'version' => self::PROTOCOL_VERSION,
            'id' => $id,
            'kind' => MessageKind::Response->value,
            'status' => ResponseStatus::Success->value,
            'payload' => $payload,
            'effects' => array_map(
                static fn (Effect $effect): array => $effect->toArray(),
                $effects,
            ),
            'events' => array_map(
                static fn (ClientEvent $event): array => $event->toArray(),
                $events,
            ),
        ];
    }

    /**
     * @return array<string, mixed>
     */
    private function failure(int $id, ErrorCode $code, string $message): array
    {
        if ($message === '') {
            $message = 'The PHP handler failed without an error message.';
        }

        return [
            'version' => self::PROTOCOL_VERSION,
            'id' => $id,
            'kind' => MessageKind::Response->value,
            'status' => ResponseStatus::Failure->value,
            'payload' => null,
            'effects' => [],
            'events' => [],
            'error' => [
                'code' => $code->value,
                'message' => $message,
            ],
        ];
    }
}
