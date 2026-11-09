<?php

declare(strict_types=1);

namespace Pam\Desktop\Tooling;

use FilesystemIterator;
use Pam\Desktop\App;
use Pam\Desktop\Internal\CssDiagnostic;
use Pam\Desktop\Internal\PamPhpRegistry;
use Pam\Desktop\Routing\RouteCompiler;
use Throwable;

/**
 * `pam-desktop check [--json] [--src=<dir>]...`: compiles every component
 * template and stylesheet of the application and validates the template
 * contracts and routes, without the native host or a window. Every
 * diagnostic is collected (not only the first) and printed as
 * `file:line:col CODE message`.
 *
 * The components are found the way the application finds them: its entry
 * file (pam-desktop.json "entry", else index.php) runs up to App::run(), so
 * each App::components() root it declares is checked; `--src` checks the
 * given directories instead. Compiled output goes to a private temporary
 * cache, never to the application's own.
 *
 * Codes: PAM2401 template contract (unknown action/prop/event/attribute),
 * PAM2402 template or component compile error, PAM2403 invalid CSS,
 * PAM2404 route error, PAM2400 entry file error.
 */
final class ProjectCheck
{
    private static bool $active = false;

    private static ?string $cacheRoot = null;

    private static string $project = '';

    /** @var array<string, array{file: string, line: int, column: int, code: string, message: string}> */
    private static array $diagnostics = [];

    /** @var array<string, int> components compiled by source root */
    private static array $roots = [];

    private function __construct()
    {
    }

    /** @internal */
    public static function active(): bool
    {
        return self::$active;
    }

    /**
     * Private cache used instead of the application's component cache.
     *
     * @internal
     */
    public static function componentRoot(string $sourcePath, string $cachePath): string
    {
        return self::$cacheRoot.DIRECTORY_SEPARATOR.substr(hash('sha256', $cachePath), 0, 16);
    }

    /** @internal */
    public static function compiled(string $sourcePath, int $count): void
    {
        self::$roots[realpath($sourcePath) ?: $sourcePath] = $count;
    }

    /** @internal */
    public static function finish(mixed $root = null): never
    {
        throw new ProjectCheckFinished($root);
    }

    /** @internal */
    public static function report(Throwable $error, ?string $source = null, ?string $code = null): void
    {
        $diagnostic = self::diagnostic($error, $source, $code);
        $key = implode("\0", $diagnostic);
        self::$diagnostics[$key] ??= $diagnostic;
    }

