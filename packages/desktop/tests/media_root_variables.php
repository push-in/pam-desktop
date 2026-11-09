<?php

declare(strict_types=1);

use Pam\Desktop\EventKind;
use Pam\Desktop\Internal\CompiledTemplateNode;
use Pam\Desktop\Internal\Runtime;
use Pam\Desktop\Internal\ScopedStyleCompiler;
use Pam\Desktop\Internal\TemplateCompiler;
use Pam\Desktop\Internal\TemplateRenderer;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\PropKey;

/** @var Closure(bool, string): void $assert */

// `@media (prefers-color-scheme: …) { :root { --x: … } }` redeclares the
// custom properties for the whole sheet, as in CSS: the rules outside the
// query, their state blocks and the rules of other matched queries read the
// new values, and keep their place in the cascade.
$themeStyles = json_encode(ScopedStyleCompiler::compile(<<<'CSS'
    :root { --fg: #f4f4f5; --bg: #13141b; --brand: oklch(54.1% .281 293.009); --soft: color-mix(in srgb, var(--brand) 20%, transparent); }
    @media (prefers-color-scheme: light) { :root { --fg: #16161d; --bg: #ffffff; } }
    Text { color: var(--fg); }
    .card { background-color: var(--bg); border-color: var(--soft); }
    .card:hover { background-color: var(--fg); }
    .fixed { background-color: #ff0000; }
    @media (min-width: 100px) { .wide { background-color: var(--bg); } }
    CSS,
    'MediaRootVariables.pam',
), JSON_THROW_ON_ERROR);
$themeTemplate = TemplateCompiler::compile('<Column class="card fixed"><Text>Theme</Text><Column class="card wide" /></Column>');
$themeRender = static function () use ($themeTemplate, $themeStyles): array {
    $styled = new CompiledTemplateNode(
        kind: $themeTemplate->kind,
        name: $themeTemplate->name,
        attributes: [...$themeTemplate->attributes, '__pamStyles' => $themeStyles],
        source: $themeTemplate->source,
        line: $themeTemplate->line,
        column: $themeTemplate->column,
        value: $themeTemplate->value,
    );
    $styled->children = $themeTemplate->children;
    $root = TemplateRenderer::render($styled, null, []);
    $card = $root->properties();
    $text = $root->children()[0]->properties();
    $wide = $root->children()[1]->properties();

    return [
        'text' => $text[PropKey::TextColor->value] ?? null,
        'fixed' => $card[PropKey::BackgroundColor->value] ?? null,
        'wide' => $wide[PropKey::BackgroundColor->value] ?? null,
        'border' => $wide[PropKey::BorderColor->value] ?? null,
        'hover' => json_decode((string) ($wide[PropKey::NativeStateStyles->value] ?? 'null'), true),
    ];
};
$themeAppearance = static function (int $appearance): void {
    Runtime::dispatchEvent(0, EventKind::Dimensions->value, Wire::map([
        'width' => 400.0, 'height' => 800.0, 'density' => 1.0, 'appearance' => $appearance,
    ]));
};

$themeAppearance(2);
$dark = $themeRender();
$themeAppearance(1);
$light = $themeRender();
$hoverColor = static fn (mixed $states): mixed => is_array($states) ? array_values(array_filter(array_map(
    static fn (mixed $state): mixed => is_array($state) ? ($state[PropKey::BackgroundColor->value] ?? null) : null,
    $states,
)))[0] ?? null : null;
$assert(
    $dark['text'] === 0xFFF4F4F5 && $light['text'] === 0xFF16161D,
    'A :root redeclared inside a matched @media must re-resolve var() rules outside the query.',
);
$assert(
    $dark['fixed'] === 0xFFFF0000 && $light['fixed'] === 0xFFFF0000,
    'A later rule without var() must keep winning over an earlier re-resolved rule (cascade order).',
);
$assert(
    $dark['wide'] === 0xFF13141B && $light['wide'] === 0xFFFFFFFF,
    'Rules inside another matched query must read the redeclared custom properties.',
);
$assert(
    $dark['border'] === $light['border'] && $dark['border'] === 0x337F22FE,
    'oklch() and color-mix() custom properties must resolve to sRGB.',
);
$assert(
    $hoverColor($dark['hover']) === 0xFFF4F4F5 && $hoverColor($light['hover']) === 0xFF16161D,
    'State blocks that read var() must follow the redeclared custom properties.',
);
