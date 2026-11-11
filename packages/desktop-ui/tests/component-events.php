<?php

declare(strict_types=1);

use Fixtures\Events\EventHost;
use Pam\Desktop\App;
use Pam\Desktop\Internal\PamPhpRegistry;
use Pam\DesktopUi\Popover;
use Pam\DesktopUi\SplitView;
use Pam\DesktopUi\TreeView;

$loader = require dirname(__DIR__, 3).'/vendor/autoload.php';
$loader->addPsr4('Pam\\DesktopUi\\', dirname(__DIR__).'/src');
$loader->addPsr4('Fixtures\\Events\\', __DIR__.'/Fixtures/Events');
App::components(dirname(__DIR__).'/src', sys_get_temp_dir().'/pam-desktop-ui-events');
App::components(__DIR__.'/Fixtures/Events', sys_get_temp_dir().'/pam-desktop-ui-events-host');
$fail = static fn (string $message) => throw new RuntimeException($message);
/** @template T @param class-string<T> $class @return T */
$child = static function (string $class): object {
    foreach ((new ReflectionProperty(PamPhpRegistry::class, 'instances'))->getValue() ?? [] as $bucket) {
        foreach ((array) $bucket as $instances) {
            foreach ((array) $instances as $instance) {
                if ($instance instanceof $class) {
                    return $instance;
                }
            }
        }
    }
    throw new RuntimeException("No mounted {$class}.");
};

$host = new EventHost();
$mounted = Pam\Desktop\Testing\mount($host);
try {
    $mounted->render();
    $child(SplitView::class)->key(new Pam\Desktop\KeyEvent('ArrowRight'));
    $child(Popover::class)->dismiss();
    $child(TreeView::class)->toggle('src');
    $mounted->render();
    $host->events === [['resize', 210.0], ['close'], ['toggle', 'src', true]]
        || $fail('on:name on a component listens to its events: '.json_encode($host->events));
    $child(SplitView::class)->size === 210.0 || $fail('The owner state flows back into the component.');
} finally {
    $mounted->unmount();
}
echo "Component events: on:resize, on:close and on:toggle reach the owner: passed\n";
