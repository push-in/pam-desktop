<?php

declare(strict_types=1);

namespace Pam\Desktop\Diagnostics;

use CompileError;
use ErrorException;
use Pam\Desktop\Internal\PamPhpRegistry;
use Pam\Desktop\TemplateException;
use Throwable;

use function array_any;
use function array_key_last;
use function array_splice;
use function class_exists;
use function count;
use function is_array;
use function is_string;
use function strlen;

/**
 * Structured, path-shortened description of an uncaught runtime error. It is
 * what the native error overlay renders and what App::onError() listeners
 * (crash reporters such as Sentry) receive next to the original Throwable.
 */
final readonly class RuntimeError
{
    private const int MAX_FRAMES = 64;
    private const int SNIPPET_CONTEXT = 3;
    private const int MAX_SOURCE_BYTES = 2_097_152;
    private const int MAX_SNIPPET_LINE = 240;

    /**
     * @param list<StackFrame> $frames
     * @param array{file: string, line: int, start: int, lines: list<string>}|null $snippet
     */
    public function __construct(
        public string $type,
        public string $message,
        public string $file,
        public int $line,
        public int $column,
        public RuntimeErrorPhase $phase,
        public array $frames,
        public ?int $appFrame,
        public ?array $snippet,
        public string $fingerprint,
    ) {
    }

    public function fatal(): bool
    {
        return $this->phase->fatal();
    }

    public static function from(
        Throwable $error,
        RuntimeErrorPhase $phase,
        bool $withSnippet = true,
    ): self {
        // A template reports where it failed; the error an expression or a
        // component raised inside it says what failed and where it came from.
        $template = $error instanceof TemplateException ? $error : null;
        $origin = $template?->getPrevious() ?? $error;
        [$frames, $paths] = self::frames($origin, $template);
        if ($origin instanceof ErrorException && ($origin->getSeverity() & (E_ERROR | E_CORE_ERROR | E_COMPILE_ERROR)) !== 0) {
            // A PHP fatal error has no stack: only where it stopped.
            $path = $origin->getFile();
            $compiled = class_exists(PamPhpRegistry::class, false) ? PamPhpRegistry::sourceOf($path) : null;
            $path = $compiled !== null && !$compiled[1] ? $compiled[0] : $path;
            $frames = [self::frame($path, $origin->getLine(), 'fatal error')];
            $paths = [$path];
        } elseif ($origin instanceof CompileError && $frames !== []) {
            // The file that does not compile, not the autoloader that read it.
            $frames[0] = new StackFrame($frames[0]->file, $frames[0]->line, 'compile', $frames[0]->kind);
        }
        // Raised by the framework while reading the template: a template error.
        $type = $origin::class;
        if ($template !== null && !array_any($frames, static fn (StackFrame $frame): bool => $frame->call === 'template')) {
            // A template error starts at the template. An error raised inside
            // it: after the app frames that raised it, before the renderer.
            $at = $origin === $error ? 0 : count($frames);
            foreach ($origin === $error ? [] : $frames as $index => $frame) {
                if ($frame->kind !== StackFrame::APP) {
                    $at = $index;
                    break;
                }
            }
            if ($at === 0) {
                $type = TemplateException::class;
            }
            array_splice($frames, $at, 0, [self::templateFrame($template)]);
            array_splice($paths, $at, 0, [$template->template]);
        }
        $appFrame = null;
        foreach ($frames as $index => $frame) {
            if ($frame->kind === StackFrame::APP && $frame->file !== null) {
                $appFrame = $index;
                break;
            }
        }
        $location = $appFrame !== null ? $frames[$appFrame] : null;
        $atTemplate = $template !== null && $location?->call === 'template';
        $file = $location?->file ?? self::shortenPath($origin->getFile());
        $line = $location?->line ?? $origin->getLine();
        $column = $atTemplate ? $template->templateColumn : 1;
        $message = $origin === $template ? $template->reason : $origin->getMessage();
        $snippet = $withSnippet && $location !== null && $location->file !== null
            ? self::snippet($paths[$appFrame] ?? null, $location->file, $location->line)
            : null;

        return new self(
            type: $type,
            message: $message,
            file: $file,
            line: $line,
            column: $column,
            phase: $phase,
            frames: $frames,
            appFrame: $appFrame,
            snippet: $snippet,
            fingerprint: hash('xxh3', $type."\0".$message."\0".$file."\0".$line),
        );
    }

    /**
     * Strips release bundle prefixes (`<...>/files/pam/releases/<hash>/`)
     * and the project root, so frames read like `app/Screens/Chat.php`.
     */
    public static function shortenPath(string $path): string
    {
        $path = self::portablePath($path);
        $short = preg_replace('#^.*?/files/pam/(?:ota-)?releases/[^/]+/#', '', $path, 1) ?? $path;
        if ($short !== $path) {
            return $short;
        }
        $windows = DIRECTORY_SEPARATOR === '\\';
        foreach (self::projectRoots() as $root) {
            $prefix = rtrim($root, '/').'/';
            if ($windows ? strncasecmp($path, $prefix, strlen($prefix)) === 0 : str_starts_with($path, $prefix)) {
                return substr($path, strlen($prefix));
            }
        }

        return $path;
    }

    /** @return array<string, mixed> */
    public function toArray(): array
    {
        return [
            'type' => $this->type,
            'message' => $this->message,
            'file' => $this->file,
            'line' => $this->line,
            'column' => $this->column,
            'phase' => $this->phase->value,
            'fatal' => $this->fatal(),
            'frames' => array_map(static fn (StackFrame $frame): array => $frame->toArray(), $this->frames),
            'appFrame' => $this->appFrame,
            'snippet' => $this->snippet,
            'fingerprint' => $this->fingerprint,
        ];
    }

    /** Human-readable trace with shortened paths, like Throwable::getTraceAsString(). */
    public function traceAsString(): string
    {
        $lines = [];
        foreach ($this->frames as $index => $frame) {
            $location = $frame->file === null ? '[internal function]' : $frame->file.'('.$frame->line.')';
            $lines[] = '#'.$index.' '.$location.': '.$frame->call;
        }

        return implode("\n", $lines);
    }

    /** @return array{0: list<StackFrame>, 1: list<?string>} frames and their absolute paths */
    private static function frames(Throwable $error, ?TemplateException $template): array
    {
        $frames = [];
        $paths = [];
        $file = $error->getFile();
        $line = $error->getLine();
        $add = static function (?string $file, int $line, string $call) use (&$frames, &$paths, $template): void {
            $compiled = $file !== null && class_exists(PamPhpRegistry::class, false)
                ? PamPhpRegistry::sourceOf($file)
                : null;
            if ($compiled !== null) {
                [$source, $expressions] = $compiled;
                if ($expressions) {
                    // Compiled template expressions: one frame for the template.
                    $last = $frames === [] ? null : $frames[array_key_last($frames)];
                    if ($last?->call === 'template' && $paths[array_key_last($paths)] === $source) {
                        return;
                    }
                    $frames[] = $template !== null && $template->template === $source
                        ? self::templateFrame($template)
                        : new StackFrame(self::shortenPath($source), 0, 'template', StackFrame::APP);
                    $paths[] = $source;

                    return;
                }
                $file = $source;
            }
            $frames[] = self::frame($file, $line, $call);
            $paths[] = $file;
        };
        foreach ($error->getTrace() as $entry) {
            $add($file, $line, self::call($entry));
            if (count($frames) >= self::MAX_FRAMES) {
                return [$frames, $paths];
            }
            $file = isset($entry['file']) ? (string) $entry['file'] : null;
            $line = (int) ($entry['line'] ?? 0);
        }
        $add($file, $line, '{main}');

        return [$frames, $paths];
    }

    private static function templateFrame(TemplateException $template): StackFrame
    {
        return new StackFrame(self::shortenPath($template->template), $template->templateLine, 'template', StackFrame::APP);
    }

    private static function frame(?string $file, int $line, string $call): StackFrame
    {
        if ($file === null || $file === '') {
            return new StackFrame(null, 0, $call, StackFrame::INTERNAL);
        }
        $kind = StackFrame::APP;
        // The package: its sources and the worker bootstrap in bin/.
        $package = self::portablePath(dirname(__DIR__, 2));
        $file = self::portablePath($file);
        if (str_starts_with($file, $package.'/src/') || str_starts_with($file, $package.'/bin/')
            || str_contains($file, '/vendor/pushinbr/pam-desktop')) {
            $kind = StackFrame::FRAMEWORK;
        } elseif (str_contains($file, '/vendor/') || str_starts_with($file, 'vendor/')) {
            $kind = StackFrame::VENDOR;
        }

        return new StackFrame(self::shortenPath($file), $line, $call, $kind);
    }

    /** @param array{function: string, class?: class-string, type?: '->'|'::'} $entry */
    private static function call(array $entry): string
    {
        $function = $entry['function'];
        $class = $entry['class'] ?? '';
        $type = $entry['type'] ?? '';

        $call = ($class !== '' ? $class.$type : '').$function.'()';
        // Anonymous classes and closures embed NUL bytes and absolute paths.
        $call = preg_replace('/@anonymous\x00[^$]*\$[0-9a-f]+/', '@anonymous', $call) ?? $call;

        return preg_replace('/\{closure:[^{}]*\}/', '{closure}', $call) ?? $call;
    }

    /** @return array{file: string, line: int, start: int, lines: list<string>}|null */
    private static function snippet(?string $path, string $file, int $line): ?array
    {
        if ($path === null || $line < 1 || !is_file($path) || !is_readable($path)) {
            return null;
        }
        $size = @filesize($path);
        if ($size === false || $size > self::MAX_SOURCE_BYTES) {
            return null;
        }
        $source = @file($path, FILE_IGNORE_NEW_LINES);
        if ($source === false || $line > count($source)) {
            return null;
        }
        $start = max(1, $line - self::SNIPPET_CONTEXT);
        $end = min(count($source), $line + self::SNIPPET_CONTEXT);
        $lines = [];
        for ($number = $start; $number <= $end; $number++) {
            $lines[] = self::clip(rtrim($source[$number - 1]));
        }

        return ['file' => $file, 'line' => $line, 'start' => $start, 'lines' => $lines];
    }

    /** Clips long source lines without needing ext-mbstring. */
    private static function clip(string $line): string
    {
        if (strlen($line) <= self::MAX_SNIPPET_LINE) {
            return $line;
        }

        return preg_match('/^.{0,'.self::MAX_SNIPPET_LINE.'}/su', $line, $match) === 1
            ? $match[0].'…'
            : substr($line, 0, self::MAX_SNIPPET_LINE).'…';
    }

    /** @return list<string> */
    private static function projectRoots(): array
    {
        $roots = [];
        $script = $_SERVER['SCRIPT_FILENAME'] ?? null;
        if (is_string($script) && preg_match('#^(?:/|[A-Za-z]:/)#', self::portablePath($script)) === 1) {
            $roots[] = self::portablePath(dirname($script));
        }
        $cwd = getcwd();
        if ($cwd !== false && preg_match('#^(?:/|[A-Za-z]:/?)$#', self::portablePath($cwd)) !== 1) {
            $roots[] = self::portablePath($cwd);
        }

        $roots = array_values(array_unique($roots));
        // The outermost root keeps the most context (`app/Chat.php`, not `Chat.php`).
        usort($roots, static fn (string $a, string $b): int => strlen($a) <=> strlen($b));

        return $roots;
    }

    /** Forward slashes on Windows, so paths compare and print alike everywhere. */
    private static function portablePath(string $path): string
    {
        return DIRECTORY_SEPARATOR === '\\' ? str_replace('\\', '/', $path) : $path;
    }
}
