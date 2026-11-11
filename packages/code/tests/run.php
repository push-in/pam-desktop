<?php

declare(strict_types=1);

use Pam\Desktop\Code\CodeTheme;
use Pam\Desktop\Code\Highlighter;
use Pam\Desktop\Code\Lsp\Diagnostic;
use Pam\Desktop\Code\Lsp\LspClient;
use Pam\Desktop\Code\TokenType;

require dirname(__DIR__, 3).'/vendor/autoload.php';

$fail = static fn (string $message) => throw new RuntimeException($message);
$typed = static function (string $source, string $language): array {
    $types = [];
    foreach (Highlighter::tokens($source, $language) as [$text, $type]) {
        if (trim($text) !== '') {
            $types[trim($text)] ??= $type;
        }
    }

    return $types;
};

// Languages by file name.
Highlighter::language('src/Editor.pam.php') === 'pam' && Highlighter::language('a/b.ts') === 'typescript'
    && Highlighter::language('Dockerfile') === 'dockerfile' && Highlighter::language('notes.unknown') === null
    || $fail('Languages by file name.');

// Every language keeps every character, in order.
$samples = [
    'php' => "<?php\n/** doc */\nfinal class A { public function b(): int { return \$this->c ?? 0x1F; } }\n",
    'pam' => "<?php\nclass A {}\n?>\n<template><Text :class=\"\$x\">{{ \$name }}</Text></template>\n",
    'javascript' => "const a = `x\${b}`; // note\nfunction go(n) { return n * 2.5e3; }\n",
    'typescript' => "interface User { id: number }\nexport const users: User[] = [];\n",
    'json' => "{\"name\": \"pam\", \"n\": -1.5e2, \"ok\": true, \"list\": [null]}",
    'css' => "/* c */ .row:hover { background-color: #2A2D2E; width: calc(100% - 8px); }\n@media (prefers-color-scheme: dark) {}\n",
    'html' => "<!-- c --><a href=\"/x\" @click=\"go\">&amp; hi</a>",
    'markdown' => "# Title\n\nSome **bold** and `code`, [link](https://x.y).\n\n```php\necho 1;\n```\n> quote\n- item\n",
    'sql' => "SELECT id, count(*) FROM cards -- c\nWHERE title = 'it''s' AND n > 3;",
    'shell' => "#!/bin/sh\nfor f in *.php; do echo \"\$f\" | wc -l; done # c\n",
    'python' => "@dataclass\nclass A:\n    def go(self, n: int) -> None:\n        return f\"{n}\" # c\n",
    'rust' => "#[derive(Debug)]\nfn main() { let mut v: Vec<u8> = vec![1]; println!(\"{}\", 'x'); }\n",
    'go' => "package main\nfunc main() { x := []string{\"a\"}; fmt.Println(x) }\n",
    'yaml' => "name: ci # c\non:\n  push:\n    branches: [main]\nok: true\n",
    'toml' => "[package]\nname = \"pam\"\nversion = 2\n",
    'diff' => "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n",
    'dockerfile' => "FROM ubuntu:22.04\nRUN apt-get update # c\nENV A=\${B}\n",
];
foreach ($samples as $language => $source) {
    $joined = implode('', array_column(Highlighter::tokens($source, $language), 0));
    $joined === $source || $fail("{$language} lost characters: ".json_encode($joined));
    $lines = Highlighter::lines($source, $language);
    count($lines) === substr_count($source, "\n") + 1 || $fail("{$language}: one entry per line.");
}

// Token types where they matter.
$php = $typed($samples['php'], 'php');
$php['/** doc */'] === TokenType::Comment && $php['final'] === TokenType::Keyword && $php['b'] === TokenType::Function
    && $php['$this'] === TokenType::Variable && $php['c'] === TokenType::Property && $php['return'] === TokenType::Control
    && $php['0x1F'] === TokenType::Number || $fail('PHP token types: '.json_encode(array_map(static fn ($t) => $t->value, $php)));
