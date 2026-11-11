<?php

declare(strict_types=1);
// Standalone runs use the project autoloader; tests/run.php brings its own.
if (!class_exists(\Pam\Desktop\Protocol::class)) {
    require_once dirname(__DIR__, 3).'/vendor/autoload.php';
}
use Pam\Desktop\{EventKind,NodeKind,PropKey};
use Pam\Desktop\Internal\{TemplateCompiler,TemplateRenderer,Wire};
use Pam\Desktop\UI\Terminal;

$check = static function (bool $ok, string $message): void {
    if (!$ok) {
        throw new RuntimeException($message);
    }
};
$scope = new class () {
    public array $seen = [];
    public array $palette = ['#000000', '#ff0000', 0xFF00FF00];
    public function resized(array $event): void { $this->seen[] = "resize {$event['cols']}x{$event['rows']}"; }
    public function titled(array $event): void { $this->seen[] = "title {$event['title']}"; }
    public function exited(array $event): void { $this->seen[] = "exit {$event['exitCode']}"; }
    public function rang(): void { $this->seen[] = 'bell'; }
};
$template = TemplateCompiler::compile(
    '<Terminal channel="C1" fontSize="15" lineHeight="1.2" background="#101010" foreground="#eeeeee" cursorColor="#ff00ff" :palette="$palette" scrollback="200000" width="640" height="400" on:resize="resized($event)" on:title="titled($event)" on:exit="exited($event)" on:bell="rang" />',
    'terminal.pam',
);
$element = TemplateRenderer::render($template, $scope, [])->toElement();
$properties = $element->properties();
$check($element->kind === NodeKind::Terminal, 'Terminal tag builds a Terminal node.');
$check($properties[PropKey::TerminalChannel->value] === 'C1', 'Channel.');
$check((float) $properties[PropKey::FontSize->value] === 15.0, 'Font size.');
$check($properties[PropKey::BackgroundColor->value] === 0xFF101010 && $properties[PropKey::TextColor->value] === 0xFFEEEEEE, 'Theme colours.');
$check($properties[PropKey::TerminalCursorColor->value] === 0xFFFF00FF, 'Cursor colour.');
$check($properties[PropKey::TerminalPalette->value] === '#000000,#ff0000,#00ff00', 'Palette as a list.');
$check($properties[PropKey::TerminalScrollback->value] === Terminal::MAX_SCROLLBACK, 'Scrollback is capped.');
$check(str_contains((string) $properties[PropKey::FontFamily->value], 'monospace'), 'A monospace stack by default.');
$events = $element->events();
$events[EventKind::TerminalResize->value](Wire::map(['cols' => 80, 'rows' => 24]));
$events[EventKind::TerminalTitle->value](Wire::map(['title' => 'vim']));
$events[EventKind::TerminalExit->value](Wire::map(['exitCode' => 0]));
$events[EventKind::TerminalBell->value](Wire::map([]));
$check($scope->seen === ['resize 80x24', 'title vim', 'exit 0', 'bell'], 'Terminal events reach handlers: '.json_encode($scope->seen));
$check(TemplateRenderer::unknownBuiltinAttribute('Terminal', 'palette') === null, 'palette is a known attribute.');
$check(TemplateRenderer::unknownBuiltinAttribute('Terminal', 'on:resize') === null, 'on:resize is a known event.');
$check(str_contains((string) TemplateRenderer::unknownBuiltinAttribute('Terminal', 'chanel'), 'channel'), 'Typos are diagnosed.');
$size = null;
$built = Terminal::make('C2')->font('Fira Code', 12, 1.1)->theme(palette: ['#111111'])->onResize(static function (int $cols, int $rows) use (&$size): void { $size = [$cols, $rows]; });
$built->events()[EventKind::TerminalResize->value](Wire::map(['cols' => 120, 'rows' => 40]));
$check($size === [120, 40] && $built->properties()[PropKey::FontFamily->value] === 'Fira Code', 'Builder API.');
foreach ([static fn () => Terminal::make(''), static fn () => Terminal::make('x')->theme(background: 'red')] as $invalid) {
    try {
        $invalid();
        throw new LogicException('Accepted an invalid terminal.');
    } catch (InvalidArgumentException) {
    }
}
// Bare method handlers get the decoded map too.
$bare = new class () {
    public array $seen = [];
    public function resized(array $event): void { $this->seen[] = $event; }
    public function exited(array $event): void { $this->seen[] = $event; }
};
$element = TemplateRenderer::render(TemplateCompiler::compile('<Terminal channel="C3" on:resize="resized" on:exit="exited" />', 'bare.pam'), $bare, [])->toElement();
$element->events()[EventKind::TerminalResize->value](Wire::map(['cols' => 90, 'rows' => 30]));
$element->events()[EventKind::TerminalExit->value](Wire::map(['exitCode' => 3]));
$check($bare->seen === [['cols' => 90, 'rows' => 30], ['exitCode' => 3]], 'Bare handlers receive the decoded map: '.json_encode($bare->seen));

// Page text styles do not reach the terminal; its own colours do.
$styled = TemplateCompiler::compile(
    '<Column textColor="#1f2328" fontFamily="Inter"><Terminal channel="C4" foreground="#c0c0c0" background="#0d1117" cursorColor="#58a6ff" selectionColor="#264f78" /><Text>x</Text></Column>',
    'styled.pam',
);
$page = TemplateRenderer::render($styled, $bare, [])->toElement();
$terminal = $page->children()[0];
$props = $terminal->properties();
$check($terminal->kind === NodeKind::Terminal, 'Styled page holds the terminal.');
$check(($page->children()[1]->properties()[PropKey::TextColor->value] ?? null) === 0xFF1F2328, 'The page colour is inherited by text (the test is meaningful).');
$check(($props[PropKey::TextColor->value] ?? null) === 0xFFC0C0C0, 'foreground wins over the page colour: '.dechex((int) ($props[PropKey::TextColor->value] ?? 0)));
$check(($props[PropKey::BackgroundColor->value] ?? null) === 0xFF0D1117 && ($props[PropKey::TerminalCursorColor->value] ?? null) === 0xFF58A6FF && ($props[PropKey::TerminalSelectionColor->value] ?? null) === 0xFF264F78, 'Hex colour attributes are applied.');
$check(str_contains((string) ($props[PropKey::FontFamily->value] ?? ''), 'monospace'), 'The page font does not reach the terminal: '.($props[PropKey::FontFamily->value] ?? ''));
$plain = TemplateRenderer::render(TemplateCompiler::compile('<Column textColor="#1f2328"><Terminal channel="C5" /></Column>', 'plain.pam'), $bare, [])->toElement()->children()[0];
$check(!isset($plain->properties()[PropKey::TextColor->value]), 'Without foreground the terminal keeps its theme colour.');
echo "terminal element ok\n";
