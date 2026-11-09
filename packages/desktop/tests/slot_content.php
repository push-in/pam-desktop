<?php

declare(strict_types=1);

// Standalone runs use the project autoloader; tests/run.php brings its own.
if (!class_exists(\Pam\Desktop\Protocol::class)) {
    require_once dirname(__DIR__, 3).'/vendor/autoload.php';
}

use Pam\Desktop\App;
use Pam\Desktop\Component;
use Pam\Desktop\Renderable;
use Pam\Desktop\UI\Text;

$root = sys_get_temp_dir().'/pam-slot-content-'.bin2hex(random_bytes(6));
mkdir($root, 0700, true);
file_put_contents($root.'/Host.pam.php', <<<'PAM'
<?php
namespace SlotContentRegression;
final class Host extends \Pam\Desktop\Component
{
    public mixed $content = null;
}
?>
<template><Column><Slot :content="$content"><Text>Empty</Text></Slot></Column></template>
PAM);
file_put_contents($root.'/StyledHost.pam.php', <<<'PAM'
<?php
namespace SlotContentRegression;
final class StyledHost extends \Pam\Desktop\Component
{
    public mixed $content = null;
}
?>
<template><Column><Slot :content="$content" /></Column></template>
<style>
:root { -webkit-font-smoothing: antialiased; }
</style>
PAM);
try {
    App::components($root, $root.'/cache');
    $host = new SlotContentRegression\Host();
    $mounted = Pam\Desktop\Testing\mount($host);
    if ($mounted->text() !== 'Empty') throw new RuntimeException('Null content must render fallback.');
    $child = new class extends Component {
        public int $mounts = 0;
        public int $unmounts = 0;
        public function mount(): void { ++$this->mounts; }
        public function unmount(): void { ++$this->unmounts; }
        public function render(): Renderable { return Text::make('Active screen'); }
    };
    $host->content = $child;
    $mounted->render();
    if ($mounted->text() !== 'Active screen' || $child->mounts !== 1) throw new RuntimeException('Bound component must mount.');
    $mounted->render();
    if ($child->mounts !== 1) throw new RuntimeException('Stable content must retain lifecycle.');
    $host->content = null;
    $mounted->render();
    if ($mounted->text() !== 'Empty' || $child->unmounts !== 1) throw new RuntimeException('Replaced content must unmount.');
    $host->content = [Text::make('One'), Text::make('Two')];
    $mounted->render();
    if (!str_contains($mounted->text(), 'One') || !str_contains($mounted->text(), 'Two')) throw new RuntimeException('Renderable lists must render.');
    $mounted->unmount();
    // A component built in PHP and placed by <Slot :content> inherits the
    // text styles of where it renders (CSS inheritance), :root included.
    $styled = new SlotContentRegression\StyledHost();
    $placed = new class extends Component {
        public function render(): Renderable { return Text::make('Placed'); }
    };
    $styled->content = $placed;
    $styledMount = Pam\Desktop\Testing\mount($styled);
    if (($placed->__pamInheritedStyles()['fontSmoothing'] ?? null) !== 'antialiased') throw new RuntimeException('Slot content must inherit the text styles of its place.');
    $styledMount->unmount();
    echo "Bound slot content: fallback, lifecycle, replacement, lists and inherited text styles passed\n";
} finally {
    $items = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($root, FilesystemIterator::SKIP_DOTS), RecursiveIteratorIterator::CHILD_FIRST);
    foreach ($items as $item) $item->isDir() ? rmdir($item->getPathname()) : unlink($item->getPathname());
    rmdir($root);
}
