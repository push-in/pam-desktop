<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\KeyEvent;
use Pam\Desktop\LayoutEvent;
use Pam\Desktop\PressEvent;
use Pam\DesktopUi\CodeEditor;

$loader = require dirname(__DIR__, 3).'/vendor/autoload.php';
$loader->addPsr4('Pam\\DesktopUi\\', dirname(__DIR__).'/src');
App::components(dirname(__DIR__).'/src', sys_get_temp_dir().'/pam-desktop-code-editor-tests');
$fail = static fn (string $message) => throw new RuntimeException($message);

$editor = new CodeEditor("<?php\necho 1;\n", language: 'php');
$mounted = Pam\Desktop\Testing\mount($editor);
try {
    $mounted->render();
    $editor->measureChar(new LayoutEvent(0, 0, 780, 19));
    $editor->charWidth === 7.8 || $fail('The column width comes from the probe.');
    count($editor->rows()) === 3 || $fail('One row per line.');
    $colours = array_unique(array_column($editor->runs(1), 1));
    count($colours) >= 2 || $fail('Lines are highlighted.');

    // A click puts the cursor at the column under the pointer.
    $x = $editor->gutterWidth() + 4 + 5 * 7.8;
    $editor->press(1, new PressEvent($x, 9, $x, 9, 0, 0));
    $editor->document->cursors[0] === [1, 5, 1, 5] || $fail('Click to column: '.json_encode($editor->document->cursors));

    // Typing arrives through the hidden field, a growing value.
    $editor->typed('2');
    $editor->typed('23');
    $editor->document->lines[1] === 'echo 231;' || $fail('Typed text goes in once, at the cursor: '.json_encode($editor->document->lines[1]));
    $editor->text === "<?php\necho 231;\n" || $fail('The text follows the document.');
    $editor->key(new KeyEvent('Enter'));
    $editor->key(new KeyEvent('z', control: true));
    $editor->document->lines[1] === 'echo 231;' && count($editor->document->lines) === 3 || $fail('Ctrl+Z undoes the new line.');
    $editor->key(new KeyEvent('End', shift: true));
    $editor->key(new KeyEvent('Backspace'));
    $editor->document->lines[1] === 'echo 23' || $fail('Shift+End selects to the end; Backspace deletes it: '.$editor->document->lines[1]);
    $selections = $editor->selections(1);
    $selections === [] || $fail('No selection after deleting.');
    $editor->key(new KeyEvent('Home', shift: true));
    $editor->selections(1) === [[0.0, 7 * 7.8]] || $fail('Selections in pixels: '.json_encode($editor->selections(1)));
    $editor->carets(1) === [0.0] || $fail('The caret sits at the head of the selection.');

    $readOnly = new CodeEditor('fixed', readOnly: true);
    $readOnly->typed('x');
    $readOnly->key(new KeyEvent('Backspace'));
    $readOnly->document->text() === 'fixed' || $fail('A read-only editor does not change.');
    $mounted->render();
} finally {
    $mounted->unmount();
}
echo "CodeEditor: column width, highlighting, click, typing through the field, keys, selections and read-only: passed\n";
