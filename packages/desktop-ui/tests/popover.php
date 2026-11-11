<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Internal\PortalRegistry;
use Pam\Desktop\LayoutEvent;
use Pam\Desktop\UI\Portal;
use Pam\Desktop\UI\View;
use Pam\Desktop\UI\WindowLayer;
use Pam\Desktop\ViewMeasurement;
use Pam\DesktopUi\Popover;
use Pam\DesktopUi\PopoverPlacement;
use Pam\DesktopUi\Tooltip;

$loader = require dirname(__DIR__, 3).'/vendor/autoload.php';
$loader->addPsr4('Pam\\DesktopUi\\', dirname(__DIR__).'/src');
App::components(dirname(__DIR__).'/src', sys_get_temp_dir().'/pam-desktop-popover-tests');
$fail = static fn (string $message) => throw new RuntimeException($message);

// Placement: a 200x100 surface under a 40x20 anchor in an 800x600 window.
$below = (new PopoverPlacement('bottom-start'))->place(100, 50, 40, 20, 200, 100, 800, 600);
$below === ['x' => 100.0, 'y' => 76.0, 'side' => 'bottom'] || $fail('bottom-start: '.json_encode($below));
$flipped = (new PopoverPlacement('bottom-start'))->place(100, 540, 40, 20, 200, 100, 800, 600);
$flipped['side'] === 'top' && $flipped['y'] === 434.0 || $fail('No room below flips above: '.json_encode($flipped));
$end = (new PopoverPlacement('top-end'))->place(700, 300, 40, 20, 200, 100, 800, 600);
$end['x'] === 540.0 && $end['y'] === 194.0 || $fail('top-end aligns the right edges: '.json_encode($end));
$clamped = (new PopoverPlacement('bottom-center'))->place(0, 50, 40, 20, 200, 100, 800, 600);
$clamped['x'] === 4.0 || $fail('A surface stays inside the window: '.json_encode($clamped));
$right = (new PopoverPlacement('right-start'))->place(700, 50, 40, 20, 200, 100, 800, 600);
$right['side'] === 'left' && $right['x'] === 494.0 || $fail('No room on the right flips left: '.json_encode($right));
try {
    new PopoverPlacement('under');
    $fail('Unknown placements throw.');
} catch (InvalidArgumentException) {
}

// An open popover draws in the window layer, above everything.
$popover = new Popover(true, 'anchor', placement: 'bottom-start');
$mounted = Pam\Desktop\Testing\mount($popover);
try {
    PortalRegistry::instance()->has(Portal::WINDOW_HOST) || $fail('An open popover portals to the window host.');
    $root = WindowLayer::attach(View::make());
    count($root->children()) === 1 || $fail('The runtime mounts the window layer while a popover is open.');
    $popover->placed && $fail('Nothing shows before the anchor and the surface are measured.');
    $box = new ReflectionProperty(Popover::class, 'anchorBox');
    $box->setValue($popover, new ViewMeasurement(0, 0, 40, 20, 10, 20, 10, 20));
    $popover->measured(new LayoutEvent(0, 0, 120, 80));
    $popover->placed && $popover->x === 10.0 && $popover->y === 46.0 || $fail("Placed under the anchor: {$popover->x},{$popover->y}");

    $popover->open = false;
    $mounted->render();
    PortalRegistry::instance()->has(Portal::WINDOW_HOST) && $fail('A closed popover leaves the window layer.');
    $popover->placed && $fail('Closing forgets the placement.');
} finally {
    $mounted->unmount();
}

$tooltip = new Tooltip('Mention someone', shortcut: 'Ctrl+Shift+2', delay: 0);
$mountedTip = Pam\Desktop\Testing\mount($tooltip);
try {
    $tooltip->enter();
    $tooltip->leave();
    $tooltip->visible && $fail('Leaving hides the tooltip.');
    try {
        new Tooltip('x', placement: 'middle');
        $fail('A tooltip checks its placement.');
    } catch (InvalidArgumentException) {
    }
} finally {
    $mountedTip->unmount();
}
echo "Popover and tooltip: placement and flipping, window layer, measured placement, close: passed\n";
