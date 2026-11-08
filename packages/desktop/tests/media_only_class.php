<?php

declare(strict_types=1);

require_once dirname(__DIR__, 3).'/vendor/autoload.php';

// A class declared only inside @media must be accepted by templates.
$directory = sys_get_temp_dir().'/pam-media-only-'.bin2hex(random_bytes(4));
mkdir($directory, 0700, true);
file_put_contents($directory.'/MediaOnly.pam.php', "<?php\nnamespace MediaOnlyRegression;\nfinal class MediaOnly extends \\Pam\\Desktop\\Component {}\n?>\n<template><Text class=\"narrow-only\">x</Text></template>\n<style scoped>\n@media (max-width: 900px) { .narrow-only { color: #FF0000; } }\n</style>\n");
try {
    \Pam\Desktop\App::components($directory, $directory.'/cache');
    if (\Pam\Desktop\Internal\PamPhpCompiler::failures() !== []) {
        throw new RuntimeException('Media-only component did not compile.');
    }
    $mounted = \Pam\Desktop\Testing\mount(new \MediaOnlyRegression\MediaOnly());
    $mounted->render();
    echo "Classes declared only inside @media are accepted: passed\n";
} finally {
    exec('rm -rf '.escapeshellarg($directory));
}
