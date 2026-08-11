<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final readonly class RustPlugin
{
    /**
     * @param list<string> $arguments
     */
    private function __construct(
        public string $id,
        public string $executable,
        public array $arguments,
        public int $timeoutMilliseconds,
        public ?string $sha256,
    ) {
        Identifier::assert($id, 'The Rust plugin identifier');
        self::assertProjectPath($executable);
        if (count($arguments) > 32) {
            throw new InvalidArgumentException('Rust plugins cannot declare more than 32 arguments.');
        }
        foreach ($arguments as $argument) {
            if (strlen($argument) > 1_024 || str_contains($argument, "\0")) {
                throw new InvalidArgumentException('A Rust plugin process argument is invalid.');
            }
        }
        if (
            $timeoutMilliseconds < Application::MIN_COMMAND_TIMEOUT_MS
            || $timeoutMilliseconds > Application::MAX_COMMAND_TIMEOUT_MS
        ) {
            throw new InvalidArgumentException(
                sprintf(
                    'Rust plugin timeouts must be between %d and %d milliseconds.',
                    Application::MIN_COMMAND_TIMEOUT_MS,
                    Application::MAX_COMMAND_TIMEOUT_MS,
                ),
            );
        }
    }

    public static function executable(string $id, string $path): self
    {
        return new self($id, $path, [], 30_000, null);
    }

    public function arguments(string ...$arguments): self
    {
        return new self(
            $this->id,
            $this->executable,
            array_values($arguments),
            $this->timeoutMilliseconds,
            $this->sha256,
        );
    }

    public function timeout(int $milliseconds): self
    {
        return new self(
            $this->id,
            $this->executable,
            $this->arguments,
            $milliseconds,
            $this->sha256,
        );
    }

    public function integrity(string $sha256): self
    {
        if (preg_match('/\A[0-9a-f]{64}\z/D', $sha256) !== 1) {
            throw new InvalidArgumentException('Rust plugin integrity must be a lowercase SHA-256 digest.');
        }

        return new self(
            $this->id,
            $this->executable,
            $this->arguments,
            $this->timeoutMilliseconds,
            $sha256,
        );
    }

    /**
     * @return array{id: string, executable: string, arguments: list<string>, timeoutMs: int, sha256: null|string}
     */
    public function toArray(): array
    {
        return [
            'id' => $this->id,
            'executable' => $this->executable,
            'arguments' => $this->arguments,
            'timeoutMs' => $this->timeoutMilliseconds,
            'sha256' => $this->sha256,
        ];
    }

    private static function assertProjectPath(string $path): void
    {
        if (
            $path === ''
            || str_contains($path, "\0")
            || str_starts_with($path, '/')
            || str_starts_with($path, '\\')
            || preg_match('/\A[A-Za-z]:[\\\\\/]/D', $path) === 1
            || preg_match('~(^|[\\\\/])\.\.([\\\\/]|$)~', $path) === 1
            || preg_match('~\A(?:\.git|\.pam|dist|node_modules|target)(?:[\\\\/]|$)~D', $path) === 1
        ) {
            throw new InvalidArgumentException(
                'Rust plugin executables must use a bundled project-relative path.',
            );
        }
    }
}
