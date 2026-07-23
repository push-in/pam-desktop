<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Closure;
use InvalidArgumentException;
use JsonException;
use RuntimeException;
use Throwable;

final class Application
{
    public const PROTOCOL_VERSION = 1;
    public const BOOT_COMMAND = '@pam/boot';
    public const MAX_MESSAGE_BYTES = 1_048_576;

    /** @var array<string, Closure(CommandContext): mixed> */
    private array $commands = [];

    private function __construct(
        private readonly Window $window,
        private readonly string $entry,
    ) {
        if (
            $entry === ''
            || str_starts_with($entry, '/')
            || preg_match('~(^|[\\\\/])\.\.([\\\\/]|$)~', $entry) === 1
        ) {
            throw new InvalidArgumentException(
                'The desktop entry must be a relative path inside the project.',
            );
        }
    }

    public static function create(
        Window $window,
        string $entry = 'resources/index.html',
    ): self {
        return new self($window, $entry);
    }

    /**
     * @param Closure(CommandContext): mixed $handler
     */
    public function command(string $name, Closure $handler): self
    {
        if (preg_match('/^[a-z][a-z0-9._-]{0,63}$/i', $name) !== 1) {
            throw new InvalidArgumentException(
                'Command names must begin with a letter and contain at most 64 letters, numbers, dots, dashes, or underscores.',
            );
        }

        if (isset($this->commands[$name])) {
            throw new InvalidArgumentException("Command {$name} is already registered.");
        }

        $this->commands[$name] = $handler;

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

        return $this->dispatch($message);
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

        $command = $message['command'] ?? null;
        if (!is_string($command) || $command === '') {
            return $this->failure($id, ErrorCode::InvalidMessage, 'The IPC command is missing.');
        }

        if ($command === self::BOOT_COMMAND) {
            return $this->success($id, [
                'entry' => $this->entry,
                'window' => $this->window->toArray(),
            ]);
        }

        $handler = $this->commands[$command] ?? null;
        if ($handler === null) {
            return $this->failure(
                $id,
                ErrorCode::UnknownCommand,
                "The command {$command} is not registered.",
            );
        }

        try {
            $result = $handler(new CommandContext(
                id: $id,
                name: $command,
                payload: $message['payload'] ?? null,
            ));
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
        );
    }

    /**
     * @param list<Effect> $effects
     * @return array<string, mixed>
     */
    private function success(int $id, mixed $payload, array $effects = []): array
    {
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
        ];
    }

    /**
     * @return array<string, mixed>
     */
    private function failure(int $id, ErrorCode $code, string $message): array
    {
        if ($message === '') {
            throw new RuntimeException('Protocol errors must include a message.');
        }

        return [
            'version' => self::PROTOCOL_VERSION,
            'id' => $id,
            'kind' => MessageKind::Response->value,
            'status' => ResponseStatus::Failure->value,
            'payload' => null,
            'effects' => [],
            'error' => [
                'code' => $code->value,
                'message' => $message,
            ],
        ];
    }
}

