<?php

// Finds which shared PHP extensions an application refers to. `pam-desktop
// build` runs it with the PHP it bundles:
//
//   php -r <this file> -- <application directory> <extension>...
//
// and reads `{"used": [...]}` from its output. An extension counts as used
// when a source file names one of its functions, classes or constants, uses
// a PDO data source or a stream wrapper it provides, or when a `composer.json`
// requires it. Every doubt keeps the extension: leaving out one that is
// needed breaks the application on the machine of its user. `openssl` and
// `zlib` are always kept: `https://` and compressed streams use them without
// naming them.
//
// Packages Composer installed for development only are not searched.

error_reporting(0);
ini_set('memory_limit', '-1');
set_time_limit(0);

$root = $argv[1] ?? '';
$candidates = array_map('strtolower', array_slice($argv, 2));
if ($root === '' || !is_dir($root) || !function_exists('token_get_all')) {
    echo json_encode(['all' => true]);
    exit(0);
}

$functions = [];
$classes = [];
$constants = [];
$requires = [];
$used = [];
foreach ($candidates as $name) {
    try {
        $extension = new ReflectionExtension($name);
    } catch (Throwable) {
        // Not loaded here, so listed by hand: nothing to look for.
        $used[$name] = true;
        continue;
    }
    foreach (array_keys($extension->getFunctions()) as $function) {
        $functions[strtolower($function)][] = $name;
    }
    foreach ($extension->getClassNames() as $class) {
        $classes[strtolower($class)][] = $name;
    }
    foreach (array_keys($extension->getConstants()) as $constant) {
        $constants[$constant][] = $name;
    }
    foreach ($extension->getDependencies() as $dependency => $kind) {
        if ($kind === 'Required') {
            $requires[$name][] = strtolower($dependency);
        }
    }
}
$names = array_flip($candidates);
foreach (['openssl', 'zlib'] as $implicit) {
    if (isset($names[$implicit])) {
        $used[$implicit] = true;
    }
}
// `PDO` is often compiled into the binary, so it is tracked apart from the candidates.
$pdo = false;

// With PAM_DESKTOP_EXTENSION_USAGE=1 the first reference found to each
// extension is printed to the error output.
$explain = getenv('PAM_DESKTOP_EXTENSION_USAGE') === '1';
$file = '';
$mark = static function (array $extensions, string $why = '') use (&$used, &$file, $explain): void {
    foreach ($extensions as $extension) {
        if ($explain && !isset($used[$extension])) {
            fwrite(STDERR, "{$extension}: {$why} in {$file}\n");
        }
        $used[$extension] = true;
    }
};

/** A name that is a function, a class or a constant of the global scope. */
$global = static function (string $name, bool $class = true) use ($functions, $classes, $constants, $mark, &$pdo): void {
    $name = ltrim($name, '\\');
    $lower = strtolower($name);
    $mark($functions[$lower] ?? [], "function {$name}");
    $mark($constants[$name] ?? [], "constant {$name}");
    if ($class) {
        $mark($classes[$lower] ?? [], "class {$name}");
        $pdo = $pdo || $lower === 'pdo' || $lower === 'pdostatement';
    }
};

/** What a string literal may stand for: a callable, a data source, a stream wrapper. */
$literal = static function (string $text) use ($global, $mark): void {
    if (preg_match('/^(sqlite|mysql|pgsql|odbc|dblib|firebird|oci|sqlsrv):/i', $text, $match)) {
        $mark(['pdo_'.strtolower($match[1])], "data source {$match[0]}");
    }
    foreach (['phar://' => 'phar', 'zip://' => 'zip', 'compress.bzip2://' => 'bz2', 'ssh2.' => 'ssh2', 'convert.iconv' => 'iconv'] as $marker => $extension) {
        if (str_contains($text, $marker)) {
            $mark([$extension], "stream {$marker}");
        }
    }
    // A callable: `'mb_strtolower'` or `'Normalizer::normalize'`. A bare class
    // or extension name in a string is a guard (`class_exists`,
    // `extension_loaded`) of code that works without it, or plain data.
    if (strlen($text) < 3 || strlen($text) > 120 || !preg_match('/^\\\\?[A-Za-z_][A-Za-z0-9_\\\\]*(::[A-Za-z_][A-Za-z0-9_]*)?$/', $text)) {
        return;
    }
    $name = explode('::', $text)[0];
    // Only `Class::method` names a class.
    $global($name, $name !== $text);
};

