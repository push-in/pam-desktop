<?php

declare(strict_types=1);
// Standalone runs use the project autoloader; tests/run.php brings its own.
if (!class_exists(\Pam\Desktop\Protocol::class)) {
    require_once dirname(__DIR__, 3).'/vendor/autoload.php';
}
use Pam\Desktop\{EventKind,MenuItem,PropKey};
use Pam\Desktop\Internal\{TemplateCompiler,TemplateRenderer,Wire};

$scope = new class () {
    public array $ran = [];
    /** @return list<MenuItem> */
    public function menu(string $name): array
    {
        return [
            MenuItem::make('Rename', fn () => $this->ran[] = "rename {$name}", shortcut: 'F2'),
            MenuItem::separator(),
            MenuItem::submenu('Share', MenuItem::make('Copy link', fn () => $this->ran[] = 'link', shortcut: 'CmdOrCtrl+Shift+C')),
        ];
    }
};
$template = TemplateCompiler::compile('<Row :contextMenu="menu(\'notes.md\')"><Text>notes.md</Text></Row>', 'context-menu.pam');
$element = TemplateRenderer::render($template, $scope, [])->toElement();
$raw = $element->properties()[PropKey::ContextMenuItems->value] ?? null;
$json = is_object($raw) && property_exists($raw, 'bytes') ? $raw->bytes : (string) $raw;
$items = json_decode($json, true, flags: JSON_THROW_ON_ERROR);
if (($items[0]['shortcut'] ?? null) !== 'F2' || ($items[2]['children'][0]['shortcut'] ?? null) !== 'CmdOrCtrl+Shift+C') {
    throw new RuntimeException('Context menu shortcuts must reach the host: '.$json);
}
$element->events()[EventKind::MenuAction->value](Wire::map(['id' => $items[0]['id']]));
if ($scope->ran !== ['rename notes.md']) {
    throw new RuntimeException('The chosen item must run its handler.');
}
try {
    TemplateRenderer::render(TemplateCompiler::compile('<Row :contextMenu="[\'Rename\']" />', 'bad-menu.pam'), $scope, [])->toElement();
    throw new RuntimeException('A context menu of strings must be refused.');
} catch (InvalidArgumentException) {
}
echo "context menu template ok\n";
