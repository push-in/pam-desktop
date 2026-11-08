<?php

declare(strict_types=1);
// Standalone runs use the project autoloader; tests/run.php brings its own.
if (!class_exists(\Pam\Desktop\Protocol::class)) {
    require_once dirname(__DIR__, 3).'/vendor/autoload.php';
}

$directory = sys_get_temp_dir().'/pam-text-color-'.bin2hex(random_bytes(6));
mkdir($directory);
$source = <<<'SOURCE'
<?php
namespace PamExplicitColorTest;
final class ColorText extends \Pam\Desktop\Component {
    public string $color = '#34D399';
}
?>
<template><Text :textColor="$color">Status</Text></template>
<style scoped>Text { color: #E7EDF5; }</style>
SOURCE;
file_put_contents($directory.'/ColorText.pam.php', $source);
try {
    \Pam\Desktop\App::components($directory, $directory.'/cache');
    $component = new \PamExplicitColorTest\ColorText();
    foreach (['#34D399' => 0xFF34D399, '#FBBF24' => 0xFFFBBF24] as $color => $expected) {
        $component->color = $color;
        $properties = $component->toElement()->properties();
        if (($properties[\Pam\Desktop\PropKey::TextColor->value] ?? null) !== $expected) throw new \RuntimeException('Explicit bound textColor must override scoped CSS color on every render.');
    }
} finally {
    foreach (glob($directory.'/cache/*') ?: [] as $file) unlink($file);
    if (is_dir($directory.'/cache')) rmdir($directory.'/cache');
    unlink($directory.'/ColorText.pam.php'); rmdir($directory);
}
echo "Bound textColor overrides scoped color across renders.\n";
