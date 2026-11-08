<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\ViewMeasurement;
use Pam\DesktopUi\Select;
use Pam\DesktopUi\SelectOption;

$loader = require dirname(__DIR__, 3).'/vendor/autoload.php';
$loader->addPsr4('Pam\\DesktopUi\\', dirname(__DIR__).'/src');
App::components(dirname(__DIR__).'/src', sys_get_temp_dir().'/pam-desktop-ui-tests');
function checkSelect(bool $passed, string $message): void { if (!$passed) throw new RuntimeException($message); }
$changed = [];
$select = new Select([new SelectOption(1, 'First'), new SelectOption(2, 'Disabled', true), new SelectOption(3, 'Third')], 1,
    static function (int|string $value) use (&$changed): void { $changed[] = $value; });
$mounted = Pam\Desktop\Testing\mount($select);
checkSelect(str_contains($mounted->text(), 'First'), 'The selected label renders in the template.');
$select->place(new ViewMeasurement(0, 0, 200, 40, 0, 0, 130, 420), new ViewMeasurement(0, 0, 400, 400, 0, 0, 100, 100));
checkSelect($select->popupX === 30.0 && $select->popupY < 320 && $select->popupWidth === 200.0, 'Popup anchors in provider space and flips above near bottom.');
$select->open = true;
$select->highlighted = 0;
$select->key(Wire::map(['key' => 'ArrowDown']));
checkSelect($select->highlighted === 2, 'Keyboard skips disabled choices.');
$select->key(Wire::map(['key' => 'Enter']));
checkSelect($changed === [3] && !$select->open && $select->value === 1, 'Selection emits the integer value and remains controlled.');
$select->open = true;
$select->choose(2);
checkSelect($changed === [3] && $select->open, 'Disabled options cannot emit a change.');
$select->key(Wire::map(['key' => 'Escape']));
checkSelect(!$select->open, 'Escape dismisses without changing the selection.');
$select->open = true;
$select->highlighted = 0;
$mounted->render();
checkSelect(str_contains($mounted->text(), 'First'), 'Open portal renders without parsing errors.');
$mounted->unmount();
checkSelect(!$select->open, 'Unmount closes the control and invalidates pending measurements.');
echo "Select template, popup geometry, keyboard, disabled options and controlled values: passed\n";

$changes = [];
$filterable = new Select([new SelectOption(1, 'Água'), new SelectOption(2, 'Café'), new SelectOption(3, 'Chá', true)], 1,
    static function (int|string|null $value) use (&$changes): void { $changes[] = $value; }, showClear: true, filter: true);
$filterable->search('CAFÉ');
checkSelect(count($filterable->visibleOptions) === 1 && $filterable->visibleOptions[0]->value === 2, 'Filtering is Unicode case-insensitive and preserves option identifiers.');
$filterable->open = true;
$filterable->key(Wire::map(['key' => 'Enter']));
checkSelect($changes === [2], 'Keyboard selects the filtered option, not the original index.');
$filterable->clear();
checkSelect($changes === [2, null] && $filterable->value === 1, 'Clear emits null while the value remains controlled.');
$filterable->enabled = false;
$filterable->clear();
checkSelect(count($changes) === 2, 'Disabled controls cannot clear.');
$filterable->search('missing');
$filterable->open = true;
$filterable->enabled = true;
$filterable->key(Wire::map(['key' => 'ArrowDown']));
$filteredMount = Pam\Desktop\Testing\mount($filterable);
$filteredMount->render();
$filteredMount->unmount();
checkSelect($filterable->visibleOptions === [] && count($changes) === 2, 'Empty filter results are safe to navigate and render.');
echo "Select filtering, null clearing and empty results: passed\n";
$filterable->setOptions([new SelectOption(8, 'Missing result')]);
checkSelect(count($filterable->visibleOptions) === 1 && $filterable->visibleOptions[0]->value === 8, 'Replacing options reapplies the current filter.');
try {
    $filterable->setOptions([new SelectOption(8, 'A'), new SelectOption(8, 'B')]);
    throw new LogicException('Duplicate replacement accepted');
} catch (InvalidArgumentException) {}
checkSelect(count($filterable->options) === 1, 'Invalid replacement cannot corrupt existing choices.');
echo "Select option replacement validation: passed\n";
