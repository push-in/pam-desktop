<?php

declare(strict_types=1);

use Pam\Desktop\Internal\PamPhpCompiler;

// Standalone runs use the project autoloader; tests/run.php brings its own.
if (!class_exists(\Pam\Desktop\Internal\PamPhpCompiler::class)) {
    require dirname(__DIR__, 3).'/vendor/autoload.php';
}

// A broken component must not take the others down, and symlinked folders
// (path repositories, shared component folders) are followed once, but only
// into the project, its vendor/ or a path repository its composer.json declares.
$root = sys_get_temp_dir().'/pam-desktop-isolation-'.bin2hex(random_bytes(6));
$shared = $root.'-shared';
mkdir($root.'/src', 0700, true);
mkdir($shared, 0700, true);
file_put_contents($root.'/src/Good.pam.php', "<?php\nnamespace IsolationRegression;\nfinal class Good extends \\Pam\\Desktop\\Component {}\n?>\n<template><Text>ok</Text></template>\n");
file_put_contents($root.'/src/Broken.pam.php', "<?php\nnamespace IsolationRegression;\nfinal class Broken extends \\Pam\\Desktop\\Component {}\n?>\n<template><Column><Text p-for=\"broken\">x</Text></Column></template>\n");
file_put_contents($shared.'/Linked.pam.php', "<?php\nnamespace IsolationRegression;\nfinal class Linked extends \\Pam\\Desktop\\Component {}\n?>\n<template><Text>linked</Text></template>\n");
$outside = $root.'-outside';
mkdir($outside, 0700, true);
file_put_contents($outside.'/Stolen.pam.php', "<?php\nnamespace IsolationRegression;\nfinal class Stolen extends \\Pam\\Desktop\\Component {}\n?>\n<template><Text>no</Text></template>\n");
file_put_contents($root.'/composer.json', json_encode(['repositories' => [['type' => 'path', 'url' => '../'.basename($shared)]]]));
symlink($shared, $root.'/src/shared');
symlink($root.'/src', $root.'/src/shared-loop');
try {
    $components = PamPhpCompiler::compileDirectory($root.'/src', $root.'/cache');
    $classes = array_map(static fn ($component): string => $component->className, $components);
    sort($classes);
    $failures = PamPhpCompiler::failures();
    if ($classes !== ['IsolationRegression\\Good', 'IsolationRegression\\Linked']) {
        throw new RuntimeException('Healthy and symlinked components must compile: '.implode(', ', $classes));
    }
    if (count($failures) !== 1 || !str_ends_with((string) array_key_first($failures), 'Broken.pam.php')) {
        throw new RuntimeException('The broken component must be reported on its own.');
    }
    // Links out of the project that no path repository declares are refused,
    // a folder (vendor-like or not) and a single file alike.
    foreach ([[$outside, $root.'/src/vendor'], [$outside.'/Stolen.pam.php', $root.'/src/Stolen.pam.php']] as [$target, $link]) {
        symlink($target, $link);
        try {
            PamPhpCompiler::compileDirectory($root.'/src', $root.'/cache');
            throw new LogicException("A link to {$target} must be refused.");
        } catch (RuntimeException $error) {
            if (!str_contains($error->getMessage(), 'outside the project')) {
                throw $error;
            }
        } finally {
            unlink($link);
        }
    }
    echo "Component isolation and symlinked component folders: passed\n";
} finally {
    foreach ([$root, $shared, $outside] as $directory) {
        if (is_link($root.'/src/shared')) unlink($root.'/src/shared');
        if (is_link($root.'/src/shared-loop')) unlink($root.'/src/shared-loop');
        $items = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($directory, FilesystemIterator::SKIP_DOTS), RecursiveIteratorIterator::CHILD_FIRST);
        foreach ($items as $item) $item->isDir() && !$item->isLink() ? rmdir($item->getPathname()) : unlink($item->getPathname());
        rmdir($directory);
    }
}
