<?php

declare(strict_types=1);

$loader = require dirname(__DIR__, 3).'/vendor/autoload.php';
$loader->addPsr4('Pam\\DesktopUi\\', dirname(__DIR__).'/src');

use Pam\Desktop\Component;
use Pam\Desktop\Element;
use Pam\Desktop\ModalPresentation;
use Pam\Desktop\NodeKind;
use Pam\Desktop\PropKey;
use Pam\Desktop\UI\{Modal, Text, View, WindowLayer};
use Pam\DesktopUi\{Toast, ToastService, ToastSeverity};

function serviceCheck(bool $ok, string $message): void { if (!$ok) throw new RuntimeException($message); }

/** @return list<Element> every element of a rendered tree */
function serviceElements(Element $element): array
{
    $all = [$element];
    foreach ($element->children() as $child) {
        if ($child instanceof Element) array_push($all, ...serviceElements($child));
    }
    return $all;
}

\Pam\Desktop\App::components(dirname(__DIR__).'/src', sys_get_temp_dir().'/pam-toast-service-tests');
serviceCheck(\Pam\Desktop\Internal\PamPhpCompiler::failures() === [], 'DesktopUI components compile.');
\Pam\Desktop\UI\Icon::register('PrimeIcons', 'asset://resources/fonts/PrimeIcons.ttf', ['check' => 0xe909, 'times' => 0xe90b, 'info-circle' => 0xe924, 'exclamation-triangle' => 0xe922, 'times-circle' => 0xe90c]);

final class ToastServiceScreen implements \Pam\Desktop\Renderable
{
    public bool $dialog = false;
    public string $route = 'a';
    public function toElement(): Element
    {
        return View::make(
            Text::make('route '.$this->route),
            Modal::make(View::make(Text::make('Dialog')), $this->dialog),
        );
    }
}

$screen = new ToastServiceScreen();
$mounted = Pam\Desktop\Testing\mount($screen);
$service = Toast::service();
serviceCheck($service === ToastService::instance(), 'Toast::service() is the window service.');
serviceCheck(!WindowLayer::has('desktop-ui.toast'), 'Nothing is mounted while the stack is empty.');

$screen->dialog = true;
$first = $service->success('Salvo', 'Produto atualizado.');
serviceCheck(str_contains((string) \Pam\Desktop\Internal\Runtime::lastFrame(), 'Produto atualizado.'), 'The stack is committed in the window layer.');
$service->error('Falhou', life: 4500);
$mounted->render();
serviceCheck(WindowLayer::has('desktop-ui.toast'), 'A message mounts the window layer.');
$root = \Pam\Desktop\UI\WindowLayer::attach(View::make());
$layer = $root->children()[0] ?? null;
serviceCheck($layer instanceof Element && $layer->kind() === NodeKind::Modal
    && ($layer->properties()[PropKey::ModalPresentation->value] ?? null) === ModalPresentation::Overlay->value, 'The layer is an Overlay modal next to the root.');
serviceCheck(count($service->messages()) === 2 && $service->messages()[1]->severity === ToastSeverity::Error, 'Severities are queued in one stack.');
serviceCheck(end($service->messages)->summary === 'Falhou', 'The messages property reads like Toast::$messages.');

// Navigation: the screen changes, the stack stays.
$screen->route = 'b';
$screen->dialog = false;
$mounted->render();
serviceCheck(count($service->messages()) === 2, 'Messages survive navigation.');

$service->remove($first);
$service->clear();
$mounted->render();
serviceCheck($service->messages() === [] && !WindowLayer::has('desktop-ui.toast'), 'The last message unmounts the layer.');

$service->info('De novo');
$mounted->render();
serviceCheck(count($service->messages()) === 1 && WindowLayer::has('desktop-ui.toast'), 'A later message mounts a fresh stack.');
ToastService::reset();
$mounted->render();
serviceCheck(!WindowLayer::has('desktop-ui.toast'), 'reset() forgets the stack.');
$mounted->unmount();
echo "ToastService window stack, overlay layer, navigation and lifecycle passed.\n";
