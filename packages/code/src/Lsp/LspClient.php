<?php

declare(strict_types=1);

namespace Pam\Desktop\Code\Lsp;

use Closure;
use InvalidArgumentException;
use JsonException;
use Pam\Desktop\System\Timers;
use RuntimeException;
use Throwable;

/**
 * A Language Server Protocol client: starts a server (rust-analyzer,
 * intelephense, typescript-language-server, pylsp…) as a child process and
 * talks JSON-RPC to it over stdio.
 *
 * ```php
 * $lsp = LspClient::start(['rust-analyzer'], $root, fn (string $path, array $diagnostics) => $this->show($diagnostics));
 * $lsp->didOpen($path, 'rust', $text);
 * $lsp->hover($path, $line, $character, fn (?string $markdown) => $this->tooltip = $markdown);
 * ```
 *
 * The server's output is read by a timer: every 15 ms while an answer is
 * awaited or the server just spoke, every 250 ms otherwise. `stop()` ends
 * the server; it also ends with the application.
 */
final class LspClient
{
    /** @var resource|null */
    private $process = null;
    /** @var array<int, resource> */
    private array $pipes = [];
    private string $buffer = '';
    private int $nextId = 1;
    /** @var array<int, array{0: Closure, 1: ?Closure}> */
    private array $pending = [];
    /** @var list<string> frames written before the server answered `initialize` */
    private array $queued = [];
    private bool $initialized = false;
    /** @var array<string, int> document versions by path */
    private array $versions = [];
    private bool $scheduled = false;
    private float $lastActivity = 0.0;
    /** @var array<string, mixed> */
    public array $capabilities = [];

    /**
     * @param list<string> $command the server and its arguments
     * @param (Closure(string, list<Diagnostic>): void)|null $onDiagnostics
     */
    private function __construct(
        private readonly string $root,
        private readonly ?Closure $onDiagnostics,
        private readonly bool $autoPump,
    ) {
    }

    /**
     * @param list<string> $command
     * @param (Closure(string, list<Diagnostic>): void)|null $onDiagnostics a file's diagnostics changed
     * @param bool $autoPump read the server's output on a timer (false: call `pump()` yourself)
     */
    public static function start(array $command, string $root, ?Closure $onDiagnostics = null, bool $autoPump = true): self
    {
        if ($command === [] || array_filter($command, static fn (mixed $part): bool => !is_string($part)) !== []) {
            throw new InvalidArgumentException('An LSP command is a list of strings.');
        }
        $client = new self(rtrim($root, '/'), $onDiagnostics, $autoPump);
        $process = proc_open($command, [0 => ['pipe', 'r'], 1 => ['pipe', 'w'], 2 => ['file', PHP_OS_FAMILY === 'Windows' ? 'NUL' : '/dev/null', 'a']], $pipes, $root);
        if (!is_resource($process)) {
            throw new RuntimeException('The language server '.$command[0].' could not start.');
        }
        stream_set_blocking($pipes[1], false);
        $client->process = $process;
        $client->pipes = $pipes;
        $client->send([
            'jsonrpc' => '2.0',
            'id' => $client->nextId,
            'method' => 'initialize',
            'params' => [
                'processId' => getmypid(),
                'rootUri' => self::uri($client->root),
                'workspaceFolders' => [['uri' => self::uri($client->root), 'name' => basename($client->root)]],
                'capabilities' => [
                    'textDocument' => [
                        'synchronization' => ['didSave' => false],
                        'hover' => ['contentFormat' => ['markdown', 'plaintext']],
                        'completion' => ['completionItem' => ['snippetSupport' => false]],
                        'definition' => [],
                        'publishDiagnostics' => [],
                    ],
                    'workspace' => ['workspaceFolders' => true, 'configuration' => true],
                ],
            ],
        ], true);
        $client->pending[$client->nextId++] = [static function (mixed $result) use ($client): void {
            $client->capabilities = is_array($result['capabilities'] ?? null) ? $result['capabilities'] : [];
            $client->initialized = true;
            $client->send(['jsonrpc' => '2.0', 'method' => 'initialized', 'params' => new \stdClass()], true);
            foreach ($client->queued as $frame) {
                $client->write($frame);
            }
            $client->queued = [];
        }, null];
        $client->schedule();

        return $client;
    }

    public function running(): bool
    {
        return is_resource($this->process) && (proc_get_status($this->process)['running'] ?? false);
    }

    public function didOpen(string $path, string $languageId, string $text): void
    {
        $this->versions[$path] = 1;
        $this->notify('textDocument/didOpen', ['textDocument' => [
            'uri' => $this->uriOf($path), 'languageId' => $languageId, 'version' => 1, 'text' => $text,
        ]]);
    }

