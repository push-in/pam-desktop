<?php

declare(strict_types=1);

use Pam\Desktop\EventKind;
use Pam\Desktop\Internal\TreeEncoder;
use Pam\Desktop\PropKey;
use Pam\Desktop\Protocol;
use Pam\Desktop\ProtocolMismatchException;
use Pam\Desktop\RendererContract;
use Pam\Desktop\UI\Text;
use Pam\Desktop\UI\View as ExtensionView;

$assert ??= static function (bool $condition, string $message): void {
    if (!$condition) throw new RuntimeException($message);
};
$throws = static function (Closure $body, string $needle) use ($assert): void {
    try {
        $body();
    } catch (ProtocolMismatchException $error) {
        $assert(str_contains($error->getMessage(), $needle), "Unexpected protocol error: {$error->getMessage()}");

        return;
    }
    $assert(false, "Expected a protocol mismatch mentioning {$needle}.");
};

// --- Tables: closed core, extension from 1001, same u16 wire field ----------
$assert(
    Protocol::CORE_PROPERTY_LIMIT === 543 && Protocol::CORE_EVENT_LIMIT === 80
        && Protocol::EXTENSION_BASE === 1001 && Protocol::EXTENSION_LIMIT === 65_535,
    'The core tables close at 543/80 and the extension table starts at 1001.',
);
$assert(
    PropKey::ListFollowEndOnResize->value === 1001 && PropKey::ScrollPageTransform->value === 1002
        && PropKey::MediaStartBufferMs->value === 1003 && PropKey::MediaIdleSurface->value === 1004
        && PropKey::ScrollOffsetAnimationMs->value === 1005 && PropKey::MediaAutoPlayVisible->value === 1006
        && PropKey::MediaAutoPlayGroup->value === 1007,
    'The extension table starts with the port reservations 1001-1007.',
);
$assert(
    PropKey::ListStackFromEnd->value === 1011
        && (\Pam\Desktop\UI\FlatList::make(['a'])->stackFromEnd()->properties()[1011] ?? null) === true
        && (\Pam\Desktop\UI\FlatList::make(['a'])->stackFromEnd(false)->properties()[1011] ?? null) === false,
    'FlatList::stackFromEnd() sends ListStackFromEnd (1011).',
);
$local = RendererContract::local();
$coreMax = max(array_filter(array_map(static fn (PropKey $key): int => $key->value, PropKey::cases()), static fn (int $v): bool => $v < 1001));
$assert(
    $local->propertyCeiling === $coreMax && $local->extensionPropertyCeiling === PropKey::TextSpanBoxes->value
        && $local->eventCeiling === max(array_filter(array_map(static fn (EventKind $kind): int => $kind->value, EventKind::cases()), static fn (int $v): bool => $v < 1001))
        && $local->extensionEventCeiling === EventKind::DragLeave->value,
    'The local contract declares the highest core and extension identifiers of the SDK tables.',
);

// --- Contract exchange --------------------------------------------------------
$legacy = RendererContract::fromJson(null);
$assert(
    !$legacy->declared && $legacy->propertyCeiling === 543 && $legacy->extensionPropertyCeiling === 1000
        && $legacy->eventCeiling === 80 && $legacy->extensionEventCeiling === 1000,
    'A host that exports no contract keeps the full core table and gets no extension IDs.',
);
$golden = '{"abi":1,"events":[79,1000],"properties":[540,1001],"protocol":[1,1]}';
$parsed = RendererContract::fromJson($golden);
$assert($parsed->declared && $parsed->toJson() === $golden, 'The contract JSON round-trips byte for byte (host golden).');
$assert(
    RendererContract::fromJson('{"protocol":[1,1],"abi":1,"properties":[540,1001],"events":[79,1000],"future":true}')->toJson() === $golden,
    'Key order and unknown future fields do not change the contract.',
);
foreach ([
    '{', '[]', '{"abi":1}', '{"abi":"1","events":[79,1000],"properties":[540,1001],"protocol":[1,1]}',
    '{"abi":1,"events":[79],"properties":[540,1001],"protocol":[1,1]}',
    '{"abi":1,"events":[79,1000],"properties":[544,1001],"protocol":[1,1]}',
    '{"abi":1,"events":[79,999],"properties":[540,1001],"protocol":[1,1]}',
    '{"abi":1,"events":[79,1000],"properties":[540,65536],"protocol":[1,1]}',
    '{"abi":1,"events":[79,1000],"properties":[540,1001],"protocol":[2,1]}',
    str_repeat(' ', 1_025),
] as $malformed) {
    $throws(static fn () => RendererContract::fromJson($malformed), 'renderer contract');
}

// --- Boot handshake -------------------------------------------------------------
$parsed->assertCompatible();
$legacy->assertCompatible();
$throws(
    static fn () => RendererContract::fromJson('{"abi":2,"events":[79,1000],"properties":[540,1001],"protocol":[1,1]}')->assertCompatible(),
    'ABI 2 is incompatible',
);
$throws(
    static fn () => RendererContract::fromJson('{"abi":1,"events":[79,1000],"properties":[540,1001],"protocol":[2,3]}')->assertCompatible(),
    'protocol ranges do not overlap',
);

// --- Encoder: extension IDs only reach renderers that declare them --------------
$withExtension = static fn (): ExtensionView => (fn (): ExtensionView => $this->withProperty(PropKey::ListFollowEndOnResize, true))
    ->call(ExtensionView::make(Text::make('rows')));
