<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\GestureEvent;
use Pam\Desktop\GestureState;
use Pam\Desktop\GestureType;
use Pam\Desktop\KeyEvent;
use Pam\Desktop\LayoutEvent;
use Pam\Desktop\PressEvent;
use Pam\DesktopUi\Slider;

$loader = require dirname(__DIR__, 3).'/vendor/autoload.php';
$loader->addPsr4('Pam\\DesktopUi\\', dirname(__DIR__).'/src');
App::components(dirname(__DIR__).'/src', sys_get_temp_dir().'/pam-desktop-slider-tests');

$changes = [];
$commits = [];
$slider = new Slider(
    0.5,
    static function (float $value) use (&$changes): void { $changes[] = $value; },
    committed: static function (float $value) use (&$commits): void { $commits[] = $value; },
    label: 'Volume',
);
$mounted = Pam\Desktop\Testing\mount($slider);
$fail = static fn (string $message) => throw new RuntimeException($message);
try {
    $slider->measure(new LayoutEvent(0, 0, 200, 16));
    $slider->press(new PressEvent(50, 8, 50, 8, 0, 0));
    $slider->value === 0.25 && $changes === [0.25] || $fail('A click sets the value from the pointer: '.json_encode($changes));
    $slider->active() || $fail('A pressed slider shows its active fill and thumb.');
    $drag = static fn (float $x, GestureState $state): GestureEvent => new GestureEvent(
        type: GestureType::Pan, state: $state, x: $x, y: 8, pageX: $x, pageY: 8, translationX: 0, translationY: 0,
        velocityX: 0, velocityY: 0, scale: 1, rotation: 0, pointerCount: 1, timestamp: 0,
    );
    $slider->drag($drag(300, GestureState::Changed));
    $slider->value === 1.0 || $fail('A drag past the end clamps to max.');
    $slider->dragEnd($drag(-40, GestureState::Ended));
    $slider->value === 0.0 && $commits === [0.0] && !$slider->dragging || $fail('Releasing commits once: '.json_encode($commits));

    $slider->key(new KeyEvent('ArrowRight'));
    abs($slider->value - 0.05) < 1e-9 || $fail('An arrow steps a twentieth of a continuous range.');
    $slider->key(new KeyEvent('End'));
    $slider->value === 1.0 && end($commits) === 1.0 || $fail('End jumps to max and commits.');
    $slider->key(new KeyEvent('ArrowRight', control: true));
    $slider->value === 1.0 || $fail('Shortcuts with modifiers are left to the application.');

    $stepped = new Slider(0, static function (float $value): void {}, min: 0, max: 100, step: 10);
    $stepped->measure(new LayoutEvent(0, 0, 100, 16));
    $stepped->press(new PressEvent(34, 8, 34, 8, 0, 0));
    $stepped->value === 30.0 || $fail('A stepped slider rounds to its step.');

    $slider->max = 4.0;
    $slider->press(new PressEvent(150, 8, 150, 8, 0, 0));
    $slider->release();
    $slider->value === 3.0 || $fail('A new max applies to the same instance, measured once: '.$slider->value);
    $slider->max = 1.0;

    $slider->enabled = false;
    $before = count($changes);
    $slider->press(new PressEvent(10, 8, 10, 8, 0, 0));
    $slider->key(new KeyEvent('Home'));
    count($changes) === $before && !$slider->active() || $fail('A disabled slider does not change.');
    $slider->enabled = true;
    $mounted->render();
    try {
        new Slider(1, static function (float $value): void {}, min: 1, max: 1);
        $fail('min must be below max.');
    } catch (InvalidArgumentException) {
    }
    echo "Controlled slider: click, drag past the ends, commit on release, keyboard steps, step rounding, disabled state: passed\n";
} finally {
    $mounted->unmount();
}