    /** The whole new text of an open document. */
    public function didChange(string $path, string $text): void
    {
        $version = $this->versions[$path] = ($this->versions[$path] ?? 0) + 1;
        $this->notify('textDocument/didChange', [
            'textDocument' => ['uri' => $this->uriOf($path), 'version' => $version],
            'contentChanges' => [['text' => $text]],
        ]);
    }

    public function didClose(string $path): void
    {
        unset($this->versions[$path]);
        $this->notify('textDocument/didClose', ['textDocument' => ['uri' => $this->uriOf($path)]]);
    }

    /** @param Closure(?string): void $done the hover text (Markdown), or null */
    public function hover(string $path, int $line, int $character, Closure $done): int
    {
        return $this->request('textDocument/hover', $this->position($path, $line, $character), static function (mixed $result) use ($done): void {
            $contents = is_array($result) ? ($result['contents'] ?? null) : null;
            $done(match (true) {
                is_string($contents) => $contents,
                is_array($contents) && isset($contents['value']) => (string) $contents['value'],
                is_array($contents) => implode("\n\n", array_map(
                    static fn (mixed $part): string => is_array($part) ? (isset($part['language']) ? "```{$part['language']}\n{$part['value']}\n```" : (string) ($part['value'] ?? '')) : (string) $part,
                    $contents,
                )),
                default => null,
            });
        });
    }

    /** @param Closure(list<CompletionItem>): void $done */
    public function completion(string $path, int $line, int $character, Closure $done): int
    {
        return $this->request('textDocument/completion', $this->position($path, $line, $character), static function (mixed $result) use ($done): void {
            $items = is_array($result) ? ($result['items'] ?? $result) : [];
            $done(array_values(array_map(static fn (array $item): CompletionItem => CompletionItem::fromLsp($item), array_filter($items, 'is_array'))));
        });
    }

    /** @param Closure(list<Location>): void $done */
    public function definition(string $path, int $line, int $character, Closure $done): int
    {
        return $this->request('textDocument/definition', $this->position($path, $line, $character), function (mixed $result) use ($done): void {
            $locations = is_array($result) && isset($result['uri']) ? [$result] : (is_array($result) ? $result : []);
            $done(array_values(array_map(fn (array $location): Location => new Location(
                $this->pathOf((string) ($location['uri'] ?? $location['targetUri'] ?? '')),
                (int) (($location['range'] ?? $location['targetSelectionRange'] ?? [])['start']['line'] ?? 0),
                (int) (($location['range'] ?? $location['targetSelectionRange'] ?? [])['start']['character'] ?? 0),
            ), array_filter($locations, 'is_array'))));
        });
    }

    /**
     * @param array<string, mixed>|list<mixed> $params
     * @param Closure(mixed): void $done the result
     * @param (Closure(string): void)|null $failed the server's error message
     */
    public function request(string $method, array $params, Closure $done, ?Closure $failed = null): int
    {
        $id = $this->nextId++;
        $this->pending[$id] = [$done, $failed];
        $this->send(['jsonrpc' => '2.0', 'id' => $id, 'method' => $method, 'params' => $params]);

        return $id;
    }

    /** @param array<string, mixed> $params */
    public function notify(string $method, array $params): void
    {
        $this->send(['jsonrpc' => '2.0', 'method' => $method, 'params' => $params]);
    }

    /** Asks the server to shut down and ends it. */
    public function stop(): void
    {
        if (!is_resource($this->process)) {
            return;
        }
        if ($this->initialized) {
            $this->write(self::frame(['jsonrpc' => '2.0', 'id' => $this->nextId++, 'method' => 'shutdown']));
            $this->write(self::frame(['jsonrpc' => '2.0', 'method' => 'exit']));
        }
        foreach ($this->pipes as $pipe) {
            if (is_resource($pipe)) {
                fclose($pipe);
            }
        }
        proc_terminate($this->process);
        proc_close($this->process);
        $this->process = null;
        $this->pending = [];
    }

    public function __destruct()
    {
        $this->stop();
    }

    /**
     * Reads what the server wrote and dispatches it. `$waitMs` blocks up to
     * that long for output (0: never blocks). Returns the messages handled.
     */
    public function pump(int $waitMs = 0): int
    {
        if (!is_resource($this->pipes[1] ?? null)) {
            return 0;
        }
        $read = [$this->pipes[1]];
        $write = $except = null;
        if (@stream_select($read, $write, $except, 0, $waitMs * 1000) > 0) {
            while (($chunk = fread($this->pipes[1], 65536)) !== false && $chunk !== '') {
                $this->buffer .= $chunk;
            }
        }
        $handled = 0;
        while (($message = $this->nextMessage()) !== null) {
            $this->dispatch($message);
            ++$handled;
        }
        if ($handled > 0) {
            $this->lastActivity = microtime(true);
        }

        return $handled;
    }

