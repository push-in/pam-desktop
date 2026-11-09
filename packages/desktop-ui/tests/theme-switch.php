<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Appearance;
use Pam\Desktop\AppearanceMode;
use Pam\Desktop\Element;
use Pam\Desktop\PropKey;
use Pam\DesktopUi\Checkbox;
use Pam\DesktopUi\DatePicker;
use Pam\DesktopUi\Select;
use Pam\DesktopUi\SelectOption;
use Pam\DesktopUi\SelectTheme;
use Pam\DesktopUi\Toast;
use Pam\DesktopUi\ToastSeverity;
use Pam\DesktopUi\ToastTheme;

$loader = require dirname(__DIR__, 3).'/vendor/autoload.php';
$loader->addPsr4('Pam\\DesktopUi\\', dirname(__DIR__).'/src');
App::components(dirname(__DIR__).'/src', sys_get_temp_dir().'/pam-desktop-ui-theme-tests');
function checkTheme(bool $passed, string $message): void { if (!$passed) throw new RuntimeException($message); }
/** @return list<int> every background color in the tree */
function backgrounds(Element $element): array
{
    $colors = [];
    $color = $element->properties()[PropKey::BackgroundColor->value] ?? null;
    if (is_int($color)) $colors[] = $color;
    foreach ($element->children() as $child) {
        if ($child instanceof Element) $colors = [...$colors, ...backgrounds($child)];
    }

    return $colors;
}

$light = new SelectTheme(surface: 0xFFFFFFFF, text: 0xFF111111, highlight: 0xFFDDDDFF);
$dark = new SelectTheme(surface: 0xFF101010, text: 0xFFEEEEEE, highlight: 0xFF333366);
$adaptive = SelectTheme::adaptive(light: $light, dark: $dark);
checkTheme(SelectTheme::resolve(null) instanceof SelectTheme && SelectTheme::resolve($light) === $light, 'resolve() keeps a plain theme.');
try { SelectTheme::resolve(static fn (): string => 'x'); throw new LogicException('A closure that returns no theme must be rejected.'); } catch (InvalidArgumentException) {}

Appearance::set(AppearanceMode::Dark);
$select = new Select([new SelectOption(1, 'First')], 1, static function (int|string|null $value): void {}, theme: $adaptive);
$checkbox = new Checkbox(false, static function (bool $value): void {}, theme: $adaptive);
$picker = new DatePicker(null, static function ($value): void {}, theme: $adaptive);
$selectView = Pam\Desktop\Testing\mount($select);
$checkboxView = Pam\Desktop\Testing\mount($checkbox);
$pickerView = Pam\Desktop\Testing\mount($picker);
checkTheme(in_array(0xFF101010, backgrounds($selectView->render()), true), 'The select draws the dark theme in the dark scheme.');
checkTheme(in_array(0xFF101010, backgrounds($checkboxView->render()), true), 'The checkbox draws the dark theme in the dark scheme.');
checkTheme($picker->theme === $dark && $picker->hours->theme === $dark, 'The date picker and its time selects draw the dark theme.');

// A live switch: the same instances draw the light theme on the next render.
Appearance::set(AppearanceMode::Light);
$selectColors = backgrounds($selectView->render());
checkTheme(in_array(0xFFFFFFFF, $selectColors, true) && !in_array(0xFF101010, $selectColors, true), 'The select follows a live switch to light.');
$checkboxColors = backgrounds($checkboxView->render());
checkTheme(in_array(0xFFFFFFFF, $checkboxColors, true) && !in_array(0xFF101010, $checkboxColors, true), 'The checkbox follows a live switch to light.');
$pickerView->render();
$picker->hours->rendering();
checkTheme($picker->theme === $light && $picker->hours->theme === $light, 'The date picker and its time selects follow a live switch.');
$selectView->unmount();
$checkboxView->unmount();
$pickerView->unmount();

// Toasts on screen follow the switch too.
\Pam\Desktop\UI\Icon::register('PrimeIcons', 'asset://resources/fonts/PrimeIcons.ttf', ['check' => 0xe909, 'times' => 0xe90b, 'info-circle' => 0xe924, 'exclamation-triangle' => 0xe922, 'times-circle' => 0xe90c]);
$toast = new Toast(theme: ToastTheme::adaptive(light: new ToastTheme(surface: 0xFFFAFAFA), dark: new ToastTheme(surface: 0xFF0A0A0A)));
$toastView = Pam\Desktop\Testing\mount($toast);
$toast->add(ToastSeverity::Info, 'Saved', life: null);
$toastView->render();
$message = $toast->messages[0];
checkTheme($toast->surface($message) === 0xFFFAFAFA, 'The toast draws the light theme.');
Appearance::set(AppearanceMode::Dark);
$toastView->render();
checkTheme($toast->surface($message) === 0xFF0A0A0A, 'A toast on screen follows a live switch to dark.');
$toastView->unmount();
Appearance::set(AppearanceMode::System);

echo "Select, Checkbox, DatePicker and Toast themes follow a live light/dark switch: passed\n";
