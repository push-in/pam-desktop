<?php

// Precompiles the components of a bundled application (`pam-desktop build`):
//   php -r <this> -- <bundled project>
// The pack goes to PamPhpCompiler::PREBUILT_DIRECTORY and manifest.sha256
// marks the staged bundle, so an installed application boots from the pack
// instead of compiling every component at its first launch. A project
// without the PHP package, or with one that cannot prebuild, is left as is.

error_reporting(E_ALL & ~E_DEPRECATED);
// Compiled the way the installed application runs (production, strict):
// prebuilt components record the mode and a bundle running in another one
// compiles every component again at its first launch.
putenv('PAM_DESKTOP_MODE=production');
ini_set('memory_limit', '-1');
set_time_limit(0);

$arguments = array_slice($argv, 1);
if (($arguments[0] ?? null) === '--') {
    array_shift($arguments);
}
$project = realpath($arguments[0] ?? '');
if ($project === false || !is_dir($project)) {
    exit(64);
}
for ($directory = $project; !is_file($directory.'/vendor/autoload.php'); $directory = $parent) {
    $parent = dirname($directory);
    if ($parent === $directory) {
        exit(0);
    }
}
require $directory.'/vendor/autoload.php';
$compiler = 'Pam\Desktop\Internal\PamPhpCompiler';
if (!class_exists($compiler) || !method_exists($compiler, 'prebuild') || !defined($compiler.'::PREBUILT_PACK')) {
    exit(0);
}
$count = $compiler::prebuild($project, $project);
if ($count > 0) {
    $pack = $compiler::PREBUILT_DIRECTORY.'/'.$compiler::PREBUILT_PACK;
    file_put_contents($project.'/manifest.sha256', hash_file('sha256', $project.'/'.$pack).'  '.$pack."\n");
}
echo $count;
