<?php

declare(strict_types=1);

use Pam\Desktop\Canvas\Canvas;
use Pam\Desktop\Canvas\CanvasView;
use Pam\Desktop\Canvas\DisplayCommandKind;
use Pam\Desktop\Canvas\FontWeight;
use Pam\Desktop\Canvas\Internal\DesktopCommands;
use Pam\Desktop\Canvas\LineCap;
use Pam\Desktop\Canvas\Path;
use Pam\Desktop\Canvas\TextAlign;

require dirname(__DIR__, 3).'/vendor/autoload.php';
spl_autoload_register(static function (string $class): void {
    $prefix = 'Pam\\Desktop\\Canvas\\';
    if (!str_starts_with($class, $prefix)) return;
    $path = dirname(__DIR__).'/src/'.str_replace('\\', '/', substr($class, strlen($prefix))).'.php';
    if (is_file($path)) require $path;
});
function canvasCheck(bool $value, string $message): void { if (!$value) throw new RuntimeException($message); }
canvasCheck(array_map(static fn (DisplayCommandKind $kind): int => $kind->value, DisplayCommandKind::cases()) === range(1, 13), 'Scene variants are sequential.');
$path = Path::start(0, 0)->quadTo(10, 0, 10, 10)->curveTo(10, 20, 20, 20, 20, 10)->close();
$canvas = (new Canvas())->fillRect(0, 0, 100, 100, '#fff')
    ->roundRect(2, 2, 10, 10, 2, '#12345680')->circle(5, 5, 2, '#ff0000')
    ->line(0, 1, 10, 1, '#fff')->path($path, '#fff')
    ->gradientPath($path, 0, 0, 0, 100, '#abcdef80', '#abcdef00')
    ->arc(20, 20, 15, 0, 90, '#fff', 2, LineCap::Round)
    ->sector(40, 40, 20, 15, 0, 360, '#123456')
    ->polyline([[0.0, 0.0], [10.0, 10.0]], '#fff')->polygon([[0.0, 0.0], [0.0, 10.0], [10.0, 10.0]], '#fff')
    ->dashedLine(0, 10, 30, 10, '#fff', 1, 3, 3)
    ->text('Português', 2, 14, 12, '#fff')->label('R$ 10,00', 50, 14, 12, '#fff', TextAlign::Right, FontWeight::Bold);
$commands = json_decode(DesktopCommands::encode($canvas->scene()), true, flags: JSON_THROW_ON_ERROR);
canvasCheck($commands[1]['color'] === 0x80123456, 'CSS alpha is converted to ARGB.');
canvasCheck($commands[5]['gradient']['end'] === 0x00ABCDEF, 'Gradient alpha preserved.');
canvasCheck($commands[count($commands) - 1]['weight'] === 700 && $commands[count($commands) - 1]['align'] === 3, 'Text maps weight/alignment to native font.');
canvasCheck(str_contains($commands[9]['path'], 'C'), 'Donut sector contains native cubic curves.');
$element = CanvasView::make($canvas->scene())->toElement();
canvasCheck($element->kind() === \Pam\Desktop\NodeKind::Canvas, 'Scene is a native Canvas node.');
foreach ([static fn () => Path::fromSpec('M0 0 Q1'), static fn () => Path::fromSpec('M0 0 L1e999 0'), static fn () => DesktopCommands::encode((new Canvas())->dashedLine(0, 0, 10000, 0, '#fff', 1, 0.01, 0.01)->scene())] as $invalid) {
    try { $invalid(); } catch (InvalidArgumentException) { continue; }
    throw new RuntimeException('Unbounded/malformed drawing was accepted.');
}
echo "Canvas native protocol, curves/gradients/arc/text and bounds: passed\n";
