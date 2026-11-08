<?php

declare(strict_types=1);

$loader = require dirname(__DIR__, 3).'/vendor/autoload.php';
$loader->addPsr4('Pam\\DesktopUi\\', dirname(__DIR__).'/src');

use Pam\Desktop\Forms\Attributes\{MinLength, Required};
use Pam\Desktop\I18n\{FrameworkText, I18n};
use Pam\Desktop\{Menu, MenuRole};
use Pam\Desktop\Navigation\ScreenOptions;
use Pam\Desktop\System\Files;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\Modules\{NativeModules, NativeModuleTransport};
use Pam\DesktopUi\{Autocomplete, Checkbox, DatePicker, DatePickerLocale, MultiSelect, Select, SelectOption};

function i18nCheck(bool $ok, string $message): void { if (!$ok) throw new RuntimeException($message); }

\Pam\Desktop\App::components(dirname(__DIR__).'/src', sys_get_temp_dir().'/pam-i18n-tests');
$saved = [getenv('LC_ALL'), getenv('LC_MESSAGES'), getenv('LANG')];
$noop = static function (mixed ...$_): void {};
$form = new class extends \Pam\Desktop\Forms\NativeForm {};
$select = static fn (): Select => new Select([new SelectOption(1, 'Um')], null, $noop);

try {
    // The system locale picks the language; unknown languages get English.
    putenv('LC_ALL='); putenv('LC_MESSAGES='); putenv('LANG=es_ES.UTF-8');
    i18nCheck(FrameworkText::language() === 'es' && $select()->placeholder === 'Seleccionar', 'es follows the system locale.');
    putenv('LANG=fr_FR.UTF-8');
    i18nCheck(FrameworkText::language() === 'en' && $select()->emptyMessage === 'No options found.', 'Unknown languages fall back to English.');
    putenv('LANG=pt_BR.UTF-8'); putenv('LC_MESSAGES=en_US.UTF-8');
    i18nCheck(FrameworkText::locale() === 'en-US', 'LC_MESSAGES wins over LANG.');
    putenv('LC_MESSAGES='); putenv('LC_ALL=C');
    i18nCheck(FrameworkText::locale() === 'pt-BR', 'The C locale is skipped.');

    // App override: I18n::use() beats the system.
    putenv('LC_ALL='); putenv('LANG=en_US.UTF-8');
    I18n::use('pt-BR');
    $pt = $select();
    i18nCheck($pt->placeholder === 'Selecionar' && $pt->filterPlaceholder === 'Buscar' && $pt->emptyMessage === 'Nenhuma opção encontrada.', 'I18n::use() drives Select defaults.');
    $auto = new Autocomplete('', $noop, $noop, $noop);
    i18nCheck($auto->label === 'Buscar' && $auto->filterLabel === 'Filtrar Buscar' && $auto->triggerAccessibilityLabel() === 'Abrir buscar opções', 'Autocomplete keeps the Portuguese copy.');
    $multi = new MultiSelect([], $noop, $noop);
    i18nCheck($multi->placeholder === 'Selecione os itens' && $multi->label === 'Selecionar vários itens', 'MultiSelect defaults are localized.');
    i18nCheck((new Checkbox(false, $noop))->label === 'Caixa de seleção', 'Checkbox label is localized.');
    $calendar = new DatePicker(null, $noop);
    i18nCheck($calendar->placeholder === 'Selecionar data' && $calendar->locale->months[2] === 'Março' && $calendar->locale->dateFormat === 'd/m/Y', 'DatePicker follows the app language.');
    i18nCheck(MenuRole::Copy->label() === 'Copiar' && Menu::edit()->label === 'Editar', 'Menu roles are localized.');
    i18nCheck((new Required())->validate('name', '', $form) === 'Este campo é obrigatório.', 'Validation messages are localized.');
    i18nCheck((new MinLength(3))->validate('n', 'ab', $form) === 'Use pelo menos 3 caracteres.', 'Parameters are filled.');
    i18nCheck((new ScreenOptions())->searchPlaceholder() === 'Buscar', 'The header search placeholder is localized.');
    i18nCheck(\Pam\Desktop\AsyncValue::offline()->message === 'Você está offline.', 'AsyncValue messages are localized.');

    // Component props and explicit labels win.
    i18nCheck((new Select([], null, $noop, placeholder: 'Escolha'))->placeholder === 'Escolha' && Menu::edit('Edição')->label === 'Edição' && (new Required('Obrigatório'))->message === 'Obrigatório', 'Explicit texts win over the catalog.');

    // App catalogs reword framework texts under the `pam` namespace.
    I18n::catalog('pt', ['pam' => ['select.placeholder' => 'Escolher']]);
    i18nCheck($select()->placeholder === 'Escolher' && $select()->label === 'Selecionar', 'An app catalog overrides one key.');
    I18n::use('es-MX');
    i18nCheck(DatePickerLocale::system()->months[0] === 'Enero' && DatePickerLocale::spanish()->apply === 'Aplicar' && MenuRole::Paste->label() === 'Pegar', 'Spanish is included.');
    I18n::use(null);
    I18n::reset();

    // Native dialogs get the localized default title, or the app's one.
    I18n::use('es');
    $calls = [];
    NativeModules::useTransport(new class($calls) implements NativeModuleTransport {
        public function __construct(private array &$calls) {}
        public function invoke(int $requestId, string $module, string $method, string $payload, Closure $complete): void { $this->calls[] = Wire::decodeMap($payload); $complete(\Pam\Desktop\ModuleResultStatus::Success, Wire::map(['saved' => false])); }
    });
    Files::save('a', 'a.csv', $noop);
    Files::save('a', 'b.csv', $noop, title: 'Exportar informe');
    NativeModules::useTransport(null);
    i18nCheck(($calls[0]['title'] ?? null) === 'Guardar archivo' && ($calls[1]['title'] ?? null) === 'Exportar informe', 'Files::save sends the localized or the given title.');
} finally {
    I18n::reset();
    foreach (['LC_ALL', 'LC_MESSAGES', 'LANG'] as $i => $name) putenv($saved[$i] === false ? $name : $name.'='.$saved[$i]);
}
echo "Framework texts follow the system locale, I18n::use(), app catalogs and props (en, pt, es).\n";
