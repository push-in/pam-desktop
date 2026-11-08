<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Internal\Wire;
use Pam\DesktopUi\{AutocompleteOption, AutocompletePage, MultiSelect};

$loader = require dirname(__DIR__, 3).'/vendor/autoload.php';
$loader->addPsr4('Pam\\DesktopUi\\', dirname(__DIR__).'/src');
App::components(dirname(__DIR__).'/src', sys_get_temp_dir().'/pam-desktop-ui-tests');

function multiSelectCheck(bool $ok, string $message): void
{
    if (!$ok) throw new RuntimeException($message);
}

$requests = [];
$changes = [];
$control = new MultiSelect(
    [],
    static function (array $options) use (&$changes): void { $changes[] = array_map(static fn (AutocompleteOption $option): int|string => $option->value, $options); },
    static function (string $query, int $page, Closure $complete, Closure $failed) use (&$requests): Closure {
        $index = count($requests);
        $requests[] = compact('query', 'page', 'complete', 'failed') + ['cancelled' => false];
        return static function () use (&$requests, $index): void { $requests[$index]['cancelled'] = true; };
    },
);
$mounted = Pam\Desktop\Testing\mount($control);
$control->open = true;
$control->search();
multiSelectCheck(count($requests) === 1 && $requests[0]['page'] === 1 && $requests[0]['query'] === '', 'Opening selection fetches only the first bounded page.');
($requests[0]['complete'])(new AutocompletePage([new AutocompleteOption(1, 'Tomate'), new AutocompleteOption(2, 'Queijo')], true));
$control->toggleOption(1);
multiSelectCheck($control->selectedOptions[0]->label === 'Tomate' && $control->open && $control->checkedState(1) === Pam\Desktop\AccessibilityCheckedState::Checked->value, 'Selecting an option preserves it, exposes checked accessibility state and keeps the list open.');
$control->nextPage();
multiSelectCheck(count($requests) === 2 && $requests[1]['page'] === 2, 'Next page loads without accumulating visible rows.');
($requests[1]['complete'])(new AutocompletePage([new AutocompleteOption(3, 'Cebola')], false));
multiSelectCheck(count($control->options) === 1 && $control->selected(1), 'Selection survives result-page replacement.');
$control->toggleOption(3);
$control->key(Wire::map(['key' => 'ArrowUp']));
$control->key(Wire::map(['key' => 'Enter']));
multiSelectCheck(!$control->selected(3) && $control->selected(1) && $control->checkedState(3) === Pam\Desktop\AccessibilityCheckedState::Unchecked->value, 'Keyboard toggle removes a selected option and reports its unchecked state.');
$control->query = 'ab';
$control->search();
multiSelectCheck($requests[1]['cancelled'] && $requests[2]['query'] === 'ab', 'Replacement query cancels the prior request.');
($requests[2]['failed'])(new RuntimeException('Private transport detail'));
multiSelectCheck($control->error !== '' && !str_contains($control->error, 'Private'), 'Provider failures expose a safe retry message.');
$control->replaceSelection([]);
multiSelectCheck($control->selectedOptions === [] && $changes[array_key_last($changes)] === [], 'Consumer can commit and clear temporary selections.');
$control->open = true;
$control->query = '';
$control->search();
$mounted->unmount();
multiSelectCheck($requests[3]['cancelled'] && !$control->loading && !$control->open, 'Unmount cancels the active request and closes the popup.');
($requests[3]['complete'])([new AutocompleteOption(4, 'Late result')]);
multiSelectCheck($control->options === [], 'Late results cannot update an unmounted component.');
echo "MultiSelect bounded pagination, persistent selections, cancellation, keyboard interaction, safe errors and lifecycle passed.\n";
