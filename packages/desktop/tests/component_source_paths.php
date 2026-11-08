<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Internal\PamPhpCompiler;

require dirname(__DIR__, 3).'/vendor/autoload.php';

$root = sys_get_temp_dir().'/pam-desktop-source-paths-'.bin2hex(random_bytes(6));
mkdir($root.'/src/Screens', 0700, true);
file_put_contents($root.'/composer.json', '{}');
file_put_contents($root.'/src/Screens/SourcePaths.pam.php', <<<'PAM'
<?php
namespace SourcePathRegression;
final class SourcePaths extends \Pam\Desktop\Component
{
    public const FILE = __FILE__;
    public string $directory = __DIR__;
    public function __construct(public string $defaultPath = __DIR__.'/fixture.json') {}
    public function literal(): string { return '__DIR__ and __FILE__'; }
}
?>
<template><Text>Source paths</Text></template>
PAM);
$cache = $root.'/cache';
try {
    if (PamPhpCompiler::prebuild($root, $root.'/src') !== 1) throw new RuntimeException('Expected one prebuilt component.');
    $installed = $root.'-installed';
    rename($root, $installed);
    $root = $installed;
    App::components($root.'/src', $root.'/cache');
    $component = App::make(SourcePathRegression\SourcePaths::class);
    $directory = $root.'/src/Screens';
    if ($component->directory !== $directory || $component->defaultPath !== $directory.'/fixture.json'
        || $component::FILE !== $directory.'/SourcePaths.pam.php') {
        throw new RuntimeException('Magic constants did not follow the installed component source.');
    }
    if ($component->literal() !== '__DIR__ and __FILE__') throw new RuntimeException('String literal was rewritten.');
    echo "Component source paths, constant/default expressions and relocated prebuilt bundle: passed\n";
} finally {
    $items = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($root, FilesystemIterator::SKIP_DOTS), RecursiveIteratorIterator::CHILD_FIRST);
    foreach ($items as $item) $item->isDir() ? rmdir($item->getPathname()) : unlink($item->getPathname());
    rmdir($root);
}
