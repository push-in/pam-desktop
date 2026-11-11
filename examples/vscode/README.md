# Visual Studio Code

A VS Code workbench clone built only with PAM and PAM Desktop: no web view,
no JavaScript, every pixel a native view. It opens its own folder.

![VS Code clone, recorded live](demo.gif)

```bash
pam composer install
pam desktop dev
```

## What it shows

- **The workbench** in the Dark Modern theme: title bar with the command
  center, activity bar, side bar, editor, panel and status bar.
- **Explorer**: this project read from disk one folder at a time as it is
  expanded, folders first, file icons by type, indent guides on hover, a
  `VirtualizedList` so `vendor/` stays cheap.
- **Editor**: tabs (close on hover, Ctrl+W), breadcrumbs, line numbers, the
  cursor line, a minimap, and syntax colours from PHP's own tokenizer
  (`token_get_all`) for PHP and patterns for templates, CSS, JSON and Markdown.
- **Quick Open**: Ctrl+P, letters in order (`wrkspc` finds `Workspace.php`),
  Enter opens the first.
- **Shortcuts**: Ctrl+P, Ctrl+B (side bar), Ctrl+` (panel), Ctrl+W, registered
  with `Shortcut::register`.

Icons are the Lucide set bundled with PAM Desktop (`font="Lucide"`); JetBrains
Mono (without ligatures, as VS Code ships) and Noto Sans, Latin subsets, are
packaged under `assets/fonts`.

## Layout

| Path | What goes there |
| --- | --- |
| `src/Screens/Workbench` | The window, the activity bar, the panel, the status bar, Quick Open and the shortcuts. |
| `src/Components` | `Explorer`, `Editor`. |
| `src/Services` | `Workspace` (tree, tabs, documents) and `Highlighter` (the colours). |
| `src/Domain` | Explorer entries and file kinds. |
| `tests/VsCodeTest.php` | `pam composer test`: colours, the tree, tabs, Quick Open and the mounted screen. |

| Task | Command |
| --- | --- |
| Check every template, stylesheet and route | `pam desktop check` |
| Run the tests | `pam composer test` |

JetBrains Mono and Noto Sans: SIL Open Font License (`assets/fonts/OFL-*.txt`).
