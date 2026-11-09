<?php

declare(strict_types=1);

use Pam\Desktop\EventKind;
use Pam\Desktop\Internal\CompiledTemplateNode;
use Pam\Desktop\Internal\ScopedStyleCompiler;
use Pam\Desktop\Internal\TemplateCompiler;
use Pam\Desktop\Internal\TemplateRenderer;
use Pam\Desktop\PropKey;
use Pam\Desktop\UI\Text;

/**
 * React Native typography/component parity: nested text spans,
 * includeFontPadding, hairline units and related layout contracts.
 *
 * @var Closure(bool, string): void $assert
 */
$typographyRender = static function (string $template, string $css, ?object $scope = null): \Pam\Desktop\Element {
    $compiled = TemplateCompiler::compile($template);
    $styled = new CompiledTemplateNode(
        kind: $compiled->kind,
        name: $compiled->name,
        attributes: [
            ...$compiled->attributes,
            '__pamStyles' => json_encode(ScopedStyleCompiler::compile($css, 'TypographyParity.pam.php'), JSON_THROW_ON_ERROR),
        ],
        source: $compiled->source,
        line: $compiled->line,
        column: $compiled->column,
        value: $compiled->value,
    );
    $styled->children = $compiled->children;

    return TemplateRenderer::render($styled, $scope, []);
};

// Nested spans: one paragraph, inherited styles, per-span press handlers.
$pressedLinks = [];
$richScope = new class ($pressedLinks) {
    /** @param list<string> $log */
    public function __construct(public array &$log)
    {
    }

    public function openMention(): void
    {
        $this->log[] = 'mention';
    }

    public function openLink(): void
    {
        $this->log[] = 'link';
    }
};
$rich = $typographyRender(
    '<Text class="body">Olá <Text class="strong">mundo <Span class="link" on:press="openLink">https://zé.chat</Span></Text>'
        .' e <Span class="mention" on:press="openMention">@ana</Span>!</Text>',
    '.body { font-size: 15px; line-height: 20px; color: #111111; }'
        .' .strong { font-weight: 700; }'
        .' .link { color: #1B7A4E; text-decoration: underline; }'
        .' .mention { color: #0055FF; background-color: #EEEEEE; font-size: 13px; }',
    $richScope,
);
$richProperties = $rich->properties();
$assert(
    ($richProperties[PropKey::Text->value] ?? null) === 'Olá mundo https://zé.chat e @ana!',
    'Nested Text/Span children must render as one paragraph string: '.var_export($richProperties[PropKey::Text->value] ?? null, true),
);
$richSpans = explode(';', (string) ($richProperties[PropKey::TextSpans->value] ?? ''));
$assert(count($richSpans) === 3, 'Each styled nested run must produce one span: '.implode(';', $richSpans));
$boldFields = explode(',', $richSpans[0]);
$assert(
    $boldFields[0] === '4' && $boldFields[1] === '25' && $boldFields[3] === '700',
    'The bold run must cover "mundo https://zé.chat" in code points: '.$richSpans[0],
);
$linkFields = explode(',', $richSpans[1]);
$assert(
    $linkFields[0] === '10' && $linkFields[1] === '25' && $linkFields[3] === '700'
        && $linkFields[5] === (string) 0xFF1B7A4E && $linkFields[7] === '2' && $linkFields[10] === '0',
    'Inner spans must inherit ancestor weight and carry color, decoration and their press slot: '.$richSpans[1],
);
$mentionFields = explode(',', $richSpans[2]);
$assert(
    $mentionFields[0] === '28' && $mentionFields[1] === '32' && $mentionFields[2] === '13'
        && $mentionFields[6] === (string) 0xFFEEEEEE && $mentionFields[10] === '1',
    'Mention spans must carry size, background and their own press slot: '.$richSpans[2],
);
$assert(
    (float) ($richProperties[PropKey::LineHeight->value] ?? 0) === 20.0
        && isset($richProperties[PropKey::OnSpanPress->value]),
    'Paragraph styles stay on the outer node and span presses are wired.',
);
$spanPress = $rich->events()[EventKind::SpanPress->value] ?? null;
$assert($spanPress instanceof Closure, 'Rich text must register a span press handler.');
$spanPress('1');
$spanPress('0');
$spanPress('9');
$assert($richScope->log === ['mention', 'link'], 'Span presses must dispatch to the pressed run only.');