    /**
     * @param list<string> $arguments the arguments after `check`
     * @return int exit status: 0 clean, 1 diagnostics, 2 usage
     */
    public static function run(array $arguments, string $project): int
    {
        $json = false;
        $sources = [];
        $entry = null;
        foreach ($arguments as $argument) {
            if ($argument === '--json') {
                $json = true;
            } elseif (str_starts_with($argument, '--src=') && strlen($argument) > 6) {
                $sources[] = substr($argument, 6);
            } elseif (str_starts_with($argument, '--entry=') && strlen($argument) > 8) {
                $entry = substr($argument, 8);
            } else {
                fwrite(STDERR, "Usage: pam-desktop check [--json] [--src=<dir>]... [--entry=<file>]\n");

                return 2;
            }
        }

        $project = realpath($project) ?: $project;
        self::$project = rtrim($project, DIRECTORY_SEPARATOR);
        self::$diagnostics = [];
        self::$roots = [];
        self::$cacheRoot = sys_get_temp_dir().DIRECTORY_SEPARATOR.'pam-desktop-check-'.getmypid().'-'.bin2hex(random_bytes(4));
        self::$active = true;
        $previousDirectory = getcwd();
        @chdir($project);
        ob_start();
        try {
            if ($sources !== []) {
                foreach ($sources as $source) {
                    $path = self::absolute($source);
                    try {
                        PamPhpRegistry::discover($path, self::$project.'/.pam-desktop/components');
                    } catch (Throwable $error) {
                        self::report($error, $path);
                    }
                }
            } else {
                self::runEntry(self::absolute($entry ?? self::configuredEntry()));
            }
            self::checkRoutes($sources === [] ? self::$project.'/src' : self::absolute($sources[0]));
        } finally {
            ob_end_clean();
            self::$active = false;
            self::removeTree(self::$cacheRoot);
            if (is_string($previousDirectory)) {
                @chdir($previousDirectory);
            }
        }

        $diagnostics = array_values(self::$diagnostics);
        usort($diagnostics, static fn (array $a, array $b): int => [$a['file'], $a['line'], $a['column'], $a['code']]
            <=> [$b['file'], $b['line'], $b['column'], $b['code']]);
        $components = array_sum(self::$roots);
        if ($json) {
            fwrite(STDOUT, json_encode([
                'ok' => $diagnostics === [],
                'components' => $components,
                'roots' => array_map(self::relative(...), array_keys(self::$roots)),
                'diagnostics' => $diagnostics,
            ], JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_INVALID_UTF8_SUBSTITUTE)."\n");
        } else {
            foreach ($diagnostics as $diagnostic) {
                fwrite(STDOUT, "{$diagnostic['file']}:{$diagnostic['line']}:{$diagnostic['column']} {$diagnostic['code']} {$diagnostic['message']}\n");
            }
            $plural = static fn (int $count, string $noun): string => $count.' '.$noun.($count === 1 ? '' : 's');
            $checked = $plural($components, 'component').' in '.$plural(count(self::$roots), 'root');
            fwrite(STDOUT, $diagnostics === []
                ? "pam-desktop check: {$checked}, no diagnostics.\n"
                : 'pam-desktop check: '.$plural(count($diagnostics), 'diagnostic')." ({$checked}).\n");
        }

        return $diagnostics === [] ? 0 : 1;
    }

    private static function configuredEntry(): string
    {
        $configuration = self::$project.'/pam-desktop.json';
        if (is_file($configuration)) {
            $decoded = json_decode((string) file_get_contents($configuration), true);
            if (is_array($decoded) && is_string($decoded['entry'] ?? null) && $decoded['entry'] !== '') {
                return $decoded['entry'];
            }
        }

        return 'index.php';
    }

    private static function runEntry(string $entry): void
    {
        if (is_file($entry)) {
            try {
                (static function (string $__entry): void {
                    require $__entry;
                })($entry);
            } catch (ProjectCheckFinished) {
            } catch (Throwable $error) {
                self::report($error, $entry, 'PAM2400');
            }
        }
        // An entry without App::components() (or none at all): the
        // conventional <project>/src root App::run() would register.
        if (self::$roots === []) {
            try {
                App::discoverConventionalComponents(self::$project);
            } catch (Throwable $error) {
                self::report($error, self::$project.'/src');
            }
        }
    }

    private static function checkRoutes(string $source): void
    {
        $routesFile = self::$project.'/routes.php';
        if (!is_file($routesFile)) {
            return;
        }
        try {
            $manifest = RouteCompiler::manifest(
                RouteCompiler::load($routesFile),
                RouteCompiler::componentDiscovery($source, self::$project.'/.pam-desktop/components'),
            );
            $base = rtrim($source, DIRECTORY_SEPARATOR);
            $errors = RouteCompiler::check($manifest, $source, [$base.'/Navigation/Generated/']);
        } catch (Throwable $error) {
            self::report($error, $routesFile, 'PAM2404');

            return;
        }
        foreach ($errors as $error) {
            if (preg_match('/^(.+?):(\d+): (.*)$/s', $error, $match) === 1) {
                self::add(self::relative($base.'/'.$match[1]), (int) $match[2], 1, 'PAM2404', $match[3]);
            } else {
                self::add(self::relative($routesFile), 1, 1, 'PAM2404', $error);
            }
        }
    }

