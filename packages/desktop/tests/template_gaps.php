<?php

declare(strict_types=1);
// Standalone runs use the project autoloader; tests/run.php brings its own.
if (!class_exists(\Pam\Desktop\Protocol::class)) {
    require_once dirname(__DIR__, 3).'/vendor/autoload.php';
}
use Pam\Desktop\PropKey;
use Pam\Desktop\Internal\{TemplateCompiler,TemplateRenderer};

$check = static function (bool $ok, string $message): void {
    if (!$ok) {
        throw new RuntimeException($message);
    }
};
$render = static fn (string $template, array $data = []) => TemplateRenderer::render(TemplateCompiler::compile($template, 'gaps.pam'), null, $data)->toElement();

// min()/max() take numeric strings like PHP's (a value read from a form or a file).
$element = $render('<View :width="max(2, $x)" :height="min(\'40.5\', 50)" />', ['x' => '30']);
$check((float) $element->properties()[PropKey::Width->value] === 30.0, 'max() with a numeric string.');
$check((float) $element->properties()[PropKey::Height->value] === 40.5, 'min() with a numeric string.');
try {
    $render('<View :width="max(2, $x)" />', ['x' => 'abc']);
    throw new LogicException('A non-numeric string was accepted.');
} catch (RuntimeException $error) {
    $check(str_contains($error->getMessage(), 'expects finite numeric values'), 'Non-numeric strings still fail: '.$error->getMessage());
}

// Eight-digit hex colours: #AARRGGBB in attributes (the integer order),
// #RRGGBBAA in stylesheets (CSS). rgba() means the same in both.
$modal = $render('<Modal visible="true" backdropColor="#B3010409"><View /></Modal>');
$check($modal->properties()[PropKey::ModalBackdropColor->value] === 0xB3010409, 'Attribute #AARRGGBB.');
$modal = $render('<Modal visible="true" backdropColor="rgba(1, 4, 9, 0.7)"><View /></Modal>');
$check($modal->properties()[PropKey::ModalBackdropColor->value] === 0xB3010409, 'Attribute rgba().');
echo "template gaps ok\n";
