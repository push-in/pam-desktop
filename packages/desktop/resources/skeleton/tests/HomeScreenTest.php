<?php

declare(strict_types=1);

use App\Screens\Home\HomeScreen;
use Pam\Desktop\App;

use function Pam\Desktop\Testing\mount;

require dirname(__DIR__).'/vendor/autoload.php';

App::components(dirname(__DIR__).'/src', dirname(__DIR__).'/.pam-desktop/components');

function check(bool $passed, string $what): void
{
    if (!$passed) {
        fwrite(STDERR, "failed: {$what}\n");
        exit(1);
    }
}

$home = mount(HomeScreen::class);
check(str_contains($home->text(), '0 clicks'), 'the home screen starts at zero');
$home->instance->increment();
$home->render();
check(str_contains($home->text(), '1 click'), 'a press is counted');
$home->unmount();

echo "ok\n";
