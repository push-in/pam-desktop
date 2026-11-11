<?php

declare(strict_types=1);

use Pam\Desktop\Markdown\Markdown;
use Pam\Desktop\Markdown\MarkdownTheme;
use Pam\Desktop\EventKind;
use Pam\Desktop\PropKey;

require dirname(__DIR__, 3).'/vendor/autoload.php';

$fail = static fn (string $message) => throw new RuntimeException($message);
$texts = static fn (array $runs): string => implode('', array_column($runs, 'text'));

// Blocks.
$blocks = Markdown::parse(<<<'MD'
# Release *2.2*

Ship it **now**, see [the log](https://ci.example.com/42) or https://pam.dev.
Second line  
after a hard break, `code_here` and snake_case_name.

> Quoted with **bold**
continued lazily

- one
- [x] done
  - nested
- three

3. third
4. fourth

| Name | Size |
| :--- | ---: |
| a.php | 12 KB |

```php
echo 1;
```

---
![logo](assets/logo.png)
MD);
$kinds = array_column($blocks, 0);
$kinds === ['heading', 'paragraph', 'quote', 'list', 'list', 'table', 'code', 'hr', 'image'] || $fail('Block kinds: '.json_encode($kinds));
$blocks[0][1] === 1 && $texts($blocks[0][2]) === 'Release 2.2' && $blocks[0][2][1]['italic'] || $fail('Heading with emphasis.');

$paragraph = $blocks[1][1];
$texts($paragraph) === "Ship it now, see the log or https://pam.dev. Second line\nafter a hard break, code_here and snake_case_name." || $fail('Paragraph text: '.json_encode($texts($paragraph)));
$bold = array_values(array_filter($paragraph, static fn ($run) => $run['bold']));
$bold[0]['text'] === 'now' || $fail('Strong emphasis.');
$links = array_values(array_filter($paragraph, static fn ($run) => $run['link'] !== null));
array_column($links, 'link') === ['https://ci.example.com/42', 'https://pam.dev'] || $fail('Links and bare URLs (without trailing period): '.json_encode(array_column($links, 'link')));
$code = array_values(array_filter($paragraph, static fn ($run) => $run['code']));
$code[0]['text'] === 'code_here' || $fail('Inline code keeps underscores.');

$texts($blocks[2][1][0][1]) === 'Quoted with bold continued lazily' || $fail('Quote with lazy continuation.');
[, $ordered, , $items] = $blocks[3];
!$ordered && count($items) === 3 && $items[1]['task'] === true && $items[0]['task'] === null || $fail('Bullet list with a task.');
$items[1]['blocks'][1][0] === 'list' || $fail('A nested list belongs to its item: '.json_encode($items[1]['blocks']));
$blocks[4][1] === true && $blocks[4][2] === 3 && count($blocks[4][3]) === 2 || $fail('Ordered lists keep their start number.');
[, $aligns, $header, $rows] = $blocks[5];
$aligns === ['left', 'right'] && $texts($header[1]) === 'Size' && $texts($rows[0][0]) === 'a.php' || $fail('Tables with alignment.');
$blocks[6][1] === 'php' && $blocks[6][2] === 'echo 1;' || $fail('Fenced code with its language.');
$blocks[8] === ['image', 'logo', 'assets/logo.png'] || $fail('A paragraph that is only an image.');

// Mentions only when asked for; emails are not mentions.
$plain = Markdown::parse('ping @ana and mail ana@example.com')[0][1];
array_filter($plain, static fn ($run) => $run['mention']) === [] || $fail('No mentions by default.');
$chat = Markdown::parse('ping @ana.silva, and mail ana@example.com', true)[0][1];
array_column(array_filter($chat, static fn ($run) => $run['mention']), 'text') === ['@ana.silva'] || $fail('Mentions: '.json_encode($chat));

// Unclosed delimiters stay text; escapes too.
$texts(Markdown::parse('2 * 3 = 6 and \\*not em\\* and **open')[0][1]) === '2 * 3 = 6 and *not em* and **open' || $fail('Unclosed and escaped delimiters stay literal.');

// Rendering: native elements, pressable links.
$opened = [];
$element = Markdown::render("Hello [docs](https://pam.dev) **you**\n\n- item", MarkdownTheme::dark(), static function (string $url) use (&$opened): void {
    $opened[] = $url;
});
$paragraphText = $element->children()[0];
$paragraphText->properties()[PropKey::Text->value] === 'Hello docs you' || $fail('A paragraph is one rich text.');
$paragraphText->properties()[PropKey::TextColor->value] === MarkdownTheme::dark()->text || $fail('The theme colours the text.');
$paragraphText->events()[EventKind::SpanPress->value]('0');
$opened === ['https://pam.dev'] || $fail('A pressed link reaches onLink: '.json_encode($opened));

Markdown::register();
$template = Pam\Desktop\Internal\TemplateCompiler::compile('<Markdown source="# Hi" theme="dark" />', 'md.pam');
$rendered = Pam\Desktop\Internal\TemplateRenderer::render($template, new stdClass(), [])->toElement();
str_contains(json_encode($rendered->properties() + ['c' => array_map(static fn ($child) => $child->properties(), $rendered->children())]), 'Hi') || $fail('The <Markdown> tag renders.');
echo "Markdown: blocks, inline runs, links, mentions, tables, tasks, rendering and the tag: passed\n";
