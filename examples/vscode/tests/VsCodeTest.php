<?php

declare(strict_types=1);

use App\Domain\FileKind;
use App\Screens\Workbench\WorkbenchScreen;
use App\Services\Highlighter;
use App\Services\Workspace;
use Pam\Desktop\App;

use function Pam\Desktop\Testing\mount;

$root = dirname(__DIR__);
require $root.'/vendor/autoload.php';
chdir($root);

App::components($root.'/src', $root.'/.pam-desktop/components');

$check = static function (bool $condition, string $message): void {
    if (!$condition) {
        fwrite(STDERR, "FAIL {$message}\n");
        exit(1);
    }
};

// Highlighting: PHP through its tokenizer, a template's markup by pattern.
$lines = Highlighter::lines('a.php', "<?php\n// note\n\$total = count(\$items);\n");
$colours = static fn (array $line): array => array_column($line, 1);
$check(count($lines) === 4, 'one entry per line');
$check($colours($lines[1]) === [Highlighter::COMMENT], 'comments are green');
$check(in_array(Highlighter::VARIABLE, $colours($lines[2]), true) && in_array(Highlighter::FUNCTION, $colours($lines[2]), true), 'variables and calls have their colours');
$template = Highlighter::lines('b.pam.php', "<?php ?>\n<Text class=\"title\">{{ \$name }}</Text>\n");
$check(in_array(Highlighter::TAG, $colours($template[1]), true) && in_array(Highlighter::STRING, $colours($template[1]), true), 'template tags and attribute values are coloured');
$check(implode('', array_column(Highlighter::lines('c.css', ".a { color: #FFF; }")[0], 0)) === '.a { color: #FFF; }', 'highlighting keeps every character');

// The explorer reads this project from disk, folders first.
$workspace = new Workspace($root);
$names = array_map(static fn ($entry): string => $entry->name, $workspace->entries());
$check($names[0] === 'assets' && in_array('README.md', $names, true), 'folders come before files');
$check(!in_array('.pam-desktop', $names, true), 'generated folders are hidden');
$workspace->toggle('tests');
$check(in_array('tests/VsCodeTest.php', array_map(static fn ($entry): string => $entry->path, $workspace->entries()), true), 'expanding a folder lists it');
$check(FileKind::language('Editor.pam.php') === 'PAM Component' && FileKind::icon('composer.json') === 'braces', 'file kinds by name');

// Tabs: opening, the active tab, closing moves to the neighbour.
$workspace->open('README.md');
$workspace->open('index.php');
$workspace->open('assets/fonts/OFL-NotoSans.txt');
$workspace->close('assets/fonts/OFL-NotoSans.txt');
$check($workspace->tabs === ['README.md', 'index.php'] && $workspace->active === 'index.php', 'closing the active tab selects its neighbour');
$workspace->open('assets/fonts/JetBrainsMono-Regular.ttf');
$check(count($workspace->tabs) === 2, 'binary files do not open');

// The workbench, mounted without a host: quick open and the status bar.
$screen = mount(WorkbenchScreen::class);
$check(str_contains($screen->text(), 'EXPLORER') && str_contains($screen->text(), 'PAM Component'), 'the workbench opens on a component');
$screen->instance->query = 'wrkspc';
$check(($screen->instance->matches()[0] ?? null) === 'src/Services/Workspace.php', 'quick open matches letters in order');
$screen->instance->openFile('src/Services/Workspace.php');
$screen->render();
$check($screen->instance->workspace->active === 'src/Services/Workspace.php' && str_contains($screen->text(), 'Workspace.php') && $screen->instance->workspace->lines('src/Services/Workspace.php') !== [], 'quick open opens the file');
$screen->unmount();

echo "VS Code example tests passed.\n";