$apiRich = Text::rich('a', Text::rich('b', Text::make('c')->property(PropKey::FontWeight, 600)), 'd');
$assert(
    $apiRich->properties()[PropKey::Text->value] === 'abcd'
        && $apiRich->properties()[PropKey::TextSpans->value] === '2,3,,600',
    'Text::rich must flatten nested runs with code point offsets.',
);
$familyRich = Text::rich('a', Text::make('b')->property(PropKey::FontFamily, 'ui-monospace, "Liberation Mono", monospace'));
$assert(
    $familyRich->properties()[PropKey::TextSpans->value] === '1,2,,,,,,,,ui-monospace|Liberation Mono|monospace',
    'A span must carry a CSS font-family list, its commas as |.',
);
$plainNested = Text::rich('x', Text::make('y'));
$assert(
    !isset($plainNested->properties()[PropKey::TextSpans->value]),
    'Unstyled runs must not emit spans.',
);

// Inline boxes: a nested Text with padding, border, radius or background is
// a CSS inline box (`<kbd>`); its background moves from the run to the box.
$kbd = $typographyRender(
    '<Text>a <Text class="kbd">Ctrl</Text> b <Text padding="1px 6px" borderWidth="1" backgroundColor="#333" borderRadius="4">Esc</Text>'
        .' <Text backgroundColor="#EEEEEE">hl</Text></Text>',
    '.kbd { padding: 1px 6px; border-width: 1px; border-bottom-width: 2px; border-color: #FFFFFF; border-radius: 6px;'
        .' background-color: #222222; font-weight: 600; line-height: 15.4px; }',
);
$kbdProperties = $kbd->properties();
$assert($kbdProperties[PropKey::Text->value] === 'a Ctrl b Esc hl', 'Inline boxes keep one paragraph string.');
$assert(
    ($kbdProperties[PropKey::TextSpanBoxes->value] ?? null)
        === '2,6,1,6,1,6,1,1,2,1,' . 0xFFFFFFFF.',6,' . 0xFF222222.',15.4;9,12,1,6,1,6,1,1,1,1,,4,' . 0xFF333333,
    'Class and attribute boxes encode padding, per-side borders, colour, radius, background and line height: '
        .var_export($kbdProperties[PropKey::TextSpanBoxes->value] ?? null, true),
);
$assert(
    ($kbdProperties[PropKey::TextSpans->value] ?? null) === '2,6,,600;13,15,,,,,' . 0xFFEEEEEE,
    'A boxed run does not repeat its background as a glyph highlight; a plain background run keeps it: '
        .var_export($kbdProperties[PropKey::TextSpans->value] ?? null, true),
);
$assert(
    !isset($typographyRender('<Text>a <Text class="b">b</Text></Text>', '.b { font-weight: 700; }')->properties()[PropKey::TextSpanBoxes->value]),
    'Runs without a box send no TextSpanBoxes.',
);
$inlineBlock = $typographyRender(
    '<Text>a <Text class="kbd">Ctrl</Text> b</Text>',
    '.kbd { display: inline-block; padding: 1px 6px; border-width: 1px; line-height: 15.4px; }',
)->properties();
$assert(
    ($inlineBlock[PropKey::TextSpanBoxes->value] ?? null) === '2,6,1,6,1,6,1,1,1,1,,0,,15.4,1',
    'display: inline-block flags the box record (field 15): '
        .var_export($inlineBlock[PropKey::TextSpanBoxes->value] ?? null, true),
);

// Inline-level boxes: the parent lays them on line boxes (InlineLevel).
$inlineLevels = $typographyRender(
    '<Row class="cell"><Text class="a">a</Text><Input class="b" multiline="true" /><Row class="c"><Text>c</Text></Row></Row>',
    '.cell { font-size: 16px; line-height: 24px; } .a { display: inline; vertical-align: text-top; }'
        .' .b { display: inline-block; vertical-align: text-bottom; } .c { display: inline-flex; vertical-align: middle; }',
);
[$inlineText, $inlineField, $inlineFlex] = $inlineLevels->children();
$assert(
    ($inlineText->properties()[PropKey::InlineLevel->value] ?? null) === 1
        && ($inlineText->properties()[PropKey::VerticalAlign->value] ?? null) === 5
        && ($inlineField->properties()[PropKey::InlineLevel->value] ?? null) === 2
        && ($inlineField->properties()[PropKey::VerticalAlign->value] ?? null) === 6
        && ($inlineFlex->properties()[PropKey::InlineLevel->value] ?? null) === 3
        && ($inlineFlex->properties()[PropKey::VerticalAlign->value] ?? null) === 3
        && ($inlineLevels->properties()[PropKey::FontSize->value] ?? null) == 16,
    'display: inline/inline-block/inline-flex and vertical-align compile to InlineLevel and VerticalAlign.',
);