$scanPhp = static function (string $code) use ($global, $literal): void {
    $tokens = token_get_all($code);
    $count = count($tokens);
    $namespaced = false;
    $aliases = [];
    $previous = null;
    for ($index = 0; $index < $count; $index++) {
        $token = $tokens[$index];
        $id = is_array($token) ? $token[0] : $token;
        $text = is_array($token) ? $token[1] : $token;
        if ($id === T_WHITESPACE || $id === T_COMMENT || $id === T_DOC_COMMENT || $id === T_INLINE_HTML) {
            continue;
        }
        $before = $previous;
        $previous = $id;

        if ($id === T_NAMESPACE) {
            // `namespace Name;`, `namespace Name {`, `namespace {`; not the `namespace\name` operator.
            for ($next = $index + 1; $next < $count && is_array($tokens[$next]) && $tokens[$next][0] === T_WHITESPACE; $next++);
            $following = $tokens[$next] ?? null;
            if (is_array($following) && in_array($following[0], [T_STRING, T_NAME_QUALIFIED], true)) {
                $namespaced = true;
                $aliases = [];
                $index = $next;
            } elseif ($following === '{') {
                $namespaced = false;
                $aliases = [];
            }
            continue;
        }

        if ($id === T_USE && $before !== ')') {
            // An import (or the traits of a class): every name is taken as fully qualified.
            $prefix = '';
            $name = '';
            $alias = false;
            for ($index++; $index < $count; $index++) {
                $part = $tokens[$index];
                $partId = is_array($part) ? $part[0] : $part;
                $partText = is_array($part) ? $part[1] : $part;
                if (in_array($partId, [T_STRING, T_NAME_QUALIFIED, T_NAME_FULLY_QUALIFIED], true)) {
                    if ($alias) {
                        $aliases[strtolower($partText)] = $name;
                        $alias = false;
                        continue;
                    }
                    $name = $prefix.ltrim($partText, '\\');
                    $global($name);
                    $short = strrchr($name, '\\');
                    $aliases[strtolower($short === false ? $name : substr($short, 1))] = $name;
                } elseif ($partId === T_AS) {
                    $alias = true;
                } elseif ($partId === T_NS_SEPARATOR) {
                    // `use Prefix\{A, B};`
                    $prefix = $name.'\\';
                } elseif ($partId === '}') {
                    $prefix = '';
                } elseif ($partId === ';') {
                    break;
                }
            }
            $previous = ';';
            continue;
        }

        if ($id === T_CONSTANT_ENCAPSED_STRING) {
            $literal(str_replace('\\\\', '\\', substr($text, 1, -1)));
            continue;
        }
        if ($id === T_ENCAPSED_AND_WHITESPACE) {
            $literal($text);
            continue;
        }
        if (!in_array($id, [T_STRING, T_NAME_QUALIFIED, T_NAME_FULLY_QUALIFIED], true)) {
            continue;
        }
        // Members and declarations are not references to the global scope.
        if (in_array($before, [T_OBJECT_OPERATOR, T_NULLSAFE_OBJECT_OPERATOR, T_DOUBLE_COLON, T_FUNCTION, T_CONST, T_CLASS, T_INTERFACE, T_TRAIT, T_ENUM, T_GOTO], true)) {
            continue;
        }
        if ($id === T_NAME_FULLY_QUALIFIED) {
            $global($text);
            continue;
        }
        $first = strtolower(explode('\\', $text)[0]);
        if (isset($aliases[$first])) {
            // `Alias` or `Alias\Rest` of an import.
            $rest = strstr($text, '\\');
            $global($aliases[$first].($rest === false ? '' : $rest));
        }
        if ($id === T_STRING) {
            // Functions and constants fall back to the global scope; classes do not.
            $global($text, !$namespaced);
        } elseif (!$namespaced) {
            $global($text);
        }
    }
};

