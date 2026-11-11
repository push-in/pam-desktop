<?php

declare(strict_types=1);

use Pam\DesktopUi\CodeDocument;

$loader = require dirname(__DIR__, 3).'/vendor/autoload.php';
$loader->addPsr4('Pam\\DesktopUi\\', dirname(__DIR__).'/src');
$fail = static fn (string $message) => throw new RuntimeException($message);

$doc = new CodeDocument("function a() {\n    return 1;\n}");
$doc->setCursor(0, 14);
$doc->newline();
$doc->lines[1] === '    ' && $doc->cursors[0] === [1, 4, 1, 4] || $fail('Enter after { indents one level: '.json_encode($doc->lines));
$doc->insert('x');
$doc->insert('y');
$doc->lines[1] === '    xy' || $fail('Typing.');
$doc->undo();
$doc->lines[1] === '    ' || $fail('Typed characters undo together: '.json_encode($doc->lines[1]));
$doc->undo();
$doc->text() === "function a() {\n    return 1;\n}" || $fail('Undo the new line.');
$doc->redo();
$doc->redo();
$doc->lines[1] === '    xy' || $fail('Redo.');

// Backspace removes an indent level before the text, then characters.
$doc = new CodeDocument("        x");
$doc->setCursor(0, 8);
$doc->delete(-1);
$doc->lines[0] === '    x' || $fail('Backspace in the indent removes a level: '.json_encode($doc->lines[0]));
$doc->setCursor(0, 5);
$doc->delete(-1);
$doc->lines[0] === '    ' || $fail('Backspace removes a character.');

// Selections and multi-line edits.
$doc = new CodeDocument("alpha\nbeta\ngamma");
$doc->setCursor(0, 2);
$doc->setCursor(2, 3, extend: true);
$doc->selectedText() === "pha\nbeta\ngam" || $fail('A selection across lines: '.json_encode($doc->selectedText()));
$doc->insert('-');
$doc->text() === 'al-ma' || $fail('Typing replaces the selection.');
$doc->insert("1\n2");
$doc->text() === "al-1\n2ma" && $doc->cursors[0] === [1, 1, 1, 1] || $fail('Pasting several lines.');

// Several cursors.
$doc = new CodeDocument("let a = 1;\nlet b = a;\nreturn a;");
$doc->setCursor(0, 4);
$doc->selectNextOccurrence();
$doc->selectedText() === 'a' || $fail('Ctrl+D selects the word.');
$doc->selectNextOccurrence();
$doc->selectNextOccurrence();
count($doc->cursors) === 3 || $fail('Ctrl+D adds the next occurrences: '.json_encode($doc->cursors));
$doc->insert('total');
$doc->text() === "let total = 1;\nlet b = total;\nreturn total;" || $fail('Typing at every cursor: '.json_encode($doc->text()));
$doc->singleCursor();
count($doc->cursors) === 1 || $fail('Escape keeps one cursor.');
$doc->setCursor(0, 0);
$doc->setCursor(1, 0, add: true);
$doc->setCursor(2, 0, add: true);
$doc->insert("//\n//\n//");
$doc->lines[0] === '//let total = 1;' && $doc->lines[2] === '//return total;' || $fail('A paste with one line per cursor spreads them.');

// Tabs show as tab stops; columns map back.
$doc = new CodeDocument("\tif (x) {\n\t\ty();");
$doc->display(1) === '        y();' && $doc->visual(1, 2) === 8 && $doc->column(1, 9) === 3 || $fail('Tabs expand to stops.');

// Indent, outdent, comments, word moves.
$doc = new CodeDocument("a\nb\nc");
$doc->setCursor(0, 0);
$doc->setCursor(2, 1, extend: true);
$doc->indent();
$doc->text() === "    a\n    b\n    c" || $fail('Tab indents selected lines.');
$doc->indent(outdent: true);
$doc->text() === "a\nb\nc" || $fail('Shift+Tab outdents.');
$doc->toggleComment();
$doc->text() === "// a\n// b\n// c" || $fail('Comment the lines.');
$doc->toggleComment();
$doc->text() === "a\nb\nc" || $fail('And back.');
$doc = new CodeDocument('foo.bar(baz)');
$doc->setCursor(0, 0);
$doc->move('wordRight');
$doc->cursors[0][1] === 3 || $fail('Word right stops at the end of a word.');
$doc->move('wordRight', extend: true);
$doc->selectedText() === '.' || $fail('Shift extends.');
$doc->move('end');
$doc->move('wordLeft');
$doc->cursors[0][1] === 11 || $fail('Word left: '.json_encode($doc->cursors));
$doc->delete(-1, word: true);
$doc->text() === 'foo.bar()' || $fail('Ctrl+Backspace deletes the word before the cursor: '.$doc->text());
echo "CodeDocument: typing, indent, undo, selections, cursors, tabs, comments and words: passed\n";
