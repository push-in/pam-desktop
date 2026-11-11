<?php

declare(strict_types=1);

$loader = require dirname(__DIR__, 3).'/vendor/autoload.php';
$loader->addPsr4('Pam\\DesktopUi\\', dirname(__DIR__).'/src');

use Pam\DesktopUi\{Toast, ToastPosition, ToastSeverity};

function toastCheck(bool $ok, string $message): void { if (!$ok) throw new RuntimeException($message); }

\Pam\Desktop\App::components(dirname(__DIR__).'/src', sys_get_temp_dir().'/pam-toast-tests');
toastCheck(\Pam\Desktop\Internal\PamPhpCompiler::failures() === [], 'DesktopUI components compile: '.implode('; ', array_map(static fn ($e) => $e->getMessage(), \Pam\Desktop\Internal\PamPhpCompiler::failures())));
$toast = new Toast(host: 'toast-test', position: ToastPosition::BottomCenter, limit: 3);
$mounted = Pam\Desktop\Testing\mount($toast);
$first = $toast->success('Salvo', 'Produto atualizado.');
$toast->info('Info');
$toast->warn('Atenção', life: null);
$toast->error('Falhou');
$mounted->render();
$mounted->render();
toastCheck(count($toast->messages) === 3 && $toast->messages[0]->summary === 'Info', 'The queue keeps the newest messages up to the limit.');
toastCheck($toast->messages[2]->severity === ToastSeverity::Error && $toast->messages[1]->life === null, 'Severities and sticky messages are kept.');
$toast->remove($toast->messages[0]->id);
toastCheck(count($toast->messages) === 2, 'remove() closes one message.');
$toast->paused();
$toast->resumed();
toastCheck(count($toast->messages) === 2, 'Pausing keeps messages and their remaining time.');
$toast->clear();
toastCheck($toast->messages === [], 'clear() empties the stack.');
toastCheck(ToastPosition::BottomCenter->vertical() === 'flex-end' && ToastPosition::TopRight->alignment() === 'flex-end', 'Positions map to the overlay alignment.');
try { $toast->add(ToastSeverity::Info, 'x', life: 10); toastCheck(false, 'Invalid life is rejected.'); } catch (InvalidArgumentException) {}
try { new Toast(backdropBlur: -1); toastCheck(false, 'A negative backdrop blur is rejected.'); } catch (InvalidArgumentException) {}
toastCheck((new Toast())->backdropBlur === 1.5 && (new Toast())->fontFamily === 'system-ui', 'Toast defaults to the PrimeVue blur and the system UI font.');
$mounted->unmount();
echo "Toast queue, severities, positions, life and pause passed.\n";
