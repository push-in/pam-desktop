<?php

declare(strict_types=1);

use Pam\Desktop\App;

$root = dirname(__DIR__, 3);
$loader = require $root.'/vendor/autoload.php';
$source = dirname(__DIR__).'/resources/skeleton';
$loader->addPsr4('App\\', $source.'/src');
App::components($source.'/src', sys_get_temp_dir().'/pam-desktop-skeleton-components');
$router = \App\Navigation\AppRouter::make();
$rendered = Pam\Desktop\Testing\mount($router);
function requireText(Pam\Desktop\Testing\Mounted $mounted, string $text): void
{
    if (!str_contains($mounted->text(), $text)) throw new RuntimeException('Missing skeleton text: '.$text);
}
function pressWelcome(Pam\Desktop\Element $element): bool
{
    if (($element->properties[Pam\Desktop\PropKey::TestId->value] ?? null) === 'welcome-increment') {
        ($element->events[Pam\Desktop\EventKind::Press->value])('');
        return true;
    }
    foreach ($element->children as $child) {
        if (pressWelcome($child->toElement())) return true;
    }
    return false;
}
requireText($rendered, 'Your native desktop application starts here.');
requireText($rendered, '0 clicks');
if (!pressWelcome($rendered->element())) throw new RuntimeException('Missing skeleton action.');
$rendered->render();
requireText($rendered, '1 clicks');
$rendered->unmount();
echo "Skeleton route, component template and interaction: passed\n";

// Integer routes follow the same normalization in registration and actions.
enum IntegerRoute: int { case First = 1; case Second = 2; }
enum InvalidIntegerRoute: int { case Zero = 0; case Negative = -1; }
$navigator = Pam\Desktop\Routing\Route::stack('integers', IntegerRoute::First, static function (): void {
    Pam\Desktop\Routing\Route::screen(IntegerRoute::First, Pam\Desktop\UI\Text::make('First route'));
    Pam\Desktop\Routing\Route::screen(IntegerRoute::Second, Pam\Desktop\UI\Text::make('Second route'));
}, restoreState: false);
$screen = Pam\Desktop\Testing\mount($navigator);
requireText($screen, 'First route');
$navigator->push(IntegerRoute::Second);
$screen->render();
requireText($screen, 'Second route');
$navigator->replace(IntegerRoute::First);
$screen->render();
requireText($screen, 'First route');
$navigator->reset(IntegerRoute::Second);
$screen->render();
requireText($screen, 'Second route');
foreach (InvalidIntegerRoute::cases() as $invalid) {
    try {
        Pam\Desktop\Routing\RouteName::value($invalid);
        throw new RuntimeException('Invalid integer route accepted.');
    } catch (InvalidArgumentException) {
    }
}
$screen->unmount();
echo "Integer route registration, push, replace, reset and validation: passed\n";
