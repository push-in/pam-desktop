# Code

`Pam\Desktop\Code` colours source code and talks to language servers. It ships inside `pushinbr/pam-desktop`.

## Highlighting

```php
use Pam\Desktop\Code\CodeTheme;
use Pam\Desktop\Code\Highlighter;

$language = Highlighter::language($path);            // 'php', 'rust', … or null
$lines = Highlighter::lines($source, $language);     // one list of [text, ARGB colour] per line
```

- `lines()` gives one entry per line, without the newlines, ready for a virtualized list of `<Text>` with a `<Span>` per run (see the [VS Code example](../examples/vscode)).
- `runs()` gives the same coloured runs for the whole text. That is the shape `Markdown::render(highlight:)` takes, so `Markdown::render($doc, highlight: Highlighter::runs(...))` colours the code blocks of a document.
- `tokens()` gives `[text, TokenType]` pairs, for your own colours.
- Every character comes back, in order, whatever the language.

PHP goes through PHP's own tokenizer, so it is exact. A `.pam.php` component also has its template coloured. The other languages go through ordered token rules: JavaScript, TypeScript, JSON, CSS, HTML, XML, Markdown, SQL, shell, Python, Rust, Go, YAML, TOML, INI, diff and Dockerfile. An unknown language is plain text. `Highlighter::language()` picks a language from a file name, and `languages()` lists them all.

`CodeTheme::darkModern()` (the default) and `CodeTheme::lightModern()` are VS Code's themes. A `new CodeTheme([...])` maps each `TokenType` value (`comment`, `string`, `keyword`, `function`…) to a colour and also carries the editor's background, line number, active line, selection, cursor and guide colours.

## Language servers

`LspClient` starts a Language Server Protocol server (rust-analyzer, intelephense, typescript-language-server, pylsp…) as a child process and talks to it over stdio:

```php
use Pam\Desktop\Code\Lsp\LspClient;

$lsp = LspClient::start(['rust-analyzer'], $projectRoot, function (string $path, array $diagnostics): void {
    $this->problems[$path] = $diagnostics;      // list<Diagnostic>
});
$lsp->didOpen('src/main.rs', 'rust', $text);
$lsp->didChange('src/main.rs', $newText);        // full text, the version counts up
$lsp->hover('src/main.rs', $line, $character, fn (?string $markdown) => $this->hover = $markdown);
$lsp->completion('src/main.rs', $line, $character, fn (array $items) => $this->items = $items);
$lsp->definition('src/main.rs', $line, $character, fn (array $locations) => $this->jump($locations[0] ?? null));
```

- Paths are relative to the root (absolute ones work too), and positions are 0-based as in LSP.
- Messages sent before the server finished starting wait for it.
- `request()` and `notify()` reach any other method, and a server's error goes to the optional failure closure.
- The client answers the requests a server sends back (`workspace/configuration`) the way a simple client does.
- The server's output is read on a timer: every 15 ms while an answer is awaited or the server has just spoken, every 250 ms when it is quiet. `stop()` shuts the server down; it also ends with the application.
- `Diagnostic` has `path`, `line`, `character`, `endLine`, `endCharacter`, `severity` (`Diagnostic::ERROR`…`HINT`), `message` and `source`. `CompletionItem` has `label`, `insertText`, `detail` and `kind`, and `Location` has `path`, `line` and `character`.