// overflow: auto on a ScrollView scrolls both axes (ScrollBothAxes).
$bothAxes = $typographyRender('<ScrollView class="wrap"><Text>a</Text></ScrollView>', '.wrap { overflow: auto; }')->properties();
$assert(
    ($bothAxes[PropKey::ScrollBothAxes->value] ?? null) === true,
    'overflow: auto compiles to ScrollBothAxes on a ScrollView.',
);

// CSS tables and font-relative lengths.
$table = $typographyRender(
    '<Column class="t"><Row class="r"><Text class="c">a</Text></Row></Column>',
    '.t { display: table; border-collapse: collapse; table-layout: fixed; }'
        .' .r { display: table-row; } .c { display: table-cell; vertical-align: middle; max-width: 52ch; }',
);
$tableProperties = $table->properties();
$row = $table->children()[0];
$cell = $row->children()[0];
$assert(
    ($tableProperties[PropKey::TableDisplay->value] ?? null) === 1
        && ($tableProperties[PropKey::BorderCollapse->value] ?? null) === 2
        && ($tableProperties[PropKey::TableLayout->value] ?? null) === 2
        && ($row->properties()[PropKey::TableDisplay->value] ?? null) === 3
        && ($cell->properties()[PropKey::TableDisplay->value] ?? null) === 4
        && ($cell->properties()[PropKey::VerticalAlign->value] ?? null) === 3,
    'display: table/table-row/table-cell, border-collapse, table-layout and vertical-align compile to table properties.',
);
$assert(
    (float) ($cell->properties()[PropKey::MaxWidth->value] ?? 0) === 52.0
        && ($cell->properties()[PropKey::LengthUnits->value] ?? null) === PropKey::MaxWidth->value.'=ch',
    'max-width: 52ch travels as 52 with the ch unit for the renderer: '
        .var_export($cell->properties()[PropKey::LengthUnits->value] ?? null, true),
);

// includeFontPadding: React Native Android default true, CSS/prop opt-out.
$padded = $typographyRender('<Text class="tight">Aa</Text>', '.tight { include-font-padding: false; }');
$assert(
    ($padded->properties()[PropKey::IncludeFontPadding->value] ?? null) === false,
    'include-font-padding: false must reach the native text node.',
);
$vendorPadded = $typographyRender('<Text class="tight">Aa</Text>', '.tight { -pam-include-font-padding: false; }');
$assert(
    ($vendorPadded->properties()[PropKey::IncludeFontPadding->value] ?? null) === false,
    '-pam-include-font-padding must be accepted as an alias.',
);
$attributePadded = $typographyRender('<Text includeFontPadding="false">Aa</Text>', '');
$assert(
    ($attributePadded->properties()[PropKey::IncludeFontPadding->value] ?? null) === false,
    'The includeFontPadding attribute must compile to a boolean.',
);

