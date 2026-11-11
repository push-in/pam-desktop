<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\GestureEvent;
use Pam\Desktop\GestureState;
use Pam\Desktop\GestureType;
use Pam\Desktop\KeyEvent;
use Pam\DesktopUi\SplitView;

$loader = require dirname(__DIR__, 3).'/vendor/autoload.php';
$loader->addPsr4('Pam\\DesktopUi\\', dirname(__DIR__).'/src');
App::components(dirname(__DIR__).'/src', sys_get_temp_dir().'/pam-desktop-split-tests');
$fail = static fn (string $message) => throw new RuntimeException($message);
$pan = static fn (float $dx, float $dy = 0): GestureEvent => new GestureEvent(
    type: GestureType::Pan, state: GestureState::Changed, x: 0, y: 0, pageX: 0, pageY: 0, translationX: $dx, translationY: $dy,
    velocityX: 0, velocityY: 0, scale: 1, rotation: 0, pointerCount: 1, timestamp: 0,
);

$split = new SplitView(240, min: 170, max: 480);
$mounted = Pam\Desktop\Testing\mount($split);
try {
    $mounted->render();
    $split->fixed(0) && !$split->fixed(1) || $fail('The start pane has the size.');
    $split->begin();
    $split->drag($pan(60));
    $split->size === 300.0 || $fail('Dragging right grows the start pane: '.$split->size);
    $split->drag($pan(400));
    $split->size === 480.0 || $fail('The size stops at max.');
    $split->drag($pan(-200));
    $split->size === 170.0 || $fail('The size stops at min, from where the drag began.');
    $split->end();
    $split->dragging && $fail('Releasing ends the drag.');
    $split->key(new KeyEvent('ArrowRight'));
    $split->size === 180.0 || $fail('An arrow moves the divider by a step.');
    $split->key(new KeyEvent('End'));
    $split->size === 480.0 || $fail('End jumps to max.');

    $panel = new SplitView(200, direction: 'vertical', sized: 'end', min: 100, max: 400);
    $panel->begin();
    $panel->drag($pan(0, -50));
    $panel->size === 250.0 || $fail('Dragging up grows a bottom panel: '.$panel->size);
    $panel->key(new KeyEvent('ArrowUp'));
    $panel->size === 260.0 || $fail('Up grows a bottom panel.');

    $fixed = new SplitView(200, resizable: false);
    $fixed->begin();
    $fixed->drag($pan(80));
    $fixed->size === 200.0 || $fail('A fixed split does not move.');
    try {
        new SplitView(200, direction: 'diagonal');
        $fail('Unknown directions throw.');
    } catch (InvalidArgumentException) {
    }
} finally {
    $mounted->unmount();
}
echo "SplitView: drag within min and max, keyboard, end pane, vertical, fixed: passed\n";