/** Templates and other sources: any word that is a known name. */
$scanWords = static function (string $text) use ($global, $literal): void {
    preg_match_all('/\\\\?[A-Za-z_][A-Za-z0-9_\\\\]*/', $text, $matches);
    foreach (array_unique($matches[0]) as $word) {
        $global($word);
    }
    preg_match_all('/[\'"]([^\'"\n]{1,120})[\'"]/', $text, $matches);
    foreach (array_unique($matches[1]) as $string) {
        $literal($string);
    }
};

$composer = static function (string $path) use ($mark, $names, &$file): void {
    $file = $path;
    $manifest = json_decode((string) file_get_contents($path), true);
    foreach (array_keys(is_array($manifest) && is_array($manifest['require'] ?? null) ? $manifest['require'] : []) as $requirement) {
        $requirement = strtolower((string) $requirement);
        if (str_starts_with($requirement, 'ext-') && isset($names[substr($requirement, 4)])) {
            $mark([substr($requirement, 4)], "requirement {$requirement}");
        }
    }
};

$seen = [];
$walk = static function (string $directory, array $skip) use (&$walk, &$seen, &$file, $scanPhp, $scanWords, $composer): void {
    // Linked packages are followed, each directory once.
    $real = realpath($directory);
    $entries = $real === false || isset($seen[$real]) ? false : scandir($directory);
    if ($entries === false) {
        return;
    }
    $seen[$real] = true;
    if (is_file($directory.'/composer/installed.json')) {
        // A Composer vendor directory: development packages do not run in the bundle.
        $installed = json_decode((string) file_get_contents($directory.'/composer/installed.json'), true);
        foreach (is_array($installed) && is_array($installed['dev-package-names'] ?? null) ? $installed['dev-package-names'] : [] as $package) {
            $skip[$directory.'/'.$package] = true;
        }
    }
    foreach ($entries as $entry) {
        if ($entry === '.' || $entry === '..' || $entry === '.git' || $entry === 'node_modules') {
            continue;
        }
        $path = $directory.'/'.$entry;
        if (isset($skip[$path])) {
            continue;
        }
        if (is_dir($path)) {
            $walk($path, $skip);
            continue;
        }
        if ($entry === 'composer.json') {
            $composer($path);
            continue;
        }
        $extension = strtolower(pathinfo($entry, PATHINFO_EXTENSION));
        if (!in_array($extension, ['php', 'pam', 'phtml', 'inc'], true) || filesize($path) > 8 * 1024 * 1024) {
            continue;
        }
        $file = $path;
        $source = (string) file_get_contents($path);
        if ($extension === 'pam') {
            $scanWords($source);
        } else {
            $scanPhp($source);
        }
    }
};
$walk(rtrim($root, '/\\'), []);

// A PDO whose data source is not written in the sources (it comes from
// configuration) may use any driver.
$drivers = array_filter($candidates, static fn (string $name): bool => str_starts_with($name, 'pdo_'));
if (($pdo || isset($used['pdo'])) && array_intersect($drivers, array_keys($used)) === []) {
    $mark($drivers);
}

// What a used extension cannot load without.
do {
    $before = count($used);
    foreach (array_keys($used) as $name) {
        $mark(array_intersect($requires[$name] ?? [], $candidates), "required by {$name}, as");
    }
} while (count($used) !== $before);

echo json_encode(['used' => array_values(array_intersect($candidates, array_keys($used)))]);