// Hairline and device-pixel units resolve against the live display density.
\Pam\Desktop\Internal\Runtime::dispatchEvent(0, EventKind::Dimensions->value, \Pam\Desktop\Internal\Wire::map([
    'width' => 411.0, 'height' => 914.0, 'density' => 2.625, 'fontScale' => 1.0,
]));
$hairline = $typographyRender(
    '<View class="row"><View class="line" /></View>',
    '.row { border-bottom: hairline solid #DDDDDD; } .line { height: 2dpx; border-width: hairline; }',
);
$lineView = $hairline->children()[0];
$assert(
    abs((float) $hairline->properties()[PropKey::BorderBottomWidth->value] - 1 / 2.625) < 1e-9
        && abs((float) $lineView->properties()[PropKey::Height->value] - 2 / 2.625) < 1e-9
        && abs((float) $lineView->properties()[PropKey::BorderWidth->value] - 1 / 2.625) < 1e-9,
    'hairline must be one physical pixel and Ndpx N physical pixels.',
);
// The same element re-resolves its environment-dependent styles (cached
// per environment) after the display density changes.
\Pam\Desktop\Internal\Runtime::dispatchEvent(0, EventKind::Dimensions->value, \Pam\Desktop\Internal\Wire::map([
    'width' => 411.0, 'height' => 914.0, 'density' => 2.0, 'fontScale' => 1.0,
]));
$denser = $typographyRender(
    '<View class="row"><View class="line" /></View>',
    '.row { border-bottom: hairline solid #DDDDDD; } .line { height: 2dpx; border-width: hairline; }',
);
$assert(
    abs((float) $denser->properties()[PropKey::BorderBottomWidth->value] - 0.5) < 1e-9
        && abs((float) $denser->children()[0]->properties()[PropKey::Height->value] - 1.0) < 1e-9,
    'Environment-dependent styles must follow a density change for the same element.',
);
\Pam\Desktop\Internal\Runtime::dispatchEvent(0, EventKind::Dimensions->value, \Pam\Desktop\Internal\Wire::map([
    'width' => 411.0, 'height' => 914.0, 'density' => 2.625, 'fontScale' => 1.0,
]));
$restored = $typographyRender(
    '<View class="row"><View class="line" /></View>',
    '.row { border-bottom: hairline solid #DDDDDD; } .line { height: 2dpx; border-width: hairline; }',
);
$assert(
    abs((float) $restored->properties()[PropKey::BorderBottomWidth->value] - 1 / 2.625) < 1e-9,
    'Environment-dependent styles must come back with the previous density.',
);
$assert(
    \Pam\Desktop\PixelRatio::hairlineWidth(4.0) === 0.5
        && \Pam\Desktop\PixelRatio::hairlineWidth(1.0) === 1.0
        && \Pam\Desktop\PixelRatio::roundToNearestPixel(10.3, 2.625) === round(10.3 * 2.625) / 2.625,
    'PixelRatio must follow React Native rounding.',
);

// Safe areas are known from the first render (host-exported boot metrics).
putenv('PAM_BOOT_METRICS={"width":411.4,"height":914.3,"density":2.625,"fontScale":1.0,"safeAreaTop":24.0,"safeAreaBottom":48.0}');
$bootMetrics = (new ReflectionMethod(\Pam\Desktop\Internal\Runtime::class, 'bootMetrics'))->invoke(null);
putenv('PAM_BOOT_METRICS');
$assert(
    $bootMetrics->width === 411.4 && $bootMetrics->density === 2.625
        && $bootMetrics->safeAreaTop === 24.0 && $bootMetrics->safeAreaBottom === 48.0,
    'Boot window metrics must include host safe areas before the first Dimensions event.',
);

// Pressed transforms: :active/:pressed translate like React Native buttons.
$pressed = $typographyRender(
    '<Pressable class="primary"><Text>Enviar</Text></Pressable>',
    '.primary { background-color: #1B7A4E; } .primary:active { transform: translateY(1.1px); opacity: 0.9; }',
);
$pressedStates = json_decode((string) ($pressed->properties()[PropKey::NativeStateStyles->value] ?? '{}'), true);
$assert(
    abs((float) ($pressedStates[(string) \Pam\Desktop\Style\StyleInteractionState::Pressed->value][(string) PropKey::TranslationY->value] ?? 0) - 1.1) < 1e-9
        && (float) ($pressed->properties()[PropKey::PressOpacity->value] ?? 0) === 0.9,
    'A :active translateY must become a native pressed-state translation: '.json_encode($pressedStates),
);

