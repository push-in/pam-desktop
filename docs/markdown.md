# Markdown

`Pam\Desktop\Markdown` renders Markdown as native elements: paragraphs and headings are rich text you can select, with links you can press, and lists, quotes, code blocks and tables are boxes. There is no web view involved. It ships inside `pushinbr/pam-desktop`.

```php
use Pam\Desktop\Markdown\Markdown;

Markdown::render("**Deploy** is done, see [the log](https://ci.example.com/42).");
```

In a template, register the tag once at startup (next to `App::components()`):

```php
Markdown::register();
```

```html
<Markdown :source="$message->body" theme="dark" />
```

## What it reads

- Headings (`#` to `######`), paragraphs, hard breaks (two trailing spaces or `\`), `---` rules.
- `**bold**`, `*italic*`, `~~strike~~`, `` `code` ``, `\` escapes. Underscores inside words stay text (`snake_case_name`).
- Links `[text](url)`, autolinks `<https://…>` and bare `https://` / `www.` addresses.
- Bullet and numbered lists (nested by indentation, numbered lists keep their first number) and task lists (`- [x] done`).
- Block quotes, fenced code blocks with a language, GitHub tables with column alignment, and an image alone in a paragraph.
- Mentions (`@ana`) when `mentions` is on, and `@Ana Souza` with spaces when `people` lists the names.

## Options

| `render()` argument | Tag attribute | Meaning |
| --- | --- | --- |
| `theme` | `theme="light"`, `"dark"` or a `MarkdownTheme` | Colours, sizes and fonts. |
| `onLink` | `@link` | Called with the URL of a pressed link. By default the system browser opens it. |
| `mentions` | `mentions="true"` | `@name` becomes a mention. |
| `people` | `:people="$names"` | Names a mention may hold spaces in. |
| `me` | `:me="$myName"` | Mentions of the reader stand out (`mentionSelf` colours). |
| `slack` | `flavor="slack"` | Slack's emphasis: `*bold*`, `_italic_`, `~strike~`. |
| `highlight` | (PHP only) | A closure that colours code blocks: `(string $code, string $language)` to `list<[string, colour]>`. See [Code](code.md). |

`MarkdownTheme::light()` and `MarkdownTheme::dark()` are the defaults. A `new MarkdownTheme(...)` sets any of `fontSize`, `lineHeight`, `text`, `muted`, `link`, `codeText`, `codeBackground`, `codeBlockText`, `codeBlockBackground`, `border`, `quoteBar`, `mention`, `mentionBackground`, `mentionSelf`, `mentionSelfBackground`, `tableHeader`, `monospace`, `fontFamily`, `blockGap` and `headings` (the six heading sizes).

`Markdown::parse()` returns the block tree without rendering it, for tests or for a renderer of your own.

The [Slack example](../examples/slack) renders every message body with `<Markdown flavor="slack">`.
