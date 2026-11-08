<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\DesktopUi\MediaTile;

$loader = require dirname(__DIR__, 3).'/vendor/autoload.php';
$loader->addPsr4('Pam\\DesktopUi\\', dirname(__DIR__).'/src');
App::components(dirname(__DIR__).'/src', sys_get_temp_dir().'/pam-media-tile-tests');
$tile = new MediaTile('asset://thumbnail.png', 'Product image');
$events = [];
$tile->__pamConfigure([], [
    'primary' => static function () use (&$events): void {
        $events[] = 'primary';
    },
    'remove' => static function () use (&$events): void {
        $events[] = 'remove';
    },
]);
$mounted = Pam\Desktop\Testing\mount($tile);
try {
    $tile->focus();
    $mounted->render();
    if (!$tile->focused) {
        throw new RuntimeException('Keyboard focus must expose media actions.');
    }
    $tile->makePrimary();
    if ($tile->primary || $events !== ['primary']) {
        throw new RuntimeException('Primary action must remain controlled.');
    }
    $tile->primary = true;
    $tile->makePrimary();
    $tile->remove();
    if ($events !== ['primary', 'remove']) {
        throw new RuntimeException('Current primary is not reselected, but remains removable.');
    }
    $tile->enabled = false;
    $tile->remove();
    if (count($events) !== 2) {
        throw new RuntimeException('Disabled media must not emit mutations.');
    }
    $tile->enter();
    $tile->paused();
    if ($tile->focused || $tile->hovered) {
        throw new RuntimeException('Hidden media must clear transient interaction state.');
    }
    echo "MediaTile controlled actions, keyboard exposure, disabled guard and pause passed.\n";
} finally {
    $mounted->unmount();
}
