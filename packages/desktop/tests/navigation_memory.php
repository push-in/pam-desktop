<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Component;
use Pam\Desktop\Diagnostics\Profiler;
use Pam\Desktop\Internal\ComponentLifecycle;
use Pam\Desktop\Internal\Runtime;
use Pam\Desktop\Internal\TreeEncoder;
use Pam\Desktop\Navigation\Router;
use Pam\Desktop\Renderable;
use Pam\Desktop\UI\Text;

// Never restore a navigator checkpoint left by another process or test.
$stateDirectory = sys_get_temp_dir().'/pam-navigation-memory-'.bin2hex(random_bytes(8));
if (!mkdir($stateDirectory, 0700, true)) throw new RuntimeException('Cannot isolate navigation state.');
putenv('PAM_DESKTOP_STATE_DIR='.$stateDirectory);
register_shutdown_function(static function () use ($stateDirectory): void {
    foreach (glob($stateDirectory.'/*') ?: [] as $path) {
        if (is_file($path)) unlink($path);
    }
    rmdir($stateDirectory);
});

$assert = require __DIR__.'/arch_v2_bootstrap.php';
Runtime::shutdown();

final class NavigationMemoryProbe extends Component
{
    public function __construct(private readonly int $visit) {}

    public function render(): Renderable
    {
        // Fresh integer values exercise encoding independently of route keys.
        return Text::make('probe')->numberOfLines($this->visit);
    }
}

(static function () use ($assert): void {
    $reference = null;
    $visit = 10000;
    $navigator = Router::stack('home')
        ->route('home', static fn () => Text::make('home'))
        ->route('probe', static function () use (&$reference, &$visit) {
            $screen = new NavigationMemoryProbe(++$visit);
            $reference = WeakReference::create($screen);
            return $screen;
        })->build();
    App::run($navigator);
    Profiler::enabled(false); // Diagnostic history is separate from navigation ownership.
    $states = new ReflectionProperty(ComponentLifecycle::class, 'states');
    $integers = new ReflectionProperty(TreeEncoder::class, 'encodedIntegers');
    $baseline = 0;
    $baseObjects = 0;
    try {
        for ($cycle = 0; $cycle < 520; ++$cycle) {
            $navigator->push('probe');
            Runtime::render();
            $assert($reference?->get() instanceof NavigationMemoryProbe, 'The route must mount a real component.');
            $navigator->pop();
            Runtime::render();
            // Complete the native transition before measuring an unmounted route.
            (new ReflectionMethod($navigator, 'finalizeOutgoingRoute'))->invoke($navigator);
            Runtime::render();
            gc_collect_cycles();
            $assert($reference->get() === null, 'Popped route components must be collected.');
            if ($cycle === 319) {
                $baseline = memory_get_usage();
                $baseObjects = count($states->getValue() ?? []);
            }
        }
        $growth = memory_get_usage() - $baseline;
        $objects = count($states->getValue() ?? []);
        $assert($objects === $baseObjects, 'Lifecycle object count must stay flat after navigation.');
        $assert(count($integers->getValue()) <= 257, 'Fresh IDs must not accumulate in the integer encoding memo.');
        $assert($growth <= 16384, 'Navigation must have near-zero retained growth over 200 cycles: '.$growth.' B.');
        echo 'NAVIGATION_MEMORY_OK cycles=200 bytes='.$growth.' perCycle='.round($growth / 200, 2).' objectsDelta='.($objects - $baseObjects)." weakAlive=0\n";
    } finally {
        Runtime::shutdown();
        Profiler::enabled(true);
    }
})();