    /** @return array<string, mixed>|null */
    private function nextMessage(): ?array
    {
        $end = strpos($this->buffer, "\r\n\r\n");
        if ($end === false || preg_match('/Content-Length:\s*(\d+)/i', substr($this->buffer, 0, $end), $header) !== 1) {
            return null;
        }
        $length = (int) $header[1];
        if (strlen($this->buffer) < $end + 4 + $length) {
            return null;
        }
        $body = substr($this->buffer, $end + 4, $length);
        $this->buffer = substr($this->buffer, $end + 4 + $length);
        try {
            $message = json_decode($body, true, 512, JSON_THROW_ON_ERROR);
        } catch (JsonException) {
            return [];
        }

        return is_array($message) ? $message : [];
    }

    /** @param array<string, mixed> $message */
    private function dispatch(array $message): void
    {
        if (isset($message['id']) && !isset($message['method'])) {
            [$done, $failed] = $this->pending[(int) $message['id']] ?? [null, null];
            unset($this->pending[(int) $message['id']]);
            if (isset($message['error'])) {
                $failed?->__invoke((string) ($message['error']['message'] ?? 'Language server error'));
            } else {
                $done?->__invoke($message['result'] ?? null);
            }

            return;
        }
        $method = (string) ($message['method'] ?? '');
        if (isset($message['id'])) {
            // A request from the server: answer what a simple client answers.
            $result = match ($method) {
                'workspace/configuration' => array_fill(0, count($message['params']['items'] ?? []), null),
                default => null,
            };
            $this->send(['jsonrpc' => '2.0', 'id' => $message['id'], 'result' => $result], true);

            return;
        }
        if ($method === 'textDocument/publishDiagnostics' && $this->onDiagnostics !== null) {
            $path = $this->pathOf((string) ($message['params']['uri'] ?? ''));
            $diagnostics = array_values(array_map(
                static fn (array $raw): Diagnostic => Diagnostic::fromLsp($path, $raw),
                array_filter($message['params']['diagnostics'] ?? [], 'is_array'),
            ));
            ($this->onDiagnostics)($path, $diagnostics);
        }
    }

    /** @param array<string, mixed> $message */
    private function send(array $message, bool $now = false): void
    {
        $frame = self::frame($message);
        if ($now || $this->initialized) {
            $this->write($frame);
        } else {
            $this->queued[] = $frame;
        }
        $this->schedule();
    }

    private function write(string $frame): void
    {
        if (is_resource($this->pipes[0] ?? null)) {
            @fwrite($this->pipes[0], $frame);
        }
    }

    /** Arms the read timer: fast while answers are awaited or the server is busy. */
    private function schedule(): void
    {
        if (!$this->autoPump || $this->scheduled || !is_resource($this->process)) {
            return;
        }
        $busy = $this->pending !== [] || microtime(true) - $this->lastActivity < 1.0;
        $this->scheduled = true;
        try {
            Timers::after($busy ? 15 : 250, function (): void {
                $this->scheduled = false;
                try {
                    $this->pump();
                } catch (Throwable) {
                }
                if ($this->running()) {
                    $this->schedule();
                }
            });
        } catch (Throwable) {
            $this->scheduled = false;
        }
    }

    /** @param array<string, mixed> $message */
    private static function frame(array $message): string
    {
        $json = json_encode($message, JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_THROW_ON_ERROR);

        return 'Content-Length: '.strlen($json)."\r\n\r\n".$json;
    }

    /** @return array<string, mixed> */
    private function position(string $path, int $line, int $character): array
    {
        return ['textDocument' => ['uri' => $this->uriOf($path)], 'position' => ['line' => $line, 'character' => $character]];
    }

    private function uriOf(string $path): string
    {
        return self::uri(str_starts_with($path, '/') ? $path : $this->root.'/'.$path);
    }

    private function pathOf(string $uri): string
    {
        $path = rawurldecode((string) preg_replace('#^file://#', '', $uri));

        return str_starts_with($path, $this->root.'/') ? substr($path, strlen($this->root) + 1) : $path;
    }

    private static function uri(string $path): string
    {
        return 'file://'.implode('/', array_map('rawurlencode', explode('/', $path)));
    }
}