    /** @return array{file: string, line: int, column: int, code: string, message: string} */
    private static function diagnostic(Throwable $error, ?string $source, ?string $code): array
    {
        $message = trim($error->getMessage());
        if (preg_match('/^(PAM\d{4})\s+(.*)$/s', $message, $match) === 1) {
            $code ??= $match[1];
            $message = $match[2];
        }
        $css = false;
        for ($cause = $error; $cause !== null; $cause = $cause->getPrevious()) {
            $css = $css || $cause instanceof CssDiagnostic;
        }
        $code ??= $css ? 'PAM2403' : 'PAM2402';
        $file = null;
        $line = 1;
        $column = 1;
        if (preg_match('/^(.*?)\s+at\s+(\S[^\n]*?):(\d+):(\d+)\.?$/s', $message, $match) === 1) {
            [, $message, $file, $line, $column] = $match;
        } elseif (preg_match('/^((?:[A-Za-z]:)?[^:\s][^:\n]*?):(\d+)(?::(\d+))?: (.*)$/s', $message, $match) === 1) {
            $file = $match[1];
            $line = $match[2];
            $column = $match[3] !== '' ? $match[3] : 1;
            $message = $match[4];
        } elseif (!str_starts_with($error->getFile(), dirname(__DIR__)) && is_file($error->getFile())) {
            $file = $error->getFile();
            $line = $error->getLine();
        }
        $file ??= $source ?? $error->getFile();
        $file = self::locateFile($file, $source);
        // Absolute paths inside the message read like the location.
        $message = str_replace(self::$project.DIRECTORY_SEPARATOR, '', $message);

        return [
            'file' => self::relative($file),
            'line' => max(1, (int) $line),
            'column' => max(1, (int) $column),
            'code' => $code,
            'message' => rtrim(preg_replace('/\s+/', ' ', $message) ?? $message, '.'),
        ];
    }

    /**
     * A path relative to the Composer project of the component (how the
     * style compiler names files) found from the component upwards.
     */
    private static function locateFile(string $file, ?string $source): string
    {
        if (str_starts_with($file, '/') || preg_match('/^[A-Za-z]:[\\\/]/', $file) === 1
            || is_file(self::$project.DIRECTORY_SEPARATOR.$file) || $source === null) {
            return $file;
        }
        for ($directory = dirname($source); ; $directory = dirname($directory)) {
            if (is_file($directory.DIRECTORY_SEPARATOR.$file)) {
                return $directory.DIRECTORY_SEPARATOR.$file;
            }
            if (dirname($directory) === $directory) {
                return $file;
            }
        }
    }

    private static function add(string $file, int $line, int $column, string $code, string $message): void
    {
        $diagnostic = ['file' => $file, 'line' => $line, 'column' => $column, 'code' => $code, 'message' => rtrim($message, '.')];
        self::$diagnostics[implode("\0", $diagnostic)] ??= $diagnostic;
    }

    private static function absolute(string $path): string
    {
        $absolute = str_starts_with($path, '/') || preg_match('/^[A-Za-z]:[\\\\\/]/', $path) === 1
            ? $path
            : self::$project.DIRECTORY_SEPARATOR.$path;

        return realpath($absolute) ?: $absolute;
    }

    private static function relative(string $path): string
    {
        if (!str_starts_with($path, '/') && preg_match('/^[A-Za-z]:[\\\\\/]/', $path) !== 1) {
            $resolved = realpath(self::$project.DIRECTORY_SEPARATOR.$path);
            if ($resolved === false) {
                return $path;
            }
            $path = $resolved;
        }
        $path = realpath($path) ?: $path;
        $prefix = self::$project.DIRECTORY_SEPARATOR;

        return str_starts_with($path, $prefix) ? substr($path, strlen($prefix)) : $path;
    }

    private static function removeTree(?string $directory): void
    {
        if ($directory === null || !is_dir($directory) || is_link($directory)) {
            return;
        }
        foreach (new FilesystemIterator($directory, FilesystemIterator::SKIP_DOTS) as $entry) {
            $path = $entry->getPathname();
            if ($entry->isDir() && !$entry->isLink()) {
                self::removeTree($path);
            } else {
                @unlink($path);
            }
        }
        @rmdir($directory);
    }
}
