<?php

declare(strict_types=1);
use Pam\Desktop\App;
use Pam\Desktop\NodeKind;
use Pam\Desktop\PropKey;
use Pam\DesktopUi\ProgressButton;
$loader = require dirname(__DIR__, 3).'/vendor/autoload.php';
$loader->addPsr4('Pam\\DesktopUi\\', dirname(__DIR__).'/src');
App::components(dirname(__DIR__).'/src', sys_get_temp_dir().'/pam-progress-button-tests');
$button = new ProgressButton(
    'Entrar no sistema',
    backgroundColor: 0xFF2563EB,
    loadingBackgroundColor: 0xFF1D5E4D,
    loadingTextColor: 0xFF13181E,
    loadingIndicatorColor: 0xFF111821,
);
$presses = 0;
$button->__pamConfigure([], ['press' => static function () use (&$presses): void { $presses++; }]);
$mounted = Pam\Desktop\Testing\mount($button);
try {
    $button->press();
    $button->loading = true;
    $element = $mounted->render();
    $button->press();
    if ($presses !== 1 || !str_contains($mounted->text(), 'Entrar no sistema')) throw new RuntimeException('Busy action must preserve its label and reject another press.');
    if ($element->properties()[PropKey::AccessibilityBusy->value] !== true || $element->properties()[PropKey::Enabled->value] !== false) throw new RuntimeException('Busy accessibility state is missing.');
    if ($element->properties()[PropKey::BackgroundColor->value] !== 0xFF1D5E4D || $element->properties()[PropKey::Opacity->value] !== 1) throw new RuntimeException('Busy visual style should preserve its explicit colors and opacity.');
    $button->loading = false;
    $button->enabled = false;
    $button->press();
    if ($presses !== 1) throw new RuntimeException('Disabled action emitted a press.');
    $mounted->render();
    echo "ProgressButton retained label, busy accessibility, reentry and disabled guards passed.\n";
} finally { $mounted->unmount(); }