// Vector icons: react-native-vector-icons glyph maps rendered as icon-font text.
$iconProject = sys_get_temp_dir().'/pam-icon-'.bin2hex(random_bytes(4));
mkdir($iconProject.'/assets/fonts', 0o777, true);
file_put_contents($iconProject.'/assets/fonts/Ionicons.ttf', 'ttf');
file_put_contents($iconProject.'/assets/fonts/Ionicons.json', '{"heart":61744,"chatbubble-ellipses":61695}');
$previousDirectory = getcwd();
chdir($iconProject);
try {
    $icon = $typographyRender('<Icon name="heart" size="24" class="accent" />', '.accent { color: #1B7A4E; }');
    $iconProperties = $icon->properties();
    $assert(
        $iconProperties[PropKey::Text->value] === mb_chr(61744, 'UTF-8')
            && $iconProperties[PropKey::FontFamily->value] === 'asset://assets/fonts/Ionicons.ttf'
            && (float) $iconProperties[PropKey::FontSize->value] === 24.0
            && $iconProperties[PropKey::TextColor->value] === 0xFF1B7A4E
            && $iconProperties[PropKey::TextAllowFontScaling->value] === false,
        'Icon must render its glyph with the icon font, size and CSS color.',
    );
    // `<i class="pi">` inherits the parent's computed letter-spacing (em
    // resolved at the parent's font size) but keeps its own font size.
    $spaced = $typographyRender(
        '<Row class="nav"><Icon name="heart" size="16.8" /><Text>Home</Text></Row>',
        '.nav { font-size: 9px; letter-spacing: 0.08em; color: #7D90AC; }',
    );
    $spacedIcon = $spaced->children()[0]->properties();
    $assert(
        abs((float) ($spacedIcon[PropKey::LetterSpacing->value] ?? 0) - 0.72) < 1e-9
            && (float) $spacedIcon[PropKey::FontSize->value] === 16.8
            && $spacedIcon[PropKey::TextColor->value] === 0xFF7D90AC,
        'Icon must inherit the parent letter-spacing (computed px) and colour, not its font size: '.json_encode($spacedIcon),
    );
    $assert(\Pam\Desktop\UI\Icon::make('missing')->properties()[PropKey::Text->value] === '?', 'Unknown glyphs render "?" like React Native.');
    \Pam\Desktop\UI\Icon::register('Brand', 'asset://assets/fonts/Brand.ttf', ['logo' => 0xE001]);
    $assert(\Pam\Desktop\UI\Icon::has('logo', 'Brand') && !\Pam\Desktop\UI\Icon::has('heart', 'Brand'), 'Registered icon fonts use their own glyph maps.');
} finally {
    chdir((string) $previousDirectory);
    array_map('unlink', glob($iconProject.'/assets/fonts/*') ?: []);
    rmdir($iconProject.'/assets/fonts');
    rmdir($iconProject.'/assets');
    rmdir($iconProject);
}

// Media events: ready carries the natural size; buffering/load start events.
$mediaLog = [];
$player = \Pam\Desktop\UI\MediaPlayer::make('https://example.test/video.mp4')
    ->onReady(static function (\Pam\Desktop\MediaReadyEvent $event) use (&$mediaLog): void { $mediaLog[] = "ready:{$event->naturalWidth}x{$event->naturalHeight}"; })
    ->onBuffering(static function (bool $buffering) use (&$mediaLog): void { $mediaLog[] = $buffering ? 'buffering' : 'playing'; })
    ->onLoadStart(static function () use (&$mediaLog): void { $mediaLog[] = 'load'; });
$player->events()[EventKind::MediaLoadStart->value]('');
$player->events()[EventKind::MediaReady->value](\Pam\Desktop\Internal\Wire::map(['naturalWidth' => 1080, 'naturalHeight' => 1920, 'duration' => 12.5]));
$player->events()[EventKind::MediaBuffering->value](\Pam\Desktop\Internal\Wire::map(['buffering' => true]));
$assert($mediaLog === ['load', 'ready:1080x1920', 'buffering'], 'Media lifecycle events must decode their payloads: '.implode(',', $mediaLog));
$legacyReady = false;
\Pam\Desktop\UI\MediaPlayer::make('x.mp4')->onReady(static function () use (&$legacyReady): void { $legacyReady = true; })
    ->events()[EventKind::MediaReady->value](\Pam\Desktop\Internal\Wire::map(['naturalWidth' => 1]));
$assert($legacyReady, 'Parameterless onReady handlers keep working.');