$coreOnly = static fn (): ExtensionView => ExtensionView::make(Text::make('rows'))->key('core');

try {
    RendererContract::use($legacy);
    $frame = (new TreeEncoder())->encode($coreOnly())['frame'];
    $assert(is_string($frame) && $frame !== '', 'An old renderer keeps receiving apps that use only core IDs.');
    $throws(
        static fn () => (new TreeEncoder())->encode($withExtension()),
        'does not implement property ListFollowEndOnResize (1001); it predates the extension table',
    );

    RendererContract::use(RendererContract::fromJson('{"abi":1,"events":[79,1000],"properties":[540,1000],"protocol":[1,1]}'));
    $throws(
        static fn () => (new TreeEncoder())->encode($withExtension()),
        'it declares core IDs up to 540 and no extension IDs',
    );

    RendererContract::use($parsed);
    $frame = (new TreeEncoder())->encode($withExtension())['frame'];
    $assert(
        is_string($frame) && str_contains($frame, pack('v', 1001)."\x04\x01"),
        'A renderer that declares 1001 receives it in the ordinary u16 property field.',
    );

    $decimalInput = \Pam\Desktop\UI\Input::make('0.5')->decimal(2, 3, '', ' kg', 'en-US');
    $previousDesktop = RendererContract::fromJson('{"abi":1,"events":[80,1000],"properties":[543,2022],"protocol":[1,1]}');
    RendererContract::use($previousDesktop);
    $throws(
        static fn () => (new TreeEncoder())->encode($decimalInput),
        'does not implement property InputFormatDecimalMinDigits (2023)',
    );
    RendererContract::use(RendererContract::local());
    $decimalFrame = (new TreeEncoder())->encode($decimalInput)['frame'];
    $assert(
        is_string($decimalFrame) && str_contains($decimalFrame, pack('v', 2023)."\x02".pack('q', 2)),
        'The decimal formatter sends its minimum precision only to a renderer that declares it.',
    );

    // Inline boxes of nested texts (2028) are refused by a 2027 renderer.
    $kbd = Text::rich('a ', Text::make('Ctrl')->property(PropKey::PaddingHorizontal, 6), ' b');
    RendererContract::use(RendererContract::fromJson('{"abi":1,"events":[80,2007],"properties":[543,2027],"protocol":[1,1]}'));
    $throws(static fn () => (new TreeEncoder())->encode($kbd), 'does not implement property TextSpanBoxes (2028)');
    RendererContract::use(RendererContract::local());
    $assert(is_string((new TreeEncoder())->encode($kbd)['frame']), 'A 2028 renderer receives inline boxes.');

    // A patch that adds the property on a later frame is checked too.
    RendererContract::use($legacy);
    $encoder = new TreeEncoder();
    $encoder->encode(ExtensionView::make(Text::make('rows')));
    $throws(static fn () => $encoder->encode($withExtension()), 'ListFollowEndOnResize (1001)');

    // Events: core kinds pass, undeclared extension kinds are refused.
    $assert($legacy->supportsEvent(EventKind::Press->value) && !$legacy->supportsEvent(1001), 'Event ceilings follow the contract.');
    $throws(static fn () => $legacy->requireEvent(1001), 'does not implement event unknown (1001)');
    $assert(
        $parsed->supportsProperty(540) && !$parsed->supportsProperty(541) && !$parsed->supportsProperty(1000)
            && $parsed->supportsProperty(1001) && !$parsed->supportsProperty(1002) && !$parsed->supportsProperty(0),
        'A declared renderer accepts exactly its core and extension ranges.',
    );
} finally {
    RendererContract::use(null);
}
$gutter = \Pam\Desktop\Internal\ScopedStyleCompiler::compile('.page { scrollbar-gutter: stable; scrollbar-width: thin; } .both { scrollbar-gutter: stable both-edges; scrollbar-width: none; }', 'ScrollbarGutterTest');
$assert(
    PropKey::ScrollbarGutter->value === 2024 && PropKey::ScrollbarWidth->value === 2025
        && ($gutter['classes']['page']['scrollbarGutter'] ?? null) === 2 && ($gutter['classes']['page']['scrollbarWidth'] ?? null) === 2
        && ($gutter['classes']['both']['scrollbarGutter'] ?? null) === 3 && ($gutter['classes']['both']['scrollbarWidth'] ?? null) === 3,
    'scrollbar-gutter/scrollbar-width compile to the sequential integer variants (2024/2025).',
);
$thick = \Pam\Desktop\Internal\ScopedStyleCompiler::compile('.column { scrollbar-gutter: stable; scrollbar-width: 8px; }', 'ScrollbarThicknessTest');
$assert(($thick['classes']['column']['scrollbarThickness'] ?? null) === 8.0 && PropKey::ScrollbarThickness->value === 2026, 'scrollbar-width accepts an exact px gutter (2026).');
$colored = \Pam\Desktop\Internal\ScopedStyleCompiler::compile('.column { scrollbar-color: rgba(255, 255, 255, 0.08) transparent; }', 'ScrollbarColorTest');
$assert(($colored['classes']['column']['scrollbarColor'] ?? null) === 0x14FFFFFF && PropKey::ScrollbarColor->value === 2027, 'scrollbar-color compiles the thumb colour (2027).');
