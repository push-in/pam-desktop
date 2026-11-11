<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\KeyEvent;
use Pam\Desktop\LayoutEvent;
use Pam\DesktopUi\TreeNode;
use Pam\DesktopUi\TreeView;

$loader = require dirname(__DIR__, 3).'/vendor/autoload.php';
$loader->addPsr4('Pam\\DesktopUi\\', dirname(__DIR__).'/src');
App::components(dirname(__DIR__).'/src', sys_get_temp_dir().'/pam-desktop-tree-tests');
$fail = static fn (string $message) => throw new RuntimeException($message);

$loads = new ArrayObject();
$tree = new TreeView([
    new TreeNode('src', 'src', [
        new TreeNode('src/App.php', 'App.php', icon: 'file-code'),
        new TreeNode('src/lib', 'lib', static function () use ($loads): array {
            $loads[] = 1;

            return [new TreeNode('src/lib/a.php', 'a.php')];
        }),
    ], icon: 'folder'),
    new TreeNode('README.md', 'README.md'),
], expanded: ['src']);
$ids = static fn (): array => array_map(static fn ($row) => $row->node->id, $tree->rows());
$mounted = Pam\Desktop\Testing\mount($tree);
try {
    $mounted->render();
    $ids() === ['src', 'src/App.php', 'src/lib', 'README.md'] || $fail('Rows follow the open branches: '.json_encode($ids()));
    count($loads) === 0 || $fail('A closed lazy branch is not loaded.');
    $tree->press('src/lib');
    $visible = $ids();
    count($loads) === 1 && $visible[3] === 'src/lib/a.php' && $tree->selected === 'src/lib' || $fail('Pressing a branch opens and selects it: '.json_encode([count($loads), $ids(), $tree->selected, $tree->open]));
    $tree->rows()[3]->depth === 2 || $fail('Children are one level deeper.');
    $tree->press('src/lib');
    $tree->press('src/lib');
    count($loads) === 1 || $fail('Children load once.');

    $tree->measured(new LayoutEvent(0, 0, 200, 44));
    $tree->key(new KeyEvent('ArrowDown'));
    $tree->selected === 'src/lib/a.php' || $fail('Down selects the next row: '.$tree->selected);
    $tree->scrollRevision > 0 && $tree->scrollTarget === 88.0 - 44.0 || $fail('The selected row is scrolled into view: '.$tree->scrollTarget);
    $tree->key(new KeyEvent('ArrowLeft'));
    $tree->selected === 'src/lib' || $fail('Left on a leaf goes to its parent.');
    $tree->key(new KeyEvent('ArrowLeft'));
    !in_array('src/lib/a.php', $ids(), true) || $fail('Left on an open branch closes it.');
    $tree->key(new KeyEvent('ArrowRight'));
    in_array('src/lib/a.php', $ids(), true) || $fail('Right opens a closed branch.');
    $tree->key(new KeyEvent('ArrowRight'));
    $tree->selected === 'src/lib/a.php' || $fail('Right on an open branch goes to its first child.');
    $tree->key(new KeyEvent('End'));
    $tree->selected === 'README.md' || $fail('End goes to the last row.');
    $tree->key(new KeyEvent('Home'));
    $tree->selected === 'src' || $fail('Home goes to the first row.');
    $tree->collapseAll();
    $ids() === ['src', 'README.md'] || $fail('collapseAll closes every branch.');
    $mounted->render();
} finally {
    $mounted->unmount();
}
echo "TreeView: open branches, lazy children, selection, keyboard, scroll into view, collapse: passed\n";