// Sticky headers, full-span list rows and keyboard-aware scroll attributes.
$listTemplate = $typographyRender(
    '<VirtualizedList numColumns="2"><Column fullSpan="true" stickyHeader="true"><Text>Header</Text></Column><Column><Text>A</Text></Column></VirtualizedList>',
    '',
);
$listHeader = $listTemplate->children()[0]->properties();
$assert(
    ($listHeader[PropKey::ListFullSpan->value] ?? null) === true && ($listHeader[PropKey::StickyHeader->value] ?? null) === true,
    'List header attributes must compile to native booleans.',
);
// Keyed list sections: rows carry their section, the list the active one.
$sectionTemplate = $typographyRender(
    '<VirtualizedList activeSection="1"><Column stickyHeader="true"><Text>Tabs</Text></Column><Column listSection="0"><Text>A</Text></Column><Column :listSection="1"><Text>B</Text></Column></VirtualizedList>',
    '',
);
$sectionRows = $sectionTemplate->children();
$assert(
    ($sectionTemplate->properties()[PropKey::ListActiveSection->value] ?? null) === '1'
        && !isset($sectionRows[0]->properties()[PropKey::ListSection->value])
        && ($sectionRows[1]->properties()[PropKey::ListSection->value] ?? null) === '0'
        && ($sectionRows[2]->properties()[PropKey::ListSection->value] ?? null) === '1',
    'List sections must compile to native section keys.',
);
$sectionBuilder = \Pam\Desktop\UI\VirtualizedList::make(
    \Pam\Desktop\UI\Column::make()->listSection(7),
)->activeSection('media');
$assert(
    ($sectionBuilder->properties()[PropKey::ListActiveSection->value] ?? null) === 'media'
        && ($sectionBuilder->children()[0]->properties()[PropKey::ListSection->value] ?? null) === '7',
    'The list section builders must send string keys.',
);
$keyboardScroll = $typographyRender('<ScrollView keyboardInset="true"><Column /></ScrollView>', '');
$assert(($keyboardScroll->properties()[PropKey::ScrollKeyboardInset->value] ?? null) === true, 'keyboardInset must reach the scroll view.');
$offsetScroll = $typographyRender('<ScrollView keyboardInset="true" keyboardVerticalOffset="156"><Column /></ScrollView>', '');
$assert(($offsetScroll->properties()[PropKey::KeyboardVerticalOffset->value] ?? null) == 156, 'Keyboard-aware scroll clearance must use the existing keyboard offset property.');
// Blink's line-height conversions (ComputedStyle::ComputedLineHeight): the
// font size rounds to 1/64, the float product truncates to 1/64. The
// rounded font size can land above `font-size × factor` (22.4px × 1.5 is
// 33.609375, not the 33.59375 that `line-height: 33.6px` floors to).
foreach ([[13.12, 1.45, 19.03125], [12.8, 1.45, 18.546875], [12.8, 1 / 0.75, 17.046875], [12.0, 1 / 0.75, 16.0], [24.0, 2 / 1.5, 32.0], [12.8, 1.33333, 17.046875], [12.16, 1.4, 17.015625], [12.8, 1.5, 19.1875], [9.0, 1.5, 13.5], [22.4, 1.5, 33.609375], [13.76, 1.5, 20.640625], [16.8, 1.5, 25.1875]] as [$size, $factor, $expected]) {
    $assert(\Pam\Desktop\Style\LayoutUnit::lineHeight($size, $factor) === $expected, "line-height {$size}px x {$factor} must be {$expected}.");
}
// A built-in tag's unknown attribute is a diagnostic (PAM2401 file:line), not a silent no-op.
$assert(TemplateRenderer::unknownBuiltinAttribute('Text', 'color') === 'unknown attribute Text.color, did you mean textColor?', 'Text color= must suggest textColor.');
$assert(TemplateRenderer::unknownBuiltinAttribute('Text', ':maxLines') === 'unknown attribute Text.maxLines, did you mean numberOfLines?', 'maxLines must suggest numberOfLines.');
$assert(TemplateRenderer::unknownBuiltinAttribute('Row', 'backgroundColr') === 'unknown attribute Row.backgroundColr, did you mean backgroundColor?', 'Typos must suggest the closest attribute.');
foreach ([['Text', 'textColor'], ['Text', ':numberOfLines'], ['Icon', 'color'], ['Row', 'on:press'], ['Column', 'p-if'], ['Image', 'cacheKey'], ['Text', '__pamStyles'], ['FlatList', 'anything']] as [$tag, $attribute]) {
    $assert(TemplateRenderer::unknownBuiltinAttribute($tag, $attribute) === null, "{$tag}.{$attribute} must be accepted.");
}
$unknownAttribute = null;
try {
    \Pam\Desktop\Internal\TemplateContractValidator::validate(\Pam\Desktop\Component::class, TemplateCompiler::compile("<Column>\n  <Text color=\"#fff\">x</Text>\n</Column>", 'Card.pam.php', \Pam\Desktop\LanguageVersion::Language2));
} catch (\RuntimeException $error) {
    $unknownAttribute = $error->getMessage();
}
$assert(is_string($unknownAttribute) && str_contains($unknownAttribute, 'unknown attribute Text.color, did you mean textColor? at Card.pam.php:2'), 'The validator must report file:line: '.var_export($unknownAttribute, true));