$pam = $typed($samples['pam'], 'pam');
$pam['<template'] === TokenType::Tag && $pam[':class'] === TokenType::Attribute && $pam['{{ $name }}'] === TokenType::Variable || $fail('A .pam.php template is coloured.');
$js = $typed($samples['javascript'], 'javascript');
$js['const'] === TokenType::Keyword && $js['`x${b}`'] === TokenType::String && $js['// note'] === TokenType::Comment && $js['go'] === TokenType::Function && $js['2.5e3'] === TokenType::Number || $fail('JavaScript token types.');
$json = $typed($samples['json'], 'json');
$json['"name"'] === TokenType::Property && $json['"pam"'] === TokenType::String && $json['true'] === TokenType::Constant || $fail('JSON keys and values.');
$css = $typed($samples['css'], 'css');
$css['.row'] === TokenType::Type && $css['background-color'] === TokenType::Property && $css['#2A2D2E'] === TokenType::Number && $css['@media'] === TokenType::Control || $fail('CSS token types.');
$rust = $typed($samples['rust'], 'rust');
$rust['fn'] === TokenType::Keyword && $rust['println!'] === TokenType::Function && $rust['Vec'] === TokenType::Type && $rust['#[derive(Debug)]'] === TokenType::Attribute || $fail('Rust token types.');
$sql = $typed($samples['sql'], 'sql');
$sql['SELECT'] === TokenType::Keyword && $sql["'it''s'"] === TokenType::String && $sql['count'] === TokenType::Function || $fail('SQL token types.');
$diff = $typed($samples['diff'], 'diff');
$diff['+new'] === TokenType::Inserted && $diff['-old'] === TokenType::Deleted || $fail('Diff lines.');

// Runs carry theme colours; plain text for unknown languages.
$runs = Highlighter::runs("// hi\nx", 'javascript', CodeTheme::lightModern());
$runs[0] === ['// hi', CodeTheme::lightModern()->color(TokenType::Comment)] || $fail('Runs use the theme.');
Highlighter::runs('anything', 'klingon') === [['anything', CodeTheme::darkModern()->text]] || $fail('Unknown languages are plain text.');

// A large file stays fast.
$big = str_repeat($samples['javascript'], 2000);
$started = hrtime(true);
Highlighter::lines($big, 'javascript');
$elapsed = (hrtime(true) - $started) / 1e6;
$elapsed < 1500 || $fail("Highlighting 4000 lines took {$elapsed} ms.");

// LSP against a small fake server.
if (trim((string) shell_exec('command -v python3')) === '') {
    echo "Code: highlighting passed (no python3: LSP client skipped)\n";
    return;
}
$root = sys_get_temp_dir().'/pam-lsp-'.getmypid();
@mkdir($root);
$diagnostics = [];
$client = LspClient::start(['python3', __DIR__.'/Fixtures/fake_lsp.py'], $root, static function (string $path, array $found) use (&$diagnostics): void {
    $diagnostics[$path] = $found;
}, autoPump: false);
$client->didOpen('src/main.rs', 'rust', "fn main() {\n    let x = 1;\n}\n");
$hover = $completions = $definitions = $failure = null;
$client->hover('src/main.rs', 0, 3, static function (?string $markdown) use (&$hover): void { $hover = $markdown; });
$client->completion('src/main.rs', 1, 4, static function (array $items) use (&$completions): void { $completions = $items; });
$client->definition('src/main.rs', 1, 8, static function (array $locations) use (&$definitions): void { $definitions = $locations; });
$client->request('nope', [], static function (): void {}, static function (string $message) use (&$failure): void { $failure = $message; });
for ($i = 0; $i < 100 && ($hover === null || $completions === null || $definitions === null || $failure === null || $diagnostics === []); ++$i) {
    $client->pump(20);
}
$hover === '**fn main** at 0:3' || $fail('Hover: '.json_encode($hover));
array_map(static fn ($item) => [$item->label, $item->insertText], $completions ?? []) === [['println!', 'println!'], ['print!', 'print!()']] || $fail('Completion items.');
($definitions[0] ?? null)?->path === 'src/main.rs' && $definitions[0]->line === 7 || $fail('Definition locations come back relative to the root.');
$failure === 'no such method' || $fail('Server errors reach the failure handler.');
$found = $diagnostics['src/main.rs'] ?? [];
count($found) === 1 && $found[0]->severity === Diagnostic::WARNING && $found[0]->message === 'unused variable' && $found[0]->line === 1 || $fail('Published diagnostics.');
$client->stop();
$client->running() && $fail('stop() ends the server.');
@rmdir($root);
echo "Code: highlighting of 18 languages, themes, speed and the LSP client: passed\n";
